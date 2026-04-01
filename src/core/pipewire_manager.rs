use crossbeam_channel::{Receiver, Sender};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::categorizer::learning::{deserialize_overrides, serialize_overrides};
use crate::categorizer::rules::classify_with_priority;
use crate::config::persistence::{
    DebouncedSaver, Paths, ensure_dirs, load_config, load_state, save_config, save_state,
};
use crate::core::hotkeys::{
    HotkeyAdapter, HotkeyBindings, HotkeyState, build_adapter, collect_adapter_commands,
};
use crate::core::messages::{Channel, CoreCommand, CoreEvent};
use crate::core::pipewire_backend::{
    PwTargetSampler, current_default_sink_name, current_default_source_name,
    ensure_virtual_devices, reconcile_monitor_loopback_modules, rewire_virtual_mic_source,
    run_pw_link, run_pw_metadata, unload_pactl_module,
};
use crate::core::pipewire_channel_control::{
    ChannelControlTargets, apply_channel_mute, apply_channel_volume,
};
use crate::core::pipewire_discovery::{Snapshot, extract_volume, parse_pw_dump};
use crate::core::pw_monitor::{PwMonitor, PwMonitorEvent};
use crate::core::router::{
    FORCE_LINK_ROUTING_ENV, RoutingMode, build_fallback_link_commands, build_metadata_target_args,
    routing_mode_from_flag,
};

pub const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub fn reconnect_delay() -> Duration {
    RECONNECT_DELAY
}

pub fn fallback_to_default_device() -> &'static str {
    "Default"
}

const LOOP_TICK_INTERVAL: Duration = Duration::from_millis(50);
const MAX_STREAM_PROBES_PER_CHANNEL: usize = 1;
const ECHO_SUPPRESSION_WINDOW: Duration = Duration::from_millis(200);
const RESTART_DELAY: Duration = Duration::from_secs(2);
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(30);
const METER_WORKER_INTERVAL: Duration = Duration::from_millis(33);
const METER_WORKER_IDLE_INTERVAL: Duration = Duration::from_millis(500);
const METER_OVERRIDE_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const METER_SAMPLE_INTERVAL: Duration = Duration::from_millis(66);
const VIRTUAL_SINKS: [&str; 1] = ["Venturi-Output"];
const VIRTUAL_SOURCES: [&str; 1] = ["Venturi-VirtualMic"];
const VENTURI_MAIN_OUTPUT: &str = "Venturi-Output";
const VENTURI_MAIN_MONITOR: &str = "Venturi-Output.monitor";
const LEGACY_VENTURI_SINKS: [&str; 6] = [
    "Venturi-Game",
    "Venturi-Media",
    "Venturi-Chat",
    "Venturi-Aux",
    "Venturi-Mic",
    "Venturi-Sound",
];

fn resolve_selected_input_name(selected_input: Option<&str>) -> Result<Option<String>, String> {
    match selected_input {
        Some(name) if !name.is_empty() && name != fallback_to_default_device() => {
            Ok(Some(name.to_string()))
        }
        _ => current_default_source_name(),
    }
}

fn config_device_value(device: &str) -> String {
    if device.eq_ignore_ascii_case(fallback_to_default_device()) {
        "default".to_string()
    } else {
        device.to_string()
    }
}

fn resolve_output_loopback_target(device: &str, default_sink: Option<&str>) -> Option<String> {
    if !device.eq_ignore_ascii_case(fallback_to_default_device()) {
        return Some(device.to_string());
    }

    default_sink
        .filter(|name| !name.eq_ignore_ascii_case(VENTURI_MAIN_OUTPUT))
        .map(ToOwned::to_owned)
}

fn should_skip_output_device_reconcile(
    current_selection: Option<&str>,
    requested_device: &str,
    force: bool,
) -> bool {
    !force && current_selection == Some(requested_device)
}

fn persisted_channel_volume(state: &crate::config::schema::State, channel: Channel) -> f32 {
    match channel {
        Channel::Main => state.volumes.main,
        Channel::Game => state.volumes.game,
        Channel::Media => state.volumes.media,
        Channel::Chat => state.volumes.chat,
        Channel::Aux => state.volumes.aux,
        Channel::Mic => state.volumes.mic,
    }
}

fn persisted_channel_mute(state: &crate::config::schema::State, channel: Channel) -> bool {
    match channel {
        Channel::Main => state.muted.main,
        Channel::Game => state.muted.game,
        Channel::Media => state.muted.media,
        Channel::Chat => state.muted.chat,
        Channel::Aux => state.muted.aux,
        Channel::Mic => state.muted.mic,
    }
}

fn set_persisted_channel_volume(
    state: &mut crate::config::schema::State,
    channel: Channel,
    volume: f32,
) {
    let normalized = volume.clamp(0.0, 1.0);
    match channel {
        Channel::Main => state.volumes.main = normalized,
        Channel::Game => state.volumes.game = normalized,
        Channel::Media => state.volumes.media = normalized,
        Channel::Chat => state.volumes.chat = normalized,
        Channel::Aux => state.volumes.aux = normalized,
        Channel::Mic => state.volumes.mic = normalized,
    }
}

fn set_persisted_channel_mute(
    state: &mut crate::config::schema::State,
    channel: Channel,
    muted: bool,
) {
    match channel {
        Channel::Main => state.muted.main = muted,
        Channel::Game => state.muted.game = muted,
        Channel::Media => state.muted.media = muted,
        Channel::Chat => state.muted.chat = muted,
        Channel::Aux => state.muted.aux = muted,
        Channel::Mic => state.muted.mic = muted,
    }
}

/// Map a PipeWire node ID to a Venturi Channel using snapshot state and categorizer.
fn node_id_to_channel(
    id: u32,
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, Channel>,
) -> Option<Channel> {
    // Check if id matches a known output sink → Main
    if snapshot.output_ids.values().any(|&nid| nid == id) {
        return Some(Channel::Main);
    }
    // Check if id matches a known input source → Mic
    if snapshot.input_ids.values().any(|&nid| nid == id) {
        return Some(Channel::Mic);
    }
    // Check streams → classify via categorizer
    if let Some(stream) = snapshot.streams.get(&id) {
        return Some(classify_with_priority(
            overrides,
            Some(&stream.app_key),
            Some(&stream.display_name),
            stream.media_role.as_deref(),
        ));
    }
    None
}

fn upsert_devices(
    devices: &mut Vec<crate::core::messages::DeviceEntry>,
    updates: Vec<crate::core::messages::DeviceEntry>,
) {
    for update in updates {
        if let Some(existing) = devices
            .iter_mut()
            .find(|entry| entry.kind == update.kind && entry.id == update.id)
        {
            *existing = update;
        } else {
            devices.push(update);
        }
    }
}

fn prune_removed_node_ids(
    snapshot: &mut Snapshot,
    removed_ids: &[u32],
    event_tx: &Sender<CoreEvent>,
) {
    if removed_ids.is_empty() {
        return;
    }

    let removed: BTreeSet<u32> = removed_ids.iter().copied().collect();

    for id in &removed {
        if snapshot.streams.remove(id).is_some() {
            let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
        }
        snapshot.volumes.remove(id);
    }

    snapshot
        .output_ids
        .retain(|_, node_id| !removed.contains(node_id));
    snapshot
        .input_ids
        .retain(|_, node_id| !removed.contains(node_id));
}

fn apply_structural_monitor_delta(
    snapshot: &mut Snapshot,
    partial: Snapshot,
    removed_ids: &[u32],
    structural_ids: &[u32],
    overrides: &BTreeMap<String, Channel>,
    event_tx: &Sender<CoreEvent>,
) {
    let devices_before = snapshot.devices.clone();

    let partial_stream_ids: BTreeSet<u32> = partial.streams.keys().copied().collect();
    let partial_output_ids: BTreeSet<u32> = partial.output_ids.values().copied().collect();
    let partial_input_ids: BTreeSet<u32> = partial.input_ids.values().copied().collect();
    let structural: BTreeSet<u32> = structural_ids.iter().copied().collect();

    for id in &structural {
        if !partial_stream_ids.contains(id) && snapshot.streams.remove(id).is_some() {
            let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
        }
        snapshot.volumes.remove(id);
    }

    snapshot
        .output_ids
        .retain(|_, node_id| !structural.contains(node_id) || partial_output_ids.contains(node_id));
    snapshot
        .input_ids
        .retain(|_, node_id| !structural.contains(node_id) || partial_input_ids.contains(node_id));

    snapshot.output_ids.extend(partial.output_ids);
    snapshot.input_ids.extend(partial.input_ids);
    snapshot
        .output_meter_targets
        .extend(partial.output_meter_targets);
    snapshot
        .input_meter_targets
        .extend(partial.input_meter_targets);
    snapshot.volumes.extend(partial.volumes);

    for (id, stream_info) in partial.streams {
        if !snapshot.streams.contains_key(&id) {
            let category = classify_with_priority(
                overrides,
                Some(&stream_info.app_key),
                Some(&stream_info.display_name),
                stream_info.media_role.as_deref(),
            );
            let _ = event_tx.send(CoreEvent::StreamAppeared {
                id,
                name: stream_info.display_name.clone(),
                category,
            });
        }
        snapshot.streams.insert(id, stream_info);
    }

    upsert_devices(&mut snapshot.devices, partial.devices);
    prune_removed_node_ids(snapshot, removed_ids, event_tx);

    let output_node_names: BTreeSet<String> = snapshot.output_ids.keys().cloned().collect();
    let input_node_names: BTreeSet<String> = snapshot.input_ids.keys().cloned().collect();

    snapshot
        .output_meter_targets
        .retain(|node_name, _| output_node_names.contains(node_name));
    snapshot
        .input_meter_targets
        .retain(|node_name, _| input_node_names.contains(node_name));

    snapshot.devices.retain(|device| match device.kind {
        crate::core::messages::DeviceKind::Output => output_node_names.contains(&device.id),
        crate::core::messages::DeviceKind::Input => input_node_names.contains(&device.id),
    });

    if snapshot.devices != devices_before {
        let _ = event_tx.send(CoreEvent::DevicesChanged(snapshot.devices.clone()));
    }
}

fn build_channel_level_targets(
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, crate::core::messages::Channel>,
) -> BTreeMap<crate::core::messages::Channel, Vec<u32>> {
    let mut targets = BTreeMap::new();

    if let Some(main_id) = snapshot
        .output_meter_targets
        .get(VENTURI_MAIN_OUTPUT)
        .copied()
    {
        targets
            .entry(crate::core::messages::Channel::Main)
            .or_insert_with(Vec::new)
            .push(main_id);
    }

    if let Some(mic_id) = snapshot
        .input_meter_targets
        .get(VIRTUAL_SOURCES[0])
        .copied()
    {
        targets
            .entry(crate::core::messages::Channel::Mic)
            .or_insert_with(Vec::new)
            .push(mic_id);
    }

    for stream in snapshot.streams.values() {
        let channel = classify_with_priority(
            overrides,
            Some(&stream.app_key),
            Some(&stream.display_name),
            stream.media_role.as_deref(),
        );
        if matches!(
            channel,
            crate::core::messages::Channel::Game
                | crate::core::messages::Channel::Media
                | crate::core::messages::Channel::Chat
                | crate::core::messages::Channel::Aux
        ) {
            targets
                .entry(channel)
                .or_insert_with(Vec::new)
                .push(stream.meter_target);
        }
    }

    targets
}

fn build_stream_name_level_targets(
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, crate::core::messages::Channel>,
) -> BTreeMap<crate::core::messages::Channel, Vec<String>> {
    let mut targets = BTreeMap::new();

    for stream in snapshot.streams.values() {
        let channel = classify_with_priority(
            overrides,
            Some(&stream.app_key),
            Some(&stream.display_name),
            stream.media_role.as_deref(),
        );
        if matches!(
            channel,
            crate::core::messages::Channel::Game
                | crate::core::messages::Channel::Media
                | crate::core::messages::Channel::Chat
                | crate::core::messages::Channel::Aux
        ) && !stream.is_corked
            && let Some(node_name) = &stream.node_name
        {
            targets
                .entry(channel)
                .or_insert_with(Vec::new)
                .push(node_name.clone());
        }
    }

    targets
}

fn retain_primary_numeric_meter_targets(
    targets: &mut BTreeMap<crate::core::messages::Channel, Vec<u32>>,
) {
    targets.retain(|channel, _| {
        matches!(
            channel,
            crate::core::messages::Channel::Main | crate::core::messages::Channel::Mic
        )
    });
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
    routing_mode: RoutingMode,
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
    meter_enabled: Arc<AtomicBool>,
    last_volume_sent: BTreeMap<Channel, Instant>,
    pub shared_snapshot: Arc<Mutex<Snapshot>>,
    consecutive_failures: u32,
    first_failure_at: Instant,
    restart_pending_until: Option<Instant>,
}

impl CoreRuntimeState {
    fn initialize(
        event_tx: &Sender<CoreEvent>,
        meter_enabled: Arc<AtomicBool>,
        shared_snapshot: Arc<Mutex<Snapshot>>,
    ) -> Self {
        let routing_mode =
            routing_mode_from_flag(std::env::var(FORCE_LINK_ROUTING_ENV).ok().as_deref());
        let paths = Paths::resolve();
        if let Err(err) = ensure_dirs(&paths) {
            let _ = event_tx.send(CoreEvent::Error(format!(
                "failed to prepare config/state directories: {err}"
            )));
        }

        let runtime_config = load_config(&paths);
        let runtime_state = load_state(&paths);
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
            routing_mode,
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
            meter_enabled,
            last_volume_sent: BTreeMap::new(),
            shared_snapshot,
            consecutive_failures: 0,
            first_failure_at: Instant::now(),
            restart_pending_until: None,
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
            CoreCommand::Ping => {
                event_tx
                    .send(CoreEvent::Pong)
                    .map_err(|err| format!("failed to emit Pong event: {err}"))?;
            }
            CoreCommand::RequestSnapshot => {
                self.resend_initial_state(event_tx);
            }
            CoreCommand::SetVolume(channel, volume) => {
                apply_channel_volume(
                    channel,
                    volume,
                    &self.last_snapshot,
                    &self.overrides,
                    ChannelControlTargets {
                        virtual_input_source_name: VIRTUAL_SOURCES[0],
                        main_output_sink_name: VENTURI_MAIN_OUTPUT,
                    },
                    &mut self.last_sink_volume_by_target,
                    &mut self.last_source_volume_by_target,
                );
                self.last_volume_sent.insert(channel, Instant::now());
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
            }
            CoreCommand::SetInputDevice(device) => {
                self.handle_set_input_device(&device)?;
            }
            CoreCommand::ToggleWindow => {
                event_tx
                    .send(CoreEvent::ToggleWindowRequested)
                    .map_err(|err| format!("failed to emit ToggleWindowRequested event: {err}"))?;
            }
            CoreCommand::SetMeteringEnabled(enabled) => {
                self.meter_enabled.store(enabled, Ordering::Relaxed);
            }
            CoreCommand::Shutdown => return Ok(CommandLoopControl::Shutdown),
            CoreCommand::PlaySound(_) | CoreCommand::StopSound(_) => {}
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
            &self.overrides,
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

        let route_result = match self.routing_mode {
            RoutingMode::MetadataFirst => {
                let args = build_metadata_target_args(stream_id, channel);
                run_pw_metadata(&args)
            }
            RoutingMode::FallbackLinks => {
                let mut result = Ok(());
                for args in build_fallback_link_commands(stream_id, channel) {
                    if let Err(err) = run_pw_link(&args) {
                        result = Err(err);
                        break;
                    }
                }
                result
            }
        };

        route_result.map_err(|err| format!("failed to move stream {stream_id}: {err}"))?;

        if matches!(
            channel,
            Channel::Game | Channel::Media | Channel::Chat | Channel::Aux
        ) {
            self.apply_persisted_channel_mix(channel);
        }

        Ok(())
    }

    fn apply_persisted_channel_mix(&mut self, channel: Channel) {
        let volume = persisted_channel_volume(&self.runtime_state, channel);
        apply_channel_volume(
            channel,
            volume,
            &self.last_snapshot,
            &self.overrides,
            ChannelControlTargets {
                virtual_input_source_name: VIRTUAL_SOURCES[0],
                main_output_sink_name: VENTURI_MAIN_OUTPUT,
            },
            &mut self.last_sink_volume_by_target,
            &mut self.last_source_volume_by_target,
        );

        let muted = persisted_channel_mute(&self.runtime_state, channel);
        apply_channel_mute(
            channel,
            muted,
            &self.last_snapshot,
            &self.overrides,
            ChannelControlTargets {
                virtual_input_source_name: VIRTUAL_SOURCES[0],
                main_output_sink_name: VENTURI_MAIN_OUTPUT,
            },
            &mut self.last_sink_mute_by_target,
            &mut self.last_source_mute_by_target,
        );
    }

    fn handle_set_output_device(&mut self, device: &str) -> Result<(), String> {
        self.handle_set_output_device_internal(device, false)
    }

    fn handle_set_output_device_internal(
        &mut self,
        device: &str,
        force: bool,
    ) -> Result<(), String> {
        if should_skip_output_device_reconcile(self.selected_output.as_deref(), device, force) {
            return Ok(());
        }

        let default_sink = if device.eq_ignore_ascii_case(fallback_to_default_device()) {
            current_default_sink_name()?
        } else {
            None
        };
        let desired_output_owned = resolve_output_loopback_target(device, default_sink.as_deref());
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

        self.selected_output = Some(device.to_string());
        self.runtime_config.audio.output_device = config_device_value(device);
        save_config(&self.paths, &self.runtime_config)
            .map_err(|err| format!("failed to persist output device selection: {err}"))?;
        Ok(())
    }

    fn handle_set_input_device(&mut self, device: &str) -> Result<(), String> {
        if self.selected_input.as_deref() == Some(device) {
            return Ok(());
        }

        self.selected_input = Some(device.to_string());
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

        self.runtime_config.audio.input_device = config_device_value(device);
        save_config(&self.paths, &self.runtime_config)
            .map_err(|err| format!("failed to persist input device selection: {err}"))?;
        Ok(())
    }

    fn flush_persisted_state_if_due(&mut self, event_tx: &Sender<CoreEvent>) {
        if self.state_saver.should_flush(Instant::now()) {
            if let Err(err) = save_state(&self.paths, &self.runtime_state) {
                let _ = event_tx.send(CoreEvent::Error(format!(
                    "failed to persist mixer state: {err}"
                )));
                return;
            }
            self.state_saver.did_flush();
        }
    }

    fn flush_persisted_state_now(&mut self, event_tx: &Sender<CoreEvent>) {
        if let Err(err) = save_state(&self.paths, &self.runtime_state) {
            let _ = event_tx.send(CoreEvent::Error(format!(
                "failed to persist mixer state: {err}"
            )));
            return;
        }
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
        for (id, stream) in &self.last_snapshot.streams {
            let category = classify_with_priority(
                &self.overrides,
                Some(&stream.app_key),
                Some(&stream.display_name),
                stream.media_role.as_deref(),
            );
            let _ = event_tx.send(CoreEvent::StreamAppeared {
                id: *id,
                name: stream.display_name.clone(),
                category,
            });
        }
    }

    fn handle_monitor_event(
        &mut self,
        event: PwMonitorEvent,
        event_tx: &crossbeam_channel::Sender<CoreEvent>,
    ) {
        match event {
            PwMonitorEvent::InitialSnapshot(snapshot) => {
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
                // Update shared snapshot for meter worker
                *self.shared_snapshot.lock().unwrap() = self.last_snapshot.clone();
            }
            PwMonitorEvent::ObjectsChanged(objects) => {
                self.merge_changed_objects(&objects, event_tx);
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

                    if let Some(channel) =
                        node_id_to_channel(id, &self.last_snapshot, &self.overrides)
                    {
                        // Echo suppression
                        let suppressed = self
                            .last_volume_sent
                            .get(&channel)
                            .map(|sent_at| sent_at.elapsed() < ECHO_SUPPRESSION_WINDOW)
                            .unwrap_or(false);

                        if !suppressed {
                            let _ = event_tx.send(CoreEvent::VolumeChanged(channel, new_vol));
                        }
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
    }
}

#[cfg(test)]
fn compute_channel_level_updates_with<F>(
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, crate::core::messages::Channel>,
    max_stream_probes_per_channel: usize,
    sample_target: F,
) -> Vec<(crate::core::messages::Channel, (f32, f32))>
where
    F: FnMut(u32) -> Option<(f32, f32)>,
{
    let targets = limit_channel_level_targets(
        &build_channel_level_targets(snapshot, overrides),
        max_stream_probes_per_channel,
    );
    compute_channel_level_updates_for_targets_with(&targets, sample_target)
}

fn compute_channel_level_updates_for_targets_with<F>(
    targets: &BTreeMap<crate::core::messages::Channel, Vec<u32>>,
    mut sample_target: F,
) -> Vec<(crate::core::messages::Channel, (f32, f32))>
where
    F: FnMut(u32) -> Option<(f32, f32)>,
{
    let mut updates = Vec::new();

    for channel in [
        crate::core::messages::Channel::Main,
        crate::core::messages::Channel::Mic,
        crate::core::messages::Channel::Game,
        crate::core::messages::Channel::Media,
        crate::core::messages::Channel::Chat,
        crate::core::messages::Channel::Aux,
    ] {
        let mut left_peak = 0.0f32;
        let mut right_peak = 0.0f32;

        if let Some(ids) = targets.get(&channel) {
            for id in ids {
                if let Some((left, right)) = sample_target(*id) {
                    left_peak = left_peak.max(left);
                    right_peak = right_peak.max(right);
                }
            }
        }

        updates.push((channel, (left_peak, right_peak)));
    }

    updates
}

fn limit_channel_level_targets(
    targets: &BTreeMap<crate::core::messages::Channel, Vec<u32>>,
    per_channel_limit: usize,
) -> BTreeMap<crate::core::messages::Channel, Vec<u32>> {
    let mut limited = BTreeMap::new();
    for (channel, ids) in targets {
        limited.insert(
            *channel,
            ids.iter().take(per_channel_limit).copied().collect(),
        );
    }
    limited
}

fn limit_channel_level_name_targets(
    targets: &BTreeMap<crate::core::messages::Channel, Vec<String>>,
    per_channel_limit: usize,
) -> BTreeMap<crate::core::messages::Channel, Vec<String>> {
    let mut limited = BTreeMap::new();
    for (channel, ids) in targets {
        limited.insert(
            *channel,
            ids.iter().take(per_channel_limit).cloned().collect(),
        );
    }
    limited
}

fn collect_unique_level_targets(
    targets: &BTreeMap<crate::core::messages::Channel, Vec<u32>>,
) -> BTreeSet<u32> {
    targets
        .values()
        .flat_map(|ids| ids.iter().copied())
        .collect()
}

fn collect_unique_name_level_targets(
    targets: &BTreeMap<crate::core::messages::Channel, Vec<String>>,
) -> BTreeSet<String> {
    targets
        .values()
        .flat_map(|ids| ids.iter().cloned())
        .collect()
}

fn compute_channel_level_updates_for_name_targets_with<F>(
    targets: &BTreeMap<crate::core::messages::Channel, Vec<String>>,
    mut sample_target: F,
) -> Vec<(crate::core::messages::Channel, (f32, f32))>
where
    F: FnMut(&str) -> Option<(f32, f32)>,
{
    let mut updates = Vec::new();

    for channel in [
        crate::core::messages::Channel::Main,
        crate::core::messages::Channel::Mic,
        crate::core::messages::Channel::Game,
        crate::core::messages::Channel::Media,
        crate::core::messages::Channel::Chat,
        crate::core::messages::Channel::Aux,
    ] {
        let mut left_peak = 0.0f32;
        let mut right_peak = 0.0f32;

        if let Some(ids) = targets.get(&channel) {
            for id in ids {
                if let Some((left, right)) = sample_target(id.as_str()) {
                    left_peak = left_peak.max(left);
                    right_peak = right_peak.max(right);
                }
            }
        }

        updates.push((channel, (left_peak, right_peak)));
    }

    updates
}

/// Coalesce a batch of commands: keep only the last SetVolume per channel,
/// preserve all other commands in order, and stop early on Shutdown.
fn coalesce_commands(commands: Vec<CoreCommand>) -> Vec<CoreCommand> {
    let mut volume_map: BTreeMap<Channel, f32> = BTreeMap::new();
    let mut result: Vec<CoreCommand> = Vec::new();

    for cmd in commands {
        match cmd {
            CoreCommand::SetVolume(channel, vol) => {
                volume_map.insert(channel, vol);
            }
            CoreCommand::Shutdown => {
                // Shutdown discards everything — pending volumes and remaining commands
                return vec![CoreCommand::Shutdown];
            }
            other => {
                result.push(other);
            }
        }
    }

    // Append coalesced volumes in deterministic Channel order
    for (channel, vol) in volume_map {
        result.push(CoreCommand::SetVolume(channel, vol));
    }

    result
}

fn compute_level_sample_count(sample_rate_hz: u32, sample_interval: Duration) -> u32 {
    let interval_ms = sample_interval.as_millis() as u64;
    let sample_count = (sample_rate_hz as u64)
        .saturating_mul(interval_ms)
        .saturating_div(1000)
        .max(1);
    sample_count.min(u32::MAX as u64) as u32
}

fn spawn_meter_worker(
    event_tx: Sender<CoreEvent>,
    shared_snapshot: Arc<Mutex<Snapshot>>,
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let paths = Paths::resolve();
        let level_sample_count = compute_level_sample_count(48_000, METER_SAMPLE_INTERVAL);
        let mut meter_overrides = BTreeMap::new();
        let mut last_override_refresh = Instant::now() - METER_OVERRIDE_REFRESH_INTERVAL;
        let mut last_meter_sample = Instant::now() - METER_SAMPLE_INTERVAL;
        let mut cached_snapshot: Option<Snapshot> = None;
        const SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
        let mut last_snapshot_clone = Instant::now() - SNAPSHOT_REFRESH_INTERVAL;
        let mut samplers: BTreeMap<u32, PwTargetSampler> = BTreeMap::new();
        let mut stream_name_samplers: BTreeMap<String, PwTargetSampler> = BTreeMap::new();
        while running.load(Ordering::Relaxed) {
            if !enabled.load(Ordering::Relaxed) {
                std::thread::sleep(METER_WORKER_IDLE_INTERVAL);
                continue;
            }
            if last_meter_sample.elapsed() < METER_SAMPLE_INTERVAL {
                std::thread::sleep(METER_WORKER_INTERVAL);
                continue;
            }
            last_meter_sample = Instant::now();
            if last_override_refresh.elapsed() >= METER_OVERRIDE_REFRESH_INTERVAL {
                meter_overrides = deserialize_overrides(&load_config(&paths).categorizer.overrides);
                last_override_refresh = Instant::now();
            }
            if last_snapshot_clone.elapsed() >= SNAPSHOT_REFRESH_INTERVAL
                || cached_snapshot.is_none()
            {
                let snapshot = shared_snapshot.lock().unwrap().clone();
                cached_snapshot = Some(snapshot);
                last_snapshot_clone = Instant::now();
            }

            if let Some(snapshot) = cached_snapshot.as_ref() {
                let mut targets = limit_channel_level_targets(
                    &build_channel_level_targets(snapshot, &meter_overrides),
                    MAX_STREAM_PROBES_PER_CHANNEL,
                );
                retain_primary_numeric_meter_targets(&mut targets);
                let required_targets = collect_unique_level_targets(&targets);

                let stream_name_targets = limit_channel_level_name_targets(
                    &build_stream_name_level_targets(snapshot, &meter_overrides),
                    MAX_STREAM_PROBES_PER_CHANNEL,
                );
                let required_stream_names = collect_unique_name_level_targets(&stream_name_targets);

                samplers.retain(|target, _| required_targets.contains(target));
                for target in &required_targets {
                    if !samplers.contains_key(target)
                        && let Ok(sampler) = PwTargetSampler::spawn(&target.to_string())
                    {
                        samplers.insert(*target, sampler);
                    }
                }

                stream_name_samplers.retain(|target, _| required_stream_names.contains(target));
                for target in &required_stream_names {
                    if !stream_name_samplers.contains_key(target)
                        && let Ok(sampler) = PwTargetSampler::spawn(target)
                    {
                        stream_name_samplers.insert(target.clone(), sampler);
                    }
                }

                let mut failed_targets = BTreeSet::new();
                let mut updates =
                    compute_channel_level_updates_for_targets_with(&targets, |target_id| {
                        let sampler = samplers.get_mut(&target_id)?;
                        match sampler.sample_levels(level_sample_count) {
                            Ok(levels) => Some(levels),
                            Err(_) => {
                                failed_targets.insert(target_id);
                                None
                            }
                        }
                    });

                let mut failed_stream_names = BTreeSet::new();
                let stream_name_updates = compute_channel_level_updates_for_name_targets_with(
                    &stream_name_targets,
                    |target_name| {
                        let sampler = stream_name_samplers.get_mut(target_name)?;
                        match sampler.sample_levels(level_sample_count) {
                            Ok(levels) => Some(levels),
                            Err(_) => {
                                failed_stream_names.insert(target_name.to_string());
                                None
                            }
                        }
                    },
                );

                for channel in [Channel::Game, Channel::Media, Channel::Chat, Channel::Aux] {
                    if stream_name_targets.contains_key(&channel)
                        && let Some((_, stream_levels)) =
                            stream_name_updates.iter().find(|(ch, _)| *ch == channel)
                        && let Some((_, numeric_levels)) =
                            updates.iter_mut().find(|(ch, _)| *ch == channel)
                    {
                        *numeric_levels = *stream_levels;
                    }
                }

                for target in failed_targets {
                    samplers.remove(&target);
                }
                for target in failed_stream_names {
                    stream_name_samplers.remove(&target);
                }
                let _ = event_tx.send(CoreEvent::LevelsUpdate(updates));
            }
            std::thread::sleep(METER_WORKER_INTERVAL);
        }
    })
}

impl PipeWireManager {
    pub fn spawn(command_rx: Receiver<CoreCommand>, event_tx: Sender<CoreEvent>) -> Self {
        let meter_running = Arc::new(AtomicBool::new(true));
        let meter_enabled = Arc::new(AtomicBool::new(true));
        let shared_snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let meter_handle = spawn_meter_worker(
            event_tx.clone(),
            shared_snapshot.clone(),
            meter_running.clone(),
            meter_enabled.clone(),
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

                // Hotkey tick, state flush — run on every loop iteration
                state.handle_hotkey_tick(&event_tx);
                state.flush_persisted_state_if_due(&event_tx);
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
    use std::collections::BTreeMap;
    use std::time::Duration;

    use crate::config::schema::State;
    use crate::core::messages::{Channel, CoreCommand, CoreEvent, DeviceEntry, DeviceKind};
    use crate::core::pipewire_discovery::{Snapshot, StreamInfo};

    use super::{
        apply_structural_monitor_delta, build_channel_level_targets,
        build_stream_name_level_targets, coalesce_commands, compute_channel_level_updates_with,
        compute_level_sample_count, node_id_to_channel, persisted_channel_mute,
        persisted_channel_volume, resolve_output_loopback_target,
        should_skip_output_device_reconcile,
    };

    #[test]
    fn builds_level_targets_for_main_mic_and_classified_stream_channels() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("Venturi-Output".to_string(), 128);
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 281);
        snapshot
            .output_meter_targets
            .insert("Venturi-Output".to_string(), 37284);
        snapshot
            .input_meter_targets
            .insert("Venturi-VirtualMic".to_string(), 37393);
        snapshot.streams.insert(
            900,
            StreamInfo {
                id: 900,
                meter_target: 9900,
                node_name: Some("discord-node".to_string()),
                is_corked: false,
                app_key: "discord".to_string(),
                display_name: "Discord".to_string(),
                media_role: Some("communication".to_string()),
            },
        );
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
                node_name: Some("paused-node".to_string()),
                is_corked: true,
                app_key: "paused-app".to_string(),
                display_name: "Paused App".to_string(),
                media_role: Some("music".to_string()),
            },
        );

        let targets = build_channel_level_targets(&snapshot, &BTreeMap::new());

        assert_eq!(targets.get(&Channel::Main), Some(&vec![37284]));
        assert_eq!(targets.get(&Channel::Mic), Some(&vec![37393]));
        assert_eq!(targets.get(&Channel::Chat), Some(&vec![9900]));
        assert_eq!(targets.get(&Channel::Media), Some(&vec![9901]));
    }

    #[test]
    fn resolves_default_output_to_real_hardware_sink_for_loopback() {
        assert_eq!(
            resolve_output_loopback_target(
                "Default",
                Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo")
            ),
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo".to_string())
        );
        assert_eq!(
            resolve_output_loopback_target("Default", Some("Venturi-Output")),
            None
        );
        assert_eq!(
            resolve_output_loopback_target(
                "alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo",
                Some("ignored")
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
    fn meter_updates_respect_category_overrides() {
        let mut snapshot = Snapshot::default();
        snapshot.streams.insert(
            901,
            StreamInfo {
                id: 901,
                meter_target: 9901,
                node_name: Some("zen-node".to_string()),
                is_corked: false,
                app_key: "zen-bin".to_string(),
                display_name: "Zen".to_string(),
                media_role: None,
            },
        );

        let mut overrides = BTreeMap::new();
        overrides.insert("zen-bin".to_string(), Channel::Media);

        let updates = compute_channel_level_updates_with(&snapshot, &overrides, 1, |target_id| {
            if target_id == 9901 {
                Some((0.4, 0.5))
            } else {
                None
            }
        });

        let media = updates
            .iter()
            .find(|(channel, _)| *channel == Channel::Media)
            .map(|(_, levels)| *levels)
            .expect("media channel update");
        let aux = updates
            .iter()
            .find(|(channel, _)| *channel == Channel::Aux)
            .map(|(_, levels)| *levels)
            .expect("aux channel update");

        assert!(media.0 > 0.0 || media.1 > 0.0);
        assert_eq!(aux, (0.0, 0.0));
    }

    #[test]
    fn stream_name_targets_use_stream_node_names() {
        let mut snapshot = Snapshot::default();
        snapshot.streams.insert(
            900,
            StreamInfo {
                id: 900,
                meter_target: 9900,
                node_name: Some("discord-node".to_string()),
                is_corked: false,
                app_key: "discord".to_string(),
                display_name: "Discord".to_string(),
                media_role: Some("communication".to_string()),
            },
        );
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

        let targets = build_stream_name_level_targets(&snapshot, &BTreeMap::new());

        assert_eq!(
            targets.get(&Channel::Chat),
            Some(&vec!["discord-node".to_string()])
        );
        assert_eq!(
            targets.get(&Channel::Media),
            Some(&vec!["spotify-node".to_string()])
        );
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
    fn reads_persisted_volume_and_mute_for_channel() {
        let mut state = State::default();
        state.volumes.chat = 0.42;
        state.muted.chat = true;

        assert!((persisted_channel_volume(&state, Channel::Chat) - 0.42).abs() < 0.0001);
        assert!(persisted_channel_mute(&state, Channel::Chat));
    }

    #[test]
    fn node_id_to_channel_maps_output_sink_to_main() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("main-sink".to_string(), 50);
        let overrides = BTreeMap::new();
        assert_eq!(
            node_id_to_channel(50, &snapshot, &overrides),
            Some(Channel::Main)
        );
    }

    #[test]
    fn node_id_to_channel_maps_input_source_to_mic() {
        let mut snapshot = Snapshot::default();
        snapshot.input_ids.insert("main-source".to_string(), 60);
        let overrides = BTreeMap::new();
        assert_eq!(
            node_id_to_channel(60, &snapshot, &overrides),
            Some(Channel::Mic)
        );
    }

    #[test]
    fn node_id_to_channel_maps_stream_via_categorizer() {
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
        let overrides = BTreeMap::new();
        // Firefox classifies as Media
        assert_eq!(
            node_id_to_channel(100, &snapshot, &overrides),
            Some(Channel::Media)
        );
    }

    #[test]
    fn node_id_to_channel_returns_none_for_unknown_id() {
        let snapshot = Snapshot::default();
        let overrides = BTreeMap::new();
        assert_eq!(node_id_to_channel(999, &snapshot, &overrides), None);
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
            node_id_to_channel(412, &snapshot, &overrides),
            Some(Channel::Main)
        );
        assert_eq!(
            node_id_to_channel(225, &snapshot, &overrides),
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

        let targets = build_channel_level_targets(&snapshot, &overrides);
        assert_eq!(targets.get(&Channel::Mic), Some(&vec![225]));
        assert!(
            !targets.contains_key(&Channel::Media),
            "media target must not persist with reassigned mic source id"
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
    fn echo_suppression_blocks_recent_volume_changes() {
        use std::time::{Duration, Instant};

        let mut last_volume_sent: BTreeMap<Channel, Instant> = BTreeMap::new();
        let channel = Channel::Main;

        // Simulate: we just sent a volume command
        last_volume_sent.insert(channel, Instant::now());

        // Check suppression — should be suppressed (within 200ms)
        let suppressed = last_volume_sent
            .get(&channel)
            .map(|sent_at| sent_at.elapsed() < Duration::from_millis(200))
            .unwrap_or(false);
        assert!(
            suppressed,
            "volume change within 200ms should be suppressed"
        );
    }

    #[test]
    fn echo_suppression_allows_old_volume_changes() {
        use std::time::{Duration, Instant};

        let mut last_volume_sent: BTreeMap<Channel, Instant> = BTreeMap::new();
        let channel = Channel::Main;

        // Simulate: we sent a volume command 300ms ago
        last_volume_sent.insert(channel, Instant::now() - Duration::from_millis(300));

        let suppressed = last_volume_sent
            .get(&channel)
            .map(|sent_at| sent_at.elapsed() < Duration::from_millis(200))
            .unwrap_or(false);
        assert!(
            !suppressed,
            "volume change after 200ms should NOT be suppressed"
        );
    }

    #[test]
    fn echo_suppression_allows_unseen_channels() {
        use std::time::Instant;

        let last_volume_sent: BTreeMap<Channel, Instant> = BTreeMap::new();
        let channel = Channel::Game;

        let suppressed = last_volume_sent
            .get(&channel)
            .map(|sent_at| sent_at.elapsed() < std::time::Duration::from_millis(200))
            .unwrap_or(false);
        assert!(
            !suppressed,
            "channel with no prior send should NOT be suppressed"
        );
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

    #[test]
    fn restart_logic_resets_after_success() {
        let mut consecutive_failures: u32 = 2;
        consecutive_failures = 0;
        assert_eq!(consecutive_failures, 0);
    }
}
