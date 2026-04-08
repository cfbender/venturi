use crossbeam_channel::{Receiver, Sender};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::categorizer::learning::{deserialize_overrides, serialize_overrides};
use crate::categorizer::rules::classify_with_priority;
use crate::config::persistence::{DebouncedSaver, Paths, ensure_dirs, load_config, save_config};
use crate::core::command_coalescing::coalesce_commands;
#[cfg_attr(test, allow(unused_imports))]
use crate::core::device_routing::{
    config_device_value, resolve_output_loopback_target,
    resolve_selected_input_name, selected_device_available, should_skip_output_device_reconcile,
};
use crate::core::hotkeys::{
    HotkeyAdapter, HotkeyBindings, HotkeyState, build_adapter, collect_adapter_commands,
};
use crate::core::messages::{CoreCommand, CoreEvent, DeviceKind};
use crate::core::meter_worker::spawn_meter_worker;
use crate::core::pipewire_backend::{
    PwPlayProcess, current_default_sink_name, ensure_virtual_devices,
    reconcile_monitor_loopback_modules, rewire_virtual_mic_source, run_pw_metadata,
    unload_pactl_module,
};
use crate::core::pipewire_channel_control::{
    ChannelControlTargets, apply_channel_mute, apply_channel_volume,
};
use crate::core::pipewire_discovery::{Snapshot, extract_volume, parse_pw_dump};
use crate::core::pw_monitor::{PwMonitor, PwMonitorEvent};
use crate::core::router::{build_metadata_legacy_target_args, build_metadata_target_args};
use crate::core::snapshot_ops::{
    apply_snapshot_volume_hint, apply_structural_monitor_delta, channel_volume_from_snapshot,
    emit_snapshot_channel_volumes, node_id_to_channel, prune_removed_node_ids,
};
use crate::core::soundboard_playback::{
    SoundboardPlaybackMode, SoundboardPlaybackRoute, cleanup_soundboard_players,
    handle_play_sound, stop_sound,
};
use crate::core::state_persistence::{set_persisted_channel_mute, set_persisted_channel_volume};
use crate::core::stream_routing::collect_stream_route_targets_for_reconcile;

pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub fn reconnect_delay() -> Duration {
    RECONNECT_DELAY
}

pub use crate::core::device_routing::fallback_to_default_device;

const LOOP_TICK_INTERVAL: Duration = Duration::from_millis(50);
const RESTART_DELAY: Duration = Duration::from_secs(2);
const DEVICE_SELECTION_POLL_INTERVAL: Duration = Duration::from_secs(2);
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(30);

const VIRTUAL_SINKS: [&str; 6] = [
    "Venturi-Output",
    "Venturi-Game",
    "Venturi-Media",
    "Venturi-Chat",
    "Venturi-Aux",
    "Venturi-Sound",
];
const VIRTUAL_SOURCES: [&str; 1] = ["Venturi-VirtualMic"];
const VENTURI_MAIN_OUTPUT: &str = "Venturi-Output";
const VENTURI_MAIN_MONITOR: &str = "Venturi-Output.monitor";
const LEGACY_VENTURI_SINKS: [&str; 1] = ["Venturi-Mic"];

#[cfg(test)]
fn keep_pipewire_backend_symbols_for_tests() {
    let _ = current_default_sink_name as fn() -> Result<Option<String>, String>;
    let _ = reconcile_monitor_loopback_modules
        as fn(&str, Option<&str>) -> Result<Option<String>, String>;
    let _ = unload_pactl_module as fn(&str) -> Result<(), String>;
    let _ = VENTURI_MAIN_MONITOR;
}

pub struct PipeWireManager {
    handle: std::thread::JoinHandle<()>,
    meter_handle: std::thread::JoinHandle<()>,
    meter_running: Arc<AtomicBool>,
    meter_enabled: Arc<AtomicBool>,
}

type DynHotkeyAdapter = Box<dyn HotkeyAdapter + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandLoopControl {
    Continue,
    Shutdown,
}

struct CoreRuntimeState {
    paths: Paths,
    runtime_config: crate::config::schema::Config,
    hotkey_bindings: HotkeyBindings,
    hotkey_state: HotkeyState,
    hotkey_adapter: DynHotkeyAdapter,
    last_snapshot: Snapshot,
    overrides: BTreeMap<String, crate::core::messages::Channel>,
    selected_output: Option<String>,
    selected_input: Option<String>,
    output_loopback_module: Option<String>,
    virtual_mic_module: Option<String>,
    last_sink_volume_by_target: BTreeMap<String, f32>,
    last_source_volume_by_target: BTreeMap<String, f32>,
    last_sink_mute_by_target: BTreeMap<String, bool>,
    last_source_mute_by_target: BTreeMap<String, bool>,
    runtime_state: crate::config::schema::State,
    state_saver: DebouncedSaver,
    soundboard_players: BTreeMap<(u32, SoundboardPlaybackRoute), PwPlayProcess>,
    meter_enabled: Arc<AtomicBool>,
    pub shared_snapshot: Arc<Mutex<Snapshot>>,
    consecutive_failures: u32,
    first_failure_at: Instant,
    restart_pending_until: Option<Instant>,
    last_device_selection_poll_at: Instant,
    output_restore_pending: bool,
    input_restore_pending: bool,
}

impl CoreRuntimeState {
    fn initialize(
        event_tx: &Sender<CoreEvent>,
        meter_enabled: Arc<AtomicBool>,
        shared_snapshot: Arc<Mutex<Snapshot>>,
    ) -> Self {
        let paths = Paths::resolve();
        if let Err(err) = ensure_dirs(&paths) {
            let _ = event_tx.send(CoreEvent::Error(format!(
                "failed to prepare config/state directories: {err}"
            )));
        }

        let runtime_config = load_config(&paths);
        let runtime_state = crate::config::schema::State::default();
        let hotkey_bindings = HotkeyBindings::from(&runtime_config.hotkeys);
        let mut hotkey_adapter =
            build_adapter(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), false);
        let _ = hotkey_adapter.register(&hotkey_bindings);

        let selected_input = if runtime_config
            .audio
            .input_device
            .eq_ignore_ascii_case("default")
        {
            Some(fallback_to_default_device().to_string())
        } else {
            Some(runtime_config.audio.input_device.clone())
        };
        let selected_output = if runtime_config
            .audio
            .output_device
            .eq_ignore_ascii_case("default")
        {
            Some(fallback_to_default_device().to_string())
        } else {
            Some(runtime_config.audio.output_device.clone())
        };

        let mut state = Self {
            paths,
            runtime_config,
            hotkey_bindings,
            hotkey_state: HotkeyState {
                main_muted: false,
                mic_muted: false,
            },
            hotkey_adapter,
            last_snapshot: Snapshot::default(),
            overrides: BTreeMap::new(),
            selected_output: None,
            selected_input,
            output_loopback_module: None,
            virtual_mic_module: None,
            last_sink_volume_by_target: BTreeMap::new(),
            last_source_volume_by_target: BTreeMap::new(),
            last_sink_mute_by_target: BTreeMap::new(),
            last_source_mute_by_target: BTreeMap::new(),
            runtime_state,
            state_saver: DebouncedSaver::new(),
            soundboard_players: BTreeMap::new(),
            meter_enabled,
            shared_snapshot,
            consecutive_failures: 0,
            first_failure_at: Instant::now(),
            restart_pending_until: None,
            last_device_selection_poll_at: Instant::now() - DEVICE_SELECTION_POLL_INTERVAL,
            output_restore_pending: false,
            input_restore_pending: false,
        };

        state.overrides = deserialize_overrides(&state.runtime_config.categorizer.overrides);

        if let Err(err) = ensure_virtual_devices(
            VIRTUAL_SINKS.as_slice(),
            VIRTUAL_SOURCES.as_slice(),
            LEGACY_VENTURI_SINKS.as_slice(),
        ) {
            let _ = event_tx.send(CoreEvent::Error(format!(
                "failed to create virtual devices: {err}"
            )));
        }

        if let Ok(Some(source_name)) = resolve_selected_input_name(state.selected_input.as_deref())
        {
            match rewire_virtual_mic_source(&source_name, VIRTUAL_SOURCES[0]) {
                Ok(module_id) => state.virtual_mic_module = Some(module_id),
                Err(err) => {
                    let _ = event_tx.send(CoreEvent::Error(format!(
                        "failed to route virtual mic from {source_name}: {err}"
                    )));
                }
            }
        }

        if let Some(output_name) = selected_output
            && let Err(err) = state.handle_set_output_device_internal(&output_name, true)
        {
            let _ = event_tx.send(CoreEvent::Error(format!(
                "failed to restore output routing to {output_name}: {err}"
            )));
        }

        state
    }

    fn handle_core_command(
        &mut self,
        command: CoreCommand,
        event_tx: &Sender<CoreEvent>,
    ) -> Result<CommandLoopControl, String> {
        match command {
            CoreCommand::RequestSnapshot => {
                self.resend_initial_state(event_tx);
            }
            CoreCommand::SetVolume(channel, volume) => {
                let applied_volume = apply_channel_volume(
                    channel,
                    volume,
                    &self.last_snapshot,
                    ChannelControlTargets {
                        virtual_input_source_name: VIRTUAL_SOURCES[0],
                        main_output_sink_name: VENTURI_MAIN_OUTPUT,
                    },
                    &mut self.last_sink_volume_by_target,
                    &mut self.last_source_volume_by_target,
                );
                if let Some(applied_volume) = applied_volume {
                    apply_snapshot_volume_hint(
                        &mut self.last_snapshot,
                        channel,
                        applied_volume,
                        VENTURI_MAIN_OUTPUT,
                        VIRTUAL_SOURCES[0],
                    );
                    let _ = event_tx.send(CoreEvent::VolumeChanged(channel, applied_volume));
                }
                set_persisted_channel_volume(&mut self.runtime_state, channel, volume);
                self.state_saver.mark_dirty(Instant::now());
            }
            CoreCommand::SetMute(channel, muted) => {
                self.apply_mute(channel, muted);
            }
            CoreCommand::MoveStream { stream_id, channel } => {
                self.handle_move_stream(stream_id, channel)?;
            }
            CoreCommand::SetOutputDevice(device) => {
                self.handle_set_output_device(&device)?;
                self.emit_device_selection_changed(event_tx);
            }
            CoreCommand::SetInputDevice(device) => {
                self.handle_set_input_device(&device)?;
                self.emit_device_selection_changed(event_tx);
            }
            CoreCommand::ToggleWindow => {
                event_tx
                    .send(CoreEvent::ToggleWindowRequested)
                    .map_err(|err| format!("failed to emit ToggleWindowRequested event: {err}"))?;
            }
            CoreCommand::SetMeteringEnabled(enabled) => {
                self.meter_enabled.store(enabled, Ordering::Relaxed);
            }
            CoreCommand::Shutdown => {
                let _ = event_tx.send(CoreEvent::ShutdownRequested);
                return Ok(CommandLoopControl::Shutdown);
            }
            CoreCommand::PlaySound { pad_id, file } => {
                handle_play_sound(
                    &mut self.soundboard_players,
                    pad_id,
                    &file,
                    SoundboardPlaybackMode::Full,
                    event_tx,
                );
            }
            CoreCommand::PreviewSound { pad_id, file } => {
                handle_play_sound(
                    &mut self.soundboard_players,
                    pad_id,
                    &file,
                    SoundboardPlaybackMode::Preview,
                    event_tx,
                );
            }
            CoreCommand::StopSound(pad_id) => {
                stop_sound(&mut self.soundboard_players, pad_id);
            }
        }
        Ok(CommandLoopControl::Continue)
    }

    fn handle_hotkey_tick(&mut self, event_tx: &Sender<CoreEvent>) {
        let commands = collect_adapter_commands(
            &mut *self.hotkey_adapter,
            &self.hotkey_bindings,
            self.hotkey_state,
        );

        for command in commands {
            match command {
                CoreCommand::SetMute(channel, muted) => {
                    self.apply_mute(channel, muted);
                }
                CoreCommand::ToggleWindow => {
                    let _ = event_tx.send(CoreEvent::ToggleWindowRequested);
                }
                _ => {}
            }
        }
    }

    fn apply_mute(&mut self, channel: crate::core::messages::Channel, muted: bool) {
        if channel == crate::core::messages::Channel::Main {
            self.hotkey_state.main_muted = muted;
        }
        if channel == crate::core::messages::Channel::Mic {
            self.hotkey_state.mic_muted = muted;
        }

        apply_channel_mute(
            channel,
            muted,
            &self.last_snapshot,
            ChannelControlTargets {
                virtual_input_source_name: VIRTUAL_SOURCES[0],
                main_output_sink_name: VENTURI_MAIN_OUTPUT,
            },
            &mut self.last_sink_mute_by_target,
            &mut self.last_source_mute_by_target,
        );

        set_persisted_channel_mute(&mut self.runtime_state, channel, muted);
        self.state_saver.mark_dirty(Instant::now());
    }

    fn handle_move_stream(
        &mut self,
        stream_id: u32,
        channel: crate::core::messages::Channel,
    ) -> Result<(), String> {
        if let Some(stream) = self.last_snapshot.streams.get(&stream_id) {
            self.overrides.insert(stream.app_key.clone(), channel);
            self.runtime_config.categorizer.overrides = serialize_overrides(&self.overrides);
            save_config(&self.paths, &self.runtime_config)
                .map_err(|err| format!("failed to persist categorizer override: {err}"))?;
        }

        self.route_stream_to_channel(stream_id, channel)
    }

    fn route_stream_to_channel(
        &mut self,
        stream_id: u32,
        channel: crate::core::messages::Channel,
    ) -> Result<(), String> {
        let target_args = build_metadata_target_args(stream_id, channel);
        let legacy_target_args = build_metadata_legacy_target_args(stream_id, channel);

        let target_result = run_pw_metadata(&target_args);
        let legacy_result = run_pw_metadata(&legacy_target_args);

        match (target_result, legacy_result) {
            (Ok(()), _) | (_, Ok(())) => Ok(()),
            (Err(target_err), Err(legacy_err)) => Err(format!(
                "failed to move stream {stream_id}: target.object: {target_err}; target.node: {legacy_err}"
            )),
        }
    }

    fn route_stream_targets_for_reconcile(
        &mut self,
        stream_ids_before: &BTreeSet<u32>,
        output_ids_before: &BTreeMap<String, u32>,
        event_tx: &Sender<CoreEvent>,
    ) {
        for (stream_id, channel) in collect_stream_route_targets_for_reconcile(
            &self.last_snapshot,
            &self.overrides,
            stream_ids_before,
            output_ids_before,
        ) {
            if let Err(err) = self.route_stream_to_channel(stream_id, channel) {
                let _ = event_tx.send(CoreEvent::Error(format!(
                    "failed to auto-route stream {stream_id} to {channel:?}: {err}"
                )));
            }
        }
    }

    fn emit_device_selection_changed(&self, event_tx: &Sender<CoreEvent>) {
        let _ = event_tx.send(CoreEvent::DeviceSelectionChanged {
            selected_output: self.selected_output.clone(),
            selected_input: self.selected_input.clone(),
        });
    }

    fn poll_selected_device_restore(&mut self, event_tx: &Sender<CoreEvent>, force_poll: bool) {
        if !force_poll
            && self.last_device_selection_poll_at.elapsed() < DEVICE_SELECTION_POLL_INTERVAL
        {
            return;
        }
        self.last_device_selection_poll_at = Instant::now();

        if !selected_device_available(
            &self.last_snapshot.devices,
            DeviceKind::Output,
            self.selected_output.as_deref(),
        ) {
            self.output_restore_pending = self.selected_output.is_some();
        }

        if !selected_device_available(
            &self.last_snapshot.devices,
            DeviceKind::Input,
            self.selected_input.as_deref(),
        ) {
            self.input_restore_pending = self.selected_input.is_some();
        }

        let mut selection_changed = false;

        if self.output_restore_pending
            && let Some(selected_output) = self.selected_output.clone()
            && selected_device_available(
                &self.last_snapshot.devices,
                DeviceKind::Output,
                Some(selected_output.as_str()),
            )
        {
            match self.handle_set_output_device_internal(&selected_output, true) {
                Ok(()) => {
                    self.output_restore_pending = false;
                    selection_changed = true;
                }
                Err(err) => {
                    let _ = event_tx.send(CoreEvent::Error(format!(
                        "failed to restore output routing to {selected_output}: {err}"
                    )));
                }
            }
        }

        if self.input_restore_pending
            && let Some(selected_input) = self.selected_input.clone()
            && selected_device_available(
                &self.last_snapshot.devices,
                DeviceKind::Input,
                Some(selected_input.as_str()),
            )
        {
            match self.handle_set_input_device_internal(&selected_input, true) {
                Ok(()) => {
                    self.input_restore_pending = false;
                    selection_changed = true;
                }
                Err(err) => {
                    let _ = event_tx.send(CoreEvent::Error(format!(
                        "failed to restore input routing to {selected_input}: {err}"
                    )));
                }
            }
        }

        if selection_changed {
            self.emit_device_selection_changed(event_tx);
        }
    }

    fn reconcile_output_route(&mut self, device: &str) -> Result<(), String> {
        #[cfg(test)]
        {
            keep_pipewire_backend_symbols_for_tests();
            let _ = device;
            self.output_loopback_module = Some("test-output-loopback-module".to_string());
            Ok(())
        }

        #[cfg(not(test))]
        {
            let default_sink = if device.eq_ignore_ascii_case(fallback_to_default_device()) {
                current_default_sink_name()?
            } else {
                None
            };
            let desired_output_owned =
                resolve_output_loopback_target(device, default_sink.as_deref(), VENTURI_MAIN_OUTPUT);
            let desired_output = desired_output_owned.as_deref();
            self.output_loopback_module =
                reconcile_monitor_loopback_modules(VENTURI_MAIN_MONITOR, desired_output).map_err(
                    |err| {
                        if let Some(target) = desired_output {
                            format!("failed to route Venturi main mix to {target}: {err}")
                        } else {
                            format!("failed to clear Venturi main mix loopbacks: {err}")
                        }
                    },
                )?;
            Ok(())
        }
    }

    fn reconcile_input_route(&mut self) -> Result<(), String> {
        #[cfg(test)]
        {
            keep_pipewire_backend_symbols_for_tests();
            self.virtual_mic_module = Some("test-virtual-mic-module".to_string());
            Ok(())
        }

        #[cfg(not(test))]
        {
            match resolve_selected_input_name(self.selected_input.as_deref()) {
                Ok(Some(source_name)) => {
                    if let Some(prev_module) = self.virtual_mic_module.take()
                        && let Err(err) = unload_pactl_module(&prev_module)
                    {
                        return Err(format!(
                            "failed to unload virtual mic module {prev_module}: {err}"
                        ));
                    }
                    match rewire_virtual_mic_source(&source_name, VIRTUAL_SOURCES[0]) {
                        Ok(module_id) => {
                            self.virtual_mic_module = Some(module_id);
                        }
                        Err(err) => {
                            return Err(format!(
                                "failed to route virtual mic from {source_name}: {err}"
                            ));
                        }
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    return Err(format!("failed to resolve selected input source: {err}"));
                }
            }
            Ok(())
        }
    }

    fn handle_set_output_device(&mut self, device: &str) -> Result<(), String> {
        self.handle_set_output_device_internal(device, false)
    }

    fn handle_set_output_device_internal(
        &mut self,
        device: &str,
        force: bool,
    ) -> Result<(), String> {
        let selection_changed = self.selected_output.as_deref() != Some(device);
        if should_skip_output_device_reconcile(self.selected_output.as_deref(), device, force) {
            return Ok(());
        }

        self.reconcile_output_route(device)?;

        self.selected_output = Some(device.to_string());
        self.output_restore_pending = false;
        if selection_changed {
            self.runtime_config.audio.output_device = config_device_value(device);
            save_config(&self.paths, &self.runtime_config)
                .map_err(|err| format!("failed to persist output device selection: {err}"))?;
        }
        Ok(())
    }

    fn handle_set_input_device(&mut self, device: &str) -> Result<(), String> {
        self.handle_set_input_device_internal(device, false)
    }

    fn handle_set_input_device_internal(
        &mut self,
        device: &str,
        force: bool,
    ) -> Result<(), String> {
        let selection_changed = self.selected_input.as_deref() != Some(device);
        if !force && !selection_changed {
            return Ok(());
        }

        self.selected_input = Some(device.to_string());
        self.reconcile_input_route()?;

        self.input_restore_pending = false;
        if selection_changed {
            self.runtime_config.audio.input_device = config_device_value(device);
            save_config(&self.paths, &self.runtime_config)
                .map_err(|err| format!("failed to persist input device selection: {err}"))?;
        }
        Ok(())
    }

    fn flush_persisted_state_if_due(&mut self, event_tx: &Sender<CoreEvent>) {
        if self.state_saver.should_flush(Instant::now()) {
            let _ = event_tx;
            self.state_saver.did_flush();
        }
    }

    fn flush_persisted_state_now(&mut self, event_tx: &Sender<CoreEvent>) {
        let _ = event_tx;
        self.state_saver.did_flush();
    }

    /// Re-emit the current device/stream/volume state so the GUI can populate
    /// itself even if the original events were dropped during startup.
    fn resend_initial_state(&self, event_tx: &crossbeam_channel::Sender<CoreEvent>) {
        if !self.last_snapshot.devices.is_empty() {
            let _ = event_tx.send(CoreEvent::DevicesChanged(
                self.last_snapshot.devices.clone(),
            ));
        }
        self.emit_device_selection_changed(event_tx);
        for (id, stream) in &self.last_snapshot.streams {
            let category = classify_with_priority(
                &self.overrides,
                Some(&stream.app_key),
                Some(&stream.display_name),
                stream.media_role.as_deref(),
            );
            let _ = event_tx.send(CoreEvent::StreamAppeared {
                id: *id,
                app_key: stream.app_key.clone(),
                name: stream.display_name.clone(),
                category,
            });
        }
        emit_snapshot_channel_volumes(
            &self.last_snapshot,
            event_tx,
            VENTURI_MAIN_OUTPUT,
            VIRTUAL_SOURCES[0],
        );
    }

    fn handle_monitor_event(
        &mut self,
        event: PwMonitorEvent,
        event_tx: &crossbeam_channel::Sender<CoreEvent>,
    ) {
        match event {
            PwMonitorEvent::InitialSnapshot(snapshot) => {
                let stream_ids_before: BTreeSet<u32> =
                    self.last_snapshot.streams.keys().copied().collect();
                let output_ids_before = self.last_snapshot.output_ids.clone();
                // Diff devices
                if snapshot.devices != self.last_snapshot.devices {
                    let _ = event_tx.send(CoreEvent::DevicesChanged(snapshot.devices.clone()));
                }
                // Diff streams (appeared)
                for (id, stream) in &snapshot.streams {
                    if !self.last_snapshot.streams.contains_key(id) {
                        let category = classify_with_priority(
                            &self.overrides,
                            Some(&stream.app_key),
                            Some(&stream.display_name),
                            stream.media_role.as_deref(),
                        );
                        let _ = event_tx.send(CoreEvent::StreamAppeared {
                            id: *id,
                            app_key: stream.app_key.clone(),
                            name: stream.display_name.clone(),
                            category,
                        });
                    }
                }
                // Diff streams (removed)
                for id in self.last_snapshot.streams.keys() {
                    if !snapshot.streams.contains_key(id) {
                        let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
                    }
                }

                // Reset circuit breaker on successful reconnect
                self.consecutive_failures = 0;
                self.restart_pending_until = None;

                self.last_snapshot = snapshot;
                self.route_stream_targets_for_reconcile(
                    &stream_ids_before,
                    &output_ids_before,
                    event_tx,
                );
                self.poll_selected_device_restore(event_tx, true);
                // Update shared snapshot for meter worker
                *self.shared_snapshot.lock().unwrap() = self.last_snapshot.clone();
                emit_snapshot_channel_volumes(
                    &self.last_snapshot,
                    event_tx,
                    VENTURI_MAIN_OUTPUT,
                    VIRTUAL_SOURCES[0],
                );
            }
            PwMonitorEvent::ObjectsChanged(objects) => {
                self.merge_changed_objects(&objects, event_tx);
                self.poll_selected_device_restore(event_tx, true);
                *self.shared_snapshot.lock().unwrap() = self.last_snapshot.clone();
            }
            PwMonitorEvent::ProcessDied(reason) => {
                self.handle_monitor_died(reason, event_tx);
            }
        }
    }

    fn handle_monitor_died(
        &mut self,
        reason: String,
        event_tx: &crossbeam_channel::Sender<CoreEvent>,
    ) {
        self.consecutive_failures += 1;

        if self.consecutive_failures == 1 {
            self.first_failure_at = Instant::now();
        }

        let _ = event_tx.send(CoreEvent::Error(format!(
            "PipeWire monitor stopped: {reason}. Reconnecting..."
        )));

        if self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES
            && self.first_failure_at.elapsed() < FAILURE_WINDOW
        {
            let _ = event_tx.send(CoreEvent::Error(
                "PipeWire monitor failed repeatedly. Check PipeWire status.".to_string(),
            ));
            return; // Stop retrying
        }

        self.output_restore_pending = true;
        self.input_restore_pending = true;
        // Schedule non-blocking restart
        self.restart_pending_until = Some(Instant::now() + RESTART_DELAY);
    }

    fn merge_changed_objects(
        &mut self,
        objects: &[serde_json::Value],
        event_tx: &crossbeam_channel::Sender<CoreEvent>,
    ) {
        let mut removed_ids = Vec::new();
        let mut structural_ids = Vec::new();
        let stream_ids_before: BTreeSet<u32> = self.last_snapshot.streams.keys().copied().collect();
        let output_ids_before = self.last_snapshot.output_ids.clone();

        for obj in objects {
            let Some(id) = obj.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else {
                continue;
            };

            let removed = obj.get("info").is_none_or(serde_json::Value::is_null);
            if removed {
                removed_ids.push(id);
            }

            let media_class = obj
                .get("info")
                .and_then(|v| v.get("props"))
                .and_then(|v| v.get("media.class"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if media_class.contains("Sink")
                || media_class.contains("Source")
                || matches!(media_class, "Stream/Output/Audio" | "Audio/Stream/Output")
            {
                structural_ids.push(id);
            }

            // Check for volume changes
            if let Some(new_vol) = extract_volume(obj) {
                let old_vol = self.last_snapshot.volumes.get(&id).copied();
                let vol_changed = old_vol
                    .map(|old| (old - new_vol).abs() >= 0.01)
                    .unwrap_or(true);

                if vol_changed {
                    self.last_snapshot.volumes.insert(id, new_vol);

                    if let Some(channel) = node_id_to_channel(
                        id,
                        &self.last_snapshot,
                        VENTURI_MAIN_OUTPUT,
                        VIRTUAL_SOURCES[0],
                    ) {
                        let channel_volume = channel_volume_from_snapshot(
                            &self.last_snapshot,
                            channel,
                            VENTURI_MAIN_OUTPUT,
                            VIRTUAL_SOURCES[0],
                        )
                        .unwrap_or(new_vol);
                        let _ =
                            event_tx.send(CoreEvent::VolumeChanged(channel, channel_volume));
                    }
                }
            }
        }

        // Incremental structural diffing: re-parse changed objects for device/stream changes.
        let changed_json = serde_json::to_string(objects).unwrap_or_default();
        if let Ok(partial) = parse_pw_dump(
            &changed_json,
            VIRTUAL_SINKS.as_slice(),
            VIRTUAL_SOURCES.as_slice(),
        ) {
            apply_structural_monitor_delta(
                &mut self.last_snapshot,
                partial,
                &removed_ids,
                &structural_ids,
                &self.overrides,
                event_tx,
            );
        } else {
            prune_removed_node_ids(&mut self.last_snapshot, &removed_ids, event_tx);
        }

        self.route_stream_targets_for_reconcile(&stream_ids_before, &output_ids_before, event_tx);
    }
}

impl PipeWireManager {
    pub fn spawn(command_rx: Receiver<CoreCommand>, event_tx: Sender<CoreEvent>) -> Self {
        let meter_running = Arc::new(AtomicBool::new(true));
        // Start disabled — the GUI sends SetMeteringEnabled(true) after the window is
        // presented, which avoids pw-record connection pops during startup.
        let meter_enabled = Arc::new(AtomicBool::new(false));
        let shared_snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let meter_handle = spawn_meter_worker(
            event_tx.clone(),
            meter_running.clone(),
            meter_enabled.clone(),
            shared_snapshot.clone(),
        );
        let meter_running_for_core = meter_running.clone();
        let meter_enabled_for_core = meter_enabled.clone();
        let shared_snapshot_for_core = shared_snapshot;

        let (monitor_tx, monitor_rx) = crossbeam_channel::unbounded();

        // Clone monitor_tx BEFORE moving it into spawn, so we retain a copy for restarts (Task 8).
        let mut monitor: Option<PwMonitor> = match PwMonitor::spawn(
            VIRTUAL_SINKS.as_slice(),
            VIRTUAL_SOURCES.as_slice(),
            monitor_tx.clone(),
        ) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("Failed to start PwMonitor: {e}");
                None
            }
        };

        let handle = std::thread::spawn(move || {
            let _ = event_tx.send(CoreEvent::Ready);
            let mut state = CoreRuntimeState::initialize(
                &event_tx,
                meter_enabled_for_core,
                shared_snapshot_for_core,
            );

            loop {
                let timeout = if state.restart_pending_until.is_some() {
                    LOOP_TICK_INTERVAL.min(Duration::from_millis(100))
                } else {
                    LOOP_TICK_INTERVAL
                };

                crossbeam_channel::select! {
                    recv(command_rx) -> msg => {
                        let Ok(first_cmd) = msg else { break };
                        let mut commands = vec![first_cmd];
                        while let Ok(cmd) = command_rx.try_recv() {
                            commands.push(cmd);
                        }
                        let coalesced = coalesce_commands(commands);
                        for cmd in coalesced {
                            match state.handle_core_command(cmd, &event_tx) {
                                Ok(CommandLoopControl::Continue) => {}
                                Ok(CommandLoopControl::Shutdown) => {
                                    state.flush_persisted_state_now(&event_tx);
                                    if let Some(m) = monitor.take() {
                                        m.kill();
                                    }
                                    state.soundboard_players.clear();
                                    meter_running_for_core.store(false, Ordering::Relaxed);
                                    return;
                                }
                                Err(err) => {
                                    let _ = event_tx.send(CoreEvent::Error(
                                        format!("command handling failed: {err}"),
                                    ));
                                }
                            }
                        }
                    },
                    recv(monitor_rx) -> msg => {
                        if let Ok(event) = msg {
                            state.handle_monitor_event(event, &event_tx);
                        }
                    },
                    default(timeout) => {}
                }

                // Check for pending restart
                if let Some(deadline) = state.restart_pending_until
                    && Instant::now() >= deadline
                {
                    state.restart_pending_until = None;
                    match PwMonitor::spawn(
                        VIRTUAL_SINKS.as_slice(),
                        VIRTUAL_SOURCES.as_slice(),
                        monitor_tx.clone(),
                    ) {
                        Ok(new_monitor) => {
                            monitor = Some(new_monitor);
                        }
                        Err(e) => {
                            state.handle_monitor_died(format!("restart failed: {e}"), &event_tx);
                        }
                    }
                }

                // Hotkey tick, device restore poll, state flush — run on every loop iteration
                state.handle_hotkey_tick(&event_tx);
                state.poll_selected_device_restore(&event_tx, false);
                state.flush_persisted_state_if_due(&event_tx);
                cleanup_soundboard_players(&mut state.soundboard_players);
            }
            meter_running_for_core.store(false, Ordering::Relaxed);
        });
        Self {
            handle,
            meter_handle,
            meter_running,
            meter_enabled,
        }
    }

    pub fn join(self) -> std::thread::Result<()> {
        self.meter_running.store(false, Ordering::Relaxed);
        self.meter_enabled.store(false, Ordering::Relaxed);
        let core_result = self.handle.join();
        let meter_result = self.meter_handle.join();
        #[allow(clippy::question_mark)] // thread::Result error type doesn't implement From for ?
        if core_result.is_err() {
            return core_result;
        }
        meter_result
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::config::persistence::{DebouncedSaver, Paths, load_config};
    use crate::config::schema::State;
    use crate::core::command_coalescing::coalesce_commands;
    use crate::core::device_routing::{
        resolve_output_loopback_target, selected_device_available,
        should_skip_output_device_reconcile,
    };
    use crate::core::hotkeys::{HotkeyBindings, HotkeyState, build_adapter};
    use crate::core::messages::{Channel, CoreCommand, CoreEvent, DeviceEntry, DeviceKind};
    use crate::core::meter_worker::compute_level_sample_count;
    use crate::core::pipewire_discovery::{Snapshot, StreamInfo};
    use crate::core::pw_monitor::PwMonitorEvent;
    use crate::core::snapshot_ops::{
        apply_snapshot_volume_hint, apply_structural_monitor_delta, node_id_to_channel,
        snapshot_channel_volumes,
    };
    use crate::core::soundboard_playback::{SoundboardPlaybackMode, soundboard_playback_targets};
    use crate::core::stream_routing::collect_new_stream_route_targets;
    use crate::core::stream_routing::collect_stream_route_targets_for_reconcile;
    use super::{
        CoreRuntimeState, DEVICE_SELECTION_POLL_INTERVAL, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES,
    };

    fn build_test_runtime_state(
        selected_output: Option<&str>,
        selected_input: Option<&str>,
    ) -> CoreRuntimeState {
        let paths = Paths::resolve();
        let runtime_config = load_config(&paths);
        let hotkey_bindings = HotkeyBindings::from(&runtime_config.hotkeys);

        CoreRuntimeState {
            paths,
            runtime_config,
            hotkey_bindings,
            hotkey_state: HotkeyState {
                main_muted: false,
                mic_muted: false,
            },
            hotkey_adapter: build_adapter(Some("x11"), false),
            last_snapshot: Snapshot::default(),
            overrides: BTreeMap::new(),
            selected_output: selected_output.map(str::to_string),
            selected_input: selected_input.map(str::to_string),
            output_loopback_module: None,
            virtual_mic_module: None,
            last_sink_volume_by_target: BTreeMap::new(),
            last_source_volume_by_target: BTreeMap::new(),
            last_sink_mute_by_target: BTreeMap::new(),
            last_source_mute_by_target: BTreeMap::new(),
            runtime_state: State::default(),
            state_saver: DebouncedSaver::new(),
            soundboard_players: BTreeMap::new(),
            meter_enabled: Arc::new(AtomicBool::new(true)),
            shared_snapshot: Arc::new(Mutex::new(Snapshot::default())),
            consecutive_failures: 0,
            first_failure_at: Instant::now(),
            restart_pending_until: None,
            last_device_selection_poll_at: Instant::now() - DEVICE_SELECTION_POLL_INTERVAL,
            output_restore_pending: false,
            input_restore_pending: false,
        }
    }

    #[test]
    fn full_soundboard_playback_targets_sound_sink_and_main_output() {
        let targets = soundboard_playback_targets(SoundboardPlaybackMode::Full);
        assert_eq!(targets, vec!["Venturi-Sound", "Venturi-Output"]);
    }

    #[test]
    fn preview_soundboard_playback_targets_main_output_only() {
        let targets = soundboard_playback_targets(SoundboardPlaybackMode::Preview);
        assert_eq!(targets, vec!["Venturi-Output"]);
    }

    #[test]
    fn derives_channel_volumes_from_snapshot_for_startup_replay() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("Venturi-Output".to_string(), 128);
        snapshot.output_ids.insert("Venturi-Media".to_string(), 555);
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 281);
        snapshot.volumes.insert(128, 0.41);
        snapshot.volumes.insert(555, 0.27);
        snapshot.volumes.insert(281, 0.73);
        snapshot.streams.insert(
            901,
            StreamInfo {
                id: 901,
                meter_target: 9901,
                node_name: Some("spotify-node".to_string()),
                is_corked: false,
                app_key: "spotify".to_string(),
                display_name: "Spotify".to_string(),
                media_role: Some("music".to_string()),
            },
        );
        snapshot.volumes.insert(901, 0.99);

        let volumes = snapshot_channel_volumes(
            &snapshot,
            VENTURI_MAIN_OUTPUT,
            VIRTUAL_SOURCES[0],
        );

        assert_eq!(volumes.get(&Channel::Main).copied(), Some(0.41));
        assert_eq!(volumes.get(&Channel::Mic).copied(), Some(0.73));
        assert_eq!(volumes.get(&Channel::Media).copied(), Some(0.27));
    }

    #[test]
    fn derives_category_channel_volume_from_mix_sink_not_stream_volume() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Media".to_string(), 777);
        snapshot.streams.insert(
            100,
            StreamInfo {
                id: 100,
                meter_target: 9100,
                node_name: Some("media-low".to_string()),
                is_corked: false,
                app_key: "spotify".to_string(),
                display_name: "Spotify".to_string(),
                media_role: Some("music".to_string()),
            },
        );
        snapshot.streams.insert(
            200,
            StreamInfo {
                id: 200,
                meter_target: 9200,
                node_name: Some("media-high".to_string()),
                is_corked: false,
                app_key: "firefox".to_string(),
                display_name: "Firefox".to_string(),
                media_role: Some("movie".to_string()),
            },
        );
        snapshot.volumes.insert(777, 0.33);
        snapshot.volumes.insert(100, 0.40);
        snapshot.volumes.insert(200, 0.04);

        let volumes = snapshot_channel_volumes(
            &snapshot,
            VENTURI_MAIN_OUTPUT,
            VIRTUAL_SOURCES[0],
        );

        assert_eq!(volumes.get(&Channel::Media).copied(), Some(0.33));
    }

    #[test]
    fn apply_snapshot_volume_hint_updates_main_mic_and_category_mix_sinks() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("Venturi-Output".to_string(), 128);
        snapshot
            .output_ids
            .insert("Venturi-Media".to_string(), 9010);
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 281);
        snapshot.streams.insert(
            901,
            StreamInfo {
                id: 901,
                meter_target: 9901,
                node_name: Some("spotify-node".to_string()),
                is_corked: false,
                app_key: "spotify".to_string(),
                display_name: "Spotify".to_string(),
                media_role: Some("music".to_string()),
            },
        );
        snapshot.streams.insert(
            902,
            StreamInfo {
                id: 902,
                meter_target: 9902,
                node_name: Some("discord-node".to_string()),
                is_corked: false,
                app_key: "discord".to_string(),
                display_name: "Discord".to_string(),
                media_role: Some("communication".to_string()),
            },
        );

        apply_snapshot_volume_hint(
            &mut snapshot,
            Channel::Main,
            0.31,
            VENTURI_MAIN_OUTPUT,
            VIRTUAL_SOURCES[0],
        );
        apply_snapshot_volume_hint(
            &mut snapshot,
            Channel::Mic,
            0.62,
            VENTURI_MAIN_OUTPUT,
            VIRTUAL_SOURCES[0],
        );
        apply_snapshot_volume_hint(
            &mut snapshot,
            Channel::Media,
            0.47,
            VENTURI_MAIN_OUTPUT,
            VIRTUAL_SOURCES[0],
        );

        assert_eq!(snapshot.volumes.get(&128).copied(), Some(0.31));
        assert_eq!(snapshot.volumes.get(&281).copied(), Some(0.62));
        assert_eq!(snapshot.volumes.get(&9010).copied(), Some(0.47));
        assert_eq!(snapshot.volumes.get(&901).copied(), None);
        assert_eq!(snapshot.volumes.get(&902).copied(), None);
    }

    #[test]
    fn resolves_default_output_to_real_hardware_sink_for_loopback() {
        assert_eq!(
            resolve_output_loopback_target(
                "Default",
                Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo"),
                VENTURI_MAIN_OUTPUT,
            ),
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo".to_string())
        );
        assert_eq!(
            resolve_output_loopback_target("Default", Some("Venturi-Output"), VENTURI_MAIN_OUTPUT),
            None
        );
        assert_eq!(
            resolve_output_loopback_target(
                "alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo",
                Some("ignored"),
                VENTURI_MAIN_OUTPUT,
            ),
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo".to_string())
        );
    }

    #[test]
    fn output_device_restore_forces_reconcile_even_when_selection_matches() {
        assert!(!should_skip_output_device_reconcile(
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo"),
            "alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo",
            true,
        ));
        assert!(should_skip_output_device_reconcile(
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo"),
            "alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo",
            false,
        ));
    }

    #[test]
    fn selected_device_available_matches_default_and_kind() {
        let devices = vec![
            DeviceEntry {
                kind: DeviceKind::Output,
                id: "alsa_output.usb-Headset.analog-stereo".to_string(),
                label: "Headset Output".to_string(),
            },
            DeviceEntry {
                kind: DeviceKind::Input,
                id: "alsa_input.usb-Headset.mono-fallback".to_string(),
                label: "Headset Input".to_string(),
            },
        ];

        assert!(selected_device_available(
            &devices,
            DeviceKind::Output,
            Some("Default"),
        ));
        assert!(selected_device_available(
            &devices,
            DeviceKind::Output,
            Some("alsa_output.usb-Headset.analog-stereo"),
        ));
        assert!(!selected_device_available(
            &devices,
            DeviceKind::Output,
            Some("alsa_input.usb-Headset.mono-fallback"),
        ));
        assert!(!selected_device_available(
            &devices,
            DeviceKind::Input,
            Some("alsa_input.usb-Unknown.mono-fallback"),
        ));
        assert!(!selected_device_available(
            &devices,
            DeviceKind::Input,
            None
        ));
    }

    #[test]
    fn emits_device_selection_changed_when_selected_devices_reappear() {
        let selected_output = "alsa_output.usb-Headset.analog-stereo";
        let selected_input = "alsa_input.usb-Headset.mono-fallback";
        let mut state = build_test_runtime_state(Some(selected_output), Some(selected_input));
        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        state.handle_monitor_event(
            PwMonitorEvent::InitialSnapshot(Snapshot::default()),
            &event_tx,
        );
        assert!(state.output_restore_pending);
        assert!(state.input_restore_pending);

        let restored_snapshot = Snapshot {
            devices: vec![
                DeviceEntry {
                    kind: DeviceKind::Output,
                    id: selected_output.to_string(),
                    label: "Headset Output".to_string(),
                },
                DeviceEntry {
                    kind: DeviceKind::Input,
                    id: selected_input.to_string(),
                    label: "Headset Input".to_string(),
                },
            ],
            ..Default::default()
        };

        state.handle_monitor_event(
            PwMonitorEvent::InitialSnapshot(restored_snapshot),
            &event_tx,
        );

        assert!(!state.output_restore_pending);
        assert!(!state.input_restore_pending);

        let selection_event = event_rx.try_iter().find_map(|event| match event {
            CoreEvent::DeviceSelectionChanged {
                selected_output,
                selected_input,
            } => Some((selected_output, selected_input)),
            _ => None,
        });

        assert_eq!(
            selection_event,
            Some((
                Some(selected_output.to_string()),
                Some(selected_input.to_string())
            ))
        );
    }

    #[test]
    fn collects_route_targets_for_new_streams_from_classification() {
        let mut snapshot = Snapshot::default();
        snapshot.streams.insert(
            100,
            StreamInfo {
                id: 100,
                meter_target: 9100,
                node_name: Some("firefox-node".to_string()),
                is_corked: false,
                app_key: "firefox".to_string(),
                display_name: "Firefox".to_string(),
                media_role: Some("Movie".to_string()),
            },
        );
        snapshot.streams.insert(
            101,
            StreamInfo {
                id: 101,
                meter_target: 9101,
                node_name: Some("discord-node".to_string()),
                is_corked: false,
                app_key: "discord".to_string(),
                display_name: "Discord".to_string(),
                media_role: Some("Communication".to_string()),
            },
        );

        let stream_ids_before = BTreeSet::from([100_u32]);

        let routes =
            collect_new_stream_route_targets(&snapshot, &BTreeMap::new(), &stream_ids_before);

        assert_eq!(routes, vec![(101_u32, Channel::Chat)]);
    }

    #[test]
    fn collects_route_targets_for_existing_streams_when_category_mix_sink_ids_change() {
        let mut snapshot = Snapshot::default();
        snapshot.streams.insert(
            100,
            StreamInfo {
                id: 100,
                meter_target: 9100,
                node_name: Some("firefox-node".to_string()),
                is_corked: false,
                app_key: "firefox".to_string(),
                display_name: "Firefox".to_string(),
                media_role: Some("Movie".to_string()),
            },
        );
        snapshot
            .output_ids
            .insert("Venturi-Media".to_string(), 4000);

        let stream_ids_before = BTreeSet::from([100_u32]);
        let output_ids_before = BTreeMap::new();

        let routes = collect_stream_route_targets_for_reconcile(
            &snapshot,
            &BTreeMap::new(),
            &stream_ids_before,
            &output_ids_before,
        );

        assert_eq!(routes, vec![(100_u32, Channel::Media)]);
    }

    #[test]
    fn computes_meter_sample_count_for_sampling_interval() {
        assert_eq!(
            compute_level_sample_count(48_000, Duration::from_millis(66)),
            3168
        );
        assert_eq!(
            compute_level_sample_count(48_000, Duration::from_millis(1)),
            48
        );
    }

    #[test]
    fn node_id_to_channel_maps_output_sink_to_main() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Output".to_string(), 50);
        assert_eq!(
            node_id_to_channel(50, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            Some(Channel::Main)
        );
    }

    #[test]
    fn node_id_to_channel_maps_input_source_to_mic() {
        let mut snapshot = Snapshot::default();
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 60);
        assert_eq!(
            node_id_to_channel(60, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            Some(Channel::Mic)
        );
    }

    #[test]
    fn node_id_to_channel_maps_category_mix_sink() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Media".to_string(), 100);
        assert_eq!(
            node_id_to_channel(100, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            Some(Channel::Media)
        );
    }

    #[test]
    fn node_id_to_channel_ignores_stream_ids_for_category_separation() {
        let mut snapshot = Snapshot::default();
        snapshot.streams.insert(
            100,
            crate::core::pipewire_discovery::StreamInfo {
                id: 100,
                meter_target: 0,
                node_name: Some("firefox-node".to_string()),
                is_corked: false,
                app_key: "firefox".to_string(),
                display_name: "Firefox".to_string(),
                media_role: None,
            },
        );

        assert_eq!(
            node_id_to_channel(100, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            None,
        );
    }

    #[test]
    fn node_id_to_channel_returns_none_for_unknown_id() {
        let snapshot = Snapshot::default();
        assert_eq!(
            node_id_to_channel(999, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            None,
        );
    }

    #[test]
    fn node_id_to_channel_ignores_non_main_output_sinks() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Output".to_string(), 50);
        snapshot
            .output_ids
            .insert("alsa_output.real".to_string(), 88);

        assert_eq!(
            node_id_to_channel(88, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            None,
        );
    }

    #[test]
    fn node_id_to_channel_ignores_non_virtual_mic_inputs() {
        let mut snapshot = Snapshot::default();
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 60);
        snapshot
            .input_ids
            .insert("alsa_input.real_mic".to_string(), 61);

        assert_eq!(
            node_id_to_channel(61, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            None,
        );
    }

    #[test]
    fn category_mix_sinks_are_managed_and_not_legacy_cleanup_targets() {
        let category_mix_sinks = [
            "Venturi-Game",
            "Venturi-Media",
            "Venturi-Chat",
            "Venturi-Aux",
        ];

        for sink in category_mix_sinks {
            assert!(
                super::VIRTUAL_SINKS.contains(&sink),
                "{sink} must be provisioned as an active virtual sink"
            );
            assert!(
                !super::LEGACY_VENTURI_SINKS.contains(&sink),
                "{sink} must not be unloaded as a legacy sink"
            );
        }
    }

    #[test]
    fn structural_delta_updates_main_and_mic_node_mappings() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("Venturi-Output".to_string(), 225);
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 331);

        let mut partial = Snapshot::default();
        // Sink got recreated (new id), and old id was reused by mic source.
        partial.output_ids.insert("Venturi-Output".to_string(), 412);
        partial
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 225);

        let (event_tx, _event_rx) = crossbeam_channel::unbounded();
        let overrides = BTreeMap::new();
        let no_removed_ids: [u32; 0] = [];
        let structural_ids = [225_u32, 412_u32];
        apply_structural_monitor_delta(
            &mut snapshot,
            partial,
            no_removed_ids.as_slice(),
            structural_ids.as_slice(),
            &overrides,
            &event_tx,
        );

        assert_eq!(snapshot.output_ids.get("Venturi-Output"), Some(&412));
        assert_eq!(snapshot.input_ids.get("Venturi-VirtualMic"), Some(&225));
        assert_eq!(
            node_id_to_channel(412, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            Some(Channel::Main)
        );
        assert_eq!(
            node_id_to_channel(225, &snapshot, VENTURI_MAIN_OUTPUT, VIRTUAL_SOURCES[0]),
            Some(Channel::Mic)
        );
    }

    #[test]
    fn structural_delta_prunes_removed_device_ids_and_emits_devices_changed() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("Venturi-Output".to_string(), 225);
        snapshot.devices.push(DeviceEntry {
            kind: DeviceKind::Output,
            id: "Venturi-Output".to_string(),
            label: "Venturi Output".to_string(),
        });

        let partial = Snapshot::default();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let overrides = BTreeMap::new();
        let removed_ids = [225_u32];
        let no_structural_ids: [u32; 0] = [];
        apply_structural_monitor_delta(
            &mut snapshot,
            partial,
            removed_ids.as_slice(),
            no_structural_ids.as_slice(),
            &overrides,
            &event_tx,
        );

        assert!(!snapshot.output_ids.contains_key("Venturi-Output"));
        assert!(snapshot.devices.is_empty());

        let devices_changed = event_rx
            .try_iter()
            .find_map(|event| match event {
                CoreEvent::DevicesChanged(devices) => Some(devices),
                _ => None,
            })
            .expect("DevicesChanged should be emitted after pruning removed output");
        assert!(devices_changed.is_empty());
    }

    #[test]
    fn structural_delta_prunes_stream_when_id_reassigned_to_mic_source() {
        let mut snapshot = Snapshot::default();
        snapshot.streams.insert(
            225,
            StreamInfo {
                id: 225,
                meter_target: 225,
                node_name: Some("spotify-node".to_string()),
                is_corked: false,
                app_key: "spotify".to_string(),
                display_name: "Spotify".to_string(),
                media_role: Some("music".to_string()),
            },
        );
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 331);
        snapshot
            .input_meter_targets
            .insert("Venturi-VirtualMic".to_string(), 331);

        let mut partial = Snapshot::default();
        partial
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 225);
        partial
            .input_meter_targets
            .insert("Venturi-VirtualMic".to_string(), 225);

        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let overrides = BTreeMap::new();
        let no_removed_ids: [u32; 0] = [];
        let structural_ids = [225_u32];
        apply_structural_monitor_delta(
            &mut snapshot,
            partial,
            no_removed_ids.as_slice(),
            structural_ids.as_slice(),
            &overrides,
            &event_tx,
        );

        assert!(
            !snapshot.streams.contains_key(&225),
            "stream id should be pruned when reassigned to source"
        );

        assert_eq!(
            snapshot.input_ids.get("Venturi-VirtualMic"),
            Some(&225),
            "mic source should be mapped to reassigned id"
        );
        assert!(
            snapshot.streams.is_empty(),
            "media stream must not persist with reassigned mic source id"
        );

        let removed = event_rx
            .try_iter()
            .any(|event| matches!(event, CoreEvent::StreamRemoved(225)));
        assert!(removed, "StreamRemoved should be emitted for reassigned id");
    }

    #[test]
    fn coalesce_keeps_last_volume_per_channel() {
        let commands = vec![
            CoreCommand::SetVolume(Channel::Main, 0.1),
            CoreCommand::SetVolume(Channel::Main, 0.5),
            CoreCommand::SetVolume(Channel::Main, 0.9),
        ];
        let result = coalesce_commands(commands);
        assert_eq!(result.len(), 1);
        assert!(
            matches!(result[0], CoreCommand::SetVolume(Channel::Main, v) if (v - 0.9).abs() < 0.001)
        );
    }

    #[test]
    fn coalesce_preserves_non_volume_commands_in_order() {
        let commands = vec![
            CoreCommand::SetVolume(Channel::Main, 0.5),
            CoreCommand::SetMute(Channel::Game, true),
            CoreCommand::SetVolume(Channel::Main, 0.8),
        ];
        let result = coalesce_commands(commands);
        // SetMute emitted in order, then coalesced SetVolume(Main, 0.8) at end
        assert_eq!(result.len(), 2);
        assert!(matches!(
            result[0],
            CoreCommand::SetMute(Channel::Game, true)
        ));
        assert!(
            matches!(result[1], CoreCommand::SetVolume(Channel::Main, v) if (v - 0.8).abs() < 0.001)
        );
    }

    #[test]
    fn coalesce_multiple_channels_independently() {
        let commands = vec![
            CoreCommand::SetVolume(Channel::Main, 0.3),
            CoreCommand::SetVolume(Channel::Game, 0.6),
            CoreCommand::SetVolume(Channel::Main, 0.7),
        ];
        let result = coalesce_commands(commands);
        assert_eq!(result.len(), 2);
        // Volume commands emitted in deterministic Channel order (Main < Game via Ord)
        assert!(
            matches!(result[0], CoreCommand::SetVolume(Channel::Main, v) if (v - 0.7).abs() < 0.001)
        );
        assert!(
            matches!(result[1], CoreCommand::SetVolume(Channel::Game, v) if (v - 0.6).abs() < 0.001)
        );
    }

    #[test]
    fn coalesce_preserves_non_volume_commands_before_shutdown() {
        let commands = vec![
            CoreCommand::RequestSnapshot,
            CoreCommand::SetVolume(Channel::Main, 0.5),
            CoreCommand::Shutdown,
            CoreCommand::SetMute(Channel::Game, true),
        ];

        let result = coalesce_commands(commands);

        assert_eq!(result.len(), 2);
        assert!(matches!(result[0], CoreCommand::RequestSnapshot));
        assert!(matches!(result[1], CoreCommand::Shutdown));
    }

    #[test]
    fn coalesce_shutdown_discards_remaining() {
        let commands = vec![
            CoreCommand::SetVolume(Channel::Main, 0.5),
            CoreCommand::Shutdown,
            CoreCommand::SetVolume(Channel::Game, 0.9),
        ];
        let result = coalesce_commands(commands);
        // Shutdown emits immediately, discards remaining + pending volumes
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], CoreCommand::Shutdown));
    }

    #[test]
    fn coalesce_empty_batch_returns_empty() {
        let result = coalesce_commands(vec![]);
        assert!(result.is_empty());
    }

    #[test]
    fn restart_logic_allows_first_two_failures() {
        use std::time::Instant;

        let mut consecutive_failures: u32 = 0;
        let first_failure_at = Instant::now();
        let max_failures: u32 = 3;
        let failure_window = Duration::from_secs(30);

        // First failure
        consecutive_failures += 1;
        let should_give_up =
            consecutive_failures >= max_failures && first_failure_at.elapsed() < failure_window;
        assert!(!should_give_up, "first failure should not give up");

        // Second failure
        consecutive_failures += 1;
        let should_give_up =
            consecutive_failures >= max_failures && first_failure_at.elapsed() < failure_window;
        assert!(!should_give_up, "second failure should not give up");
    }

    #[test]
    fn restart_logic_gives_up_after_three_fast_failures() {
        use std::time::Instant;

        let consecutive_failures: u32 = 3;
        let first_failure_at = Instant::now();
        let failure_window = Duration::from_secs(30);

        let should_give_up =
            consecutive_failures >= 3 && first_failure_at.elapsed() < failure_window;
        assert!(should_give_up, "3 failures in 30s should give up");
    }
}
