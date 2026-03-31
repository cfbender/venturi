use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::categorizer::learning::{deserialize_overrides, serialize_overrides};
use crate::categorizer::rules::classify_with_priority;
use crate::config::persistence::{Paths, ensure_dirs, load_config, save_config};
use crate::core::hotkeys::{
    HotkeyAdapter, HotkeyBindings, HotkeyState, build_adapter, collect_adapter_commands,
};
use crate::core::messages::{CoreCommand, CoreEvent};
use crate::core::pipewire_backend::{
    current_default_sink_name, current_default_source_name, ensure_virtual_devices,
    reconcile_monitor_loopback_modules, rewire_virtual_mic_source, run_pw_link, run_pw_metadata,
    unload_pactl_module, PwTargetSampler,
};
use crate::core::pipewire_channel_control::{
    ChannelControlTargets, apply_channel_mute, apply_channel_volume,
};
use crate::core::pipewire_discovery::{Snapshot, poll_snapshot};
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

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const LOOP_TICK_INTERVAL: Duration = Duration::from_millis(50);
const LEVEL_POLL_INTERVAL: Duration = Duration::from_millis(180);
const MAX_STREAM_PROBES_PER_CHANNEL: usize = 1;
const ENABLE_LEVEL_POLLING: bool = false;
const METER_WORKER_INTERVAL: Duration = Duration::from_millis(33);
const METER_WORKER_IDLE_INTERVAL: Duration = Duration::from_millis(500);
const METER_OVERRIDE_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
const METER_SAMPLE_INTERVAL: Duration = Duration::from_millis(66);
const METER_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_millis(750);
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
    meter_enabled: Arc<AtomicBool>,
}

impl CoreRuntimeState {
    fn initialize(event_tx: &Sender<CoreEvent>, meter_enabled: Arc<AtomicBool>) -> Self {
        let routing_mode =
            routing_mode_from_flag(std::env::var(FORCE_LINK_ROUTING_ENV).ok().as_deref());
        let paths = Paths::resolve();
        if let Err(err) = ensure_dirs(&paths) {
            let _ = event_tx.send(CoreEvent::Error(format!(
                "failed to prepare config/state directories: {err}"
            )));
        }

        let runtime_config = load_config(&paths);
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
            meter_enabled,
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

        route_result.map_err(|err| format!("failed to move stream {stream_id}: {err}"))
    }

    fn handle_set_output_device(&mut self, device: &str) -> Result<(), String> {
        self.handle_set_output_device_internal(device, false)
    }

    fn handle_set_output_device_internal(&mut self, device: &str, force: bool) -> Result<(), String> {
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

    fn refresh_snapshot(&mut self, event_tx: &Sender<CoreEvent>) {
        let hidden_outputs = [VENTURI_MAIN_OUTPUT];
        match poll_snapshot(hidden_outputs.as_slice(), VIRTUAL_SOURCES.as_slice()) {
            Ok(snapshot) => {
                if snapshot.devices != self.last_snapshot.devices {
                    let _ = event_tx.send(CoreEvent::DevicesChanged(snapshot.devices.clone()));
                }

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

                for id in self.last_snapshot.streams.keys() {
                    if !snapshot.streams.contains_key(id) {
                        let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
                    }
                }

                self.last_snapshot = snapshot;
            }
            Err(err) => {
                let _ = event_tx.send(CoreEvent::Error(err));
            }
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
        limited.insert(*channel, ids.iter().take(per_channel_limit).copied().collect());
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

fn should_refresh_meter_snapshot(
    snapshot_missing: bool,
    refresh_interval: Duration,
    elapsed: Duration,
) -> bool {
    snapshot_missing || elapsed >= refresh_interval
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
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let hidden_outputs = [VENTURI_MAIN_OUTPUT];
        let paths = Paths::resolve();
        let level_sample_count = compute_level_sample_count(48_000, METER_SAMPLE_INTERVAL);
        let mut meter_overrides = BTreeMap::new();
        let mut last_override_refresh = Instant::now() - METER_OVERRIDE_REFRESH_INTERVAL;
        let mut last_meter_sample = Instant::now() - METER_SAMPLE_INTERVAL;
        let mut cached_snapshot: Option<Snapshot> = None;
        let mut last_snapshot_refresh = Instant::now() - METER_SNAPSHOT_REFRESH_INTERVAL;
        let mut samplers: BTreeMap<u32, PwTargetSampler> = BTreeMap::new();
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
                meter_overrides =
                    deserialize_overrides(&load_config(&paths).categorizer.overrides);
                last_override_refresh = Instant::now();
            }
            if should_refresh_meter_snapshot(
                cached_snapshot.is_none(),
                METER_SNAPSHOT_REFRESH_INTERVAL,
                last_snapshot_refresh.elapsed(),
            ) {
                if let Ok(snapshot) = poll_snapshot(hidden_outputs.as_slice(), VIRTUAL_SOURCES.as_slice()) {
                    cached_snapshot = Some(snapshot);
                    last_snapshot_refresh = Instant::now();
                }
            }

            if let Some(snapshot) = cached_snapshot.as_ref() {
                let targets = limit_channel_level_targets(
                    &build_channel_level_targets(snapshot, &meter_overrides),
                    MAX_STREAM_PROBES_PER_CHANNEL,
                );
                let required_targets = collect_unique_level_targets(&targets);

                samplers.retain(|target, _| required_targets.contains(target));
                for target in &required_targets {
                    if !samplers.contains_key(target)
                        && let Ok(sampler) = PwTargetSampler::spawn(*target)
                    {
                        samplers.insert(*target, sampler);
                    }
                }

                let mut failed_targets = BTreeSet::new();
                let updates = compute_channel_level_updates_for_targets_with(&targets, |target_id| {
                    let sampler = samplers.get_mut(&target_id)?;
                    match sampler.sample_levels(level_sample_count) {
                        Ok(levels) => Some(levels),
                        Err(_) => {
                            failed_targets.insert(target_id);
                            None
                        }
                    }
                });
                for target in failed_targets {
                    samplers.remove(&target);
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
        let meter_handle = spawn_meter_worker(
            event_tx.clone(),
            meter_running.clone(),
            meter_enabled.clone(),
        );
        let meter_running_for_core = meter_running.clone();
        let meter_enabled_for_core = meter_enabled.clone();
        let handle = std::thread::spawn(move || {
            let _ = event_tx.send(CoreEvent::Ready);
            let mut state = CoreRuntimeState::initialize(&event_tx, meter_enabled_for_core);
            let mut last_snapshot_poll = Instant::now() - POLL_INTERVAL;
            let mut last_level_poll = Instant::now() - LEVEL_POLL_INTERVAL;

            loop {
                match command_rx.recv_timeout(LOOP_TICK_INTERVAL) {
                    Ok(command) => match state.handle_core_command(command, &event_tx) {
                        Ok(CommandLoopControl::Continue) => {}
                        Ok(CommandLoopControl::Shutdown) => break,
                        Err(err) => {
                            let _ = event_tx
                                .send(CoreEvent::Error(format!("command handling failed: {err}")));
                        }
                    },
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }

                state.handle_hotkey_tick(&event_tx);

                if last_snapshot_poll.elapsed() >= POLL_INTERVAL {
                    state.refresh_snapshot(&event_tx);
                    last_snapshot_poll = Instant::now();
                }

                if ENABLE_LEVEL_POLLING && last_level_poll.elapsed() >= LEVEL_POLL_INTERVAL {
                    last_level_poll = Instant::now();
                }
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

    use crate::core::messages::Channel;
    use crate::core::pipewire_discovery::{Snapshot, StreamInfo};

    use super::{
        build_channel_level_targets, compute_channel_level_updates_with,
        compute_level_sample_count,
        resolve_output_loopback_target, should_refresh_meter_snapshot,
        should_skip_output_device_reconcile,
    };

    #[test]
    fn builds_level_targets_for_main_mic_and_classified_stream_channels() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Output".to_string(), 128);
        snapshot.input_ids.insert("Venturi-VirtualMic".to_string(), 281);
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
                app_key: "spotify".to_string(),
                display_name: "Spotify".to_string(),
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
            resolve_output_loopback_target("Default", Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo")),
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo".to_string())
        );
        assert_eq!(
            resolve_output_loopback_target("Default", Some("Venturi-Output")),
            None
        );
        assert_eq!(
            resolve_output_loopback_target("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo", Some("ignored")),
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
    fn refreshes_meter_snapshot_when_missing_or_interval_elapsed() {
        let refresh_interval = Duration::from_millis(750);
        assert!(should_refresh_meter_snapshot(
            true,
            refresh_interval,
            Duration::from_millis(0),
        ));
        assert!(should_refresh_meter_snapshot(
            false,
            refresh_interval,
            Duration::from_millis(750),
        ));
        assert!(!should_refresh_meter_snapshot(
            false,
            refresh_interval,
            Duration::from_millis(300),
        ));
    }

    #[test]
    fn computes_meter_sample_count_for_sampling_interval() {
        assert_eq!(compute_level_sample_count(48_000, Duration::from_millis(66)), 3168);
        assert_eq!(compute_level_sample_count(48_000, Duration::from_millis(1)), 48);
    }

}
