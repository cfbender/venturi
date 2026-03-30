use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::categorizer::learning::{deserialize_overrides, serialize_overrides};
use crate::categorizer::rules::classify_with_priority;
use crate::config::persistence::{Paths, ensure_dirs, load_config, save_config};
use crate::core::hotkeys::{
    HotkeyAdapter, HotkeyBindings, HotkeyState, build_adapter, collect_adapter_commands,
};
use crate::core::messages::{CoreCommand, CoreEvent};
use crate::core::pipewire_backend::{
    current_default_source_name, ensure_virtual_devices, load_monitor_loopback_module,
    rewire_virtual_mic_source, run_pw_link, run_pw_metadata, unload_pactl_module,
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

pub struct PipeWireManager {
    handle: std::thread::JoinHandle<()>,
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
    selected_input: Option<String>,
    output_loopback_module: Option<String>,
    virtual_mic_module: Option<String>,
    last_sink_volume_by_target: BTreeMap<String, f32>,
    last_source_volume_by_target: BTreeMap<String, f32>,
    last_sink_mute_by_target: BTreeMap<String, bool>,
    last_source_mute_by_target: BTreeMap<String, bool>,
}

impl CoreRuntimeState {
    fn initialize(event_tx: &Sender<CoreEvent>) -> Self {
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
            selected_input,
            output_loopback_module: None,
            virtual_mic_module: None,
            last_sink_volume_by_target: BTreeMap::new(),
            last_source_volume_by_target: BTreeMap::new(),
            last_sink_mute_by_target: BTreeMap::new(),
            last_source_mute_by_target: BTreeMap::new(),
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
        if let Some(prev_module) = self.output_loopback_module.take()
            && let Err(err) = unload_pactl_module(&prev_module)
        {
            return Err(format!(
                "failed to unload monitor loopback module {prev_module}: {err}"
            ));
        }
        if device != fallback_to_default_device() {
            match load_monitor_loopback_module(VENTURI_MAIN_MONITOR, device) {
                Ok(module_id) => {
                    self.output_loopback_module = Some(module_id);
                }
                Err(err) => {
                    return Err(format!(
                        "failed to route Venturi main mix to {device}: {err}"
                    ));
                }
            }
        }
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

impl PipeWireManager {
    pub fn spawn(command_rx: Receiver<CoreCommand>, event_tx: Sender<CoreEvent>) -> Self {
        let handle = std::thread::spawn(move || {
            let _ = event_tx.send(CoreEvent::Ready);
            let mut state = CoreRuntimeState::initialize(&event_tx);

            loop {
                match command_rx.recv_timeout(POLL_INTERVAL) {
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
                state.refresh_snapshot(&event_tx);
            }
        });
        Self { handle }
    }

    pub fn join(self) -> std::thread::Result<()> {
        self.handle.join()
    }
}
