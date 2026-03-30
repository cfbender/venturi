use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Duration;

use crate::categorizer::learning::{deserialize_overrides, serialize_overrides};
use crate::categorizer::rules::classify_with_priority;
use crate::config::persistence::{Paths, ensure_dirs, load_config, save_config};
use crate::core::hotkeys::{HotkeyBindings, HotkeyState, build_adapter, collect_adapter_commands};
use crate::core::messages::{CoreCommand, CoreEvent};
use crate::core::router::{
    FORCE_LINK_ROUTING_ENV, RoutingMode, build_fallback_link_commands, build_metadata_target_args,
    resolve_input_target, resolve_output_target, routing_mode_from_flag,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamInfo {
    id: u32,
    app_key: String,
    display_name: String,
    media_role: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Snapshot {
    devices: Vec<String>,
    output_ids: BTreeMap<String, u32>,
    input_ids: BTreeMap<String, u32>,
    streams: BTreeMap<u32, StreamInfo>,
}

fn parse_pw_dump(raw: &str) -> Result<Snapshot, String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let arr = value
        .as_array()
        .ok_or_else(|| "pw-dump root is not array".to_string())?;

    let mut outputs = BTreeSet::new();
    let mut inputs = BTreeSet::new();
    let mut output_ids = BTreeMap::new();
    let mut input_ids = BTreeMap::new();
    let mut streams = BTreeMap::new();

    for item in arr {
        let id = item
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok());

        let props = item
            .get("info")
            .and_then(|v| v.get("props"))
            .and_then(Value::as_object);

        let Some(props) = props else {
            continue;
        };

        let media_class = props
            .get("media.class")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let node_name = props
            .get("node.name")
            .and_then(Value::as_str)
            .or_else(|| props.get("node.nick").and_then(Value::as_str))
            .or_else(|| props.get("node.description").and_then(Value::as_str))
            .unwrap_or_default();

        if media_class.contains("Sink") && !node_name.is_empty() {
            if node_name != VENTURI_MAIN_OUTPUT {
                outputs.insert(format!("out:{node_name}"));
            }
            if let Some(node_id) = id {
                output_ids.insert(node_name.to_string(), node_id);
            }
        }
        if media_class.contains("Source") && !node_name.is_empty() {
            if node_name.ends_with(".monitor") {
                continue;
            }
            if let Some(node_id) = id {
                input_ids.insert(node_name.to_string(), node_id);
            }
            if VIRTUAL_SOURCES.contains(&node_name) {
                continue;
            }
            inputs.insert(format!("in:{node_name}"));
        }

        if media_class == "Stream/Output/Audio" || media_class == "Audio/Stream/Output" {
            let Some(stream_id) = id else {
                continue;
            };
            let app_name = props
                .get("application.name")
                .and_then(Value::as_str)
                .unwrap_or("Unknown App");
            let binary = props
                .get("application.process.binary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let display_name = if !binary.is_empty()
                && !binary.eq_ignore_ascii_case("webrtc")
                && !binary.eq_ignore_ascii_case("voiceengine")
                && !binary.eq_ignore_ascii_case("webrtc voiceengine")
            {
                binary
            } else {
                app_name
            };
            let role = props
                .get("media.role")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            streams.insert(
                stream_id,
                StreamInfo {
                    id: stream_id,
                    app_key: if binary.is_empty() {
                        app_name.to_ascii_lowercase()
                    } else {
                        binary.to_ascii_lowercase()
                    },
                    display_name: display_name.to_string(),
                    media_role: role,
                },
            );
        }
    }

    let mut devices = Vec::with_capacity(outputs.len() + inputs.len());
    devices.extend(outputs);
    devices.extend(inputs);

    Ok(Snapshot {
        devices,
        output_ids,
        input_ids,
        streams,
    })
}

fn poll_snapshot() -> Result<Snapshot, String> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|e| format!("failed to run pw-dump: {e}"))?;
    if !output.status.success() {
        return Err(format!("pw-dump exited with {}", output.status));
    }
    let raw = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_pw_dump(&raw)
}

fn run_command(program: &str, args: &[String]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run {program}: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}"))
    }
}

fn run_wpctl(args: &[String]) {
    let _ = run_command("wpctl", args);
}

fn run_wpctl_checked(args: &[String]) -> Result<(), String> {
    run_command("wpctl", args)
}

fn run_pactl(args: &[String]) -> Result<String, String> {
    let output = Command::new("pactl")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run pactl: {e}"))?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "pactl exited with {}: {}",
            output.status,
            stderr.trim()
        ))
    }
}

fn parse_pactl_short_names(raw: &str) -> BTreeSet<String> {
    raw.lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let _index = cols.next()?;
            let name = cols.next()?;
            Some(name.to_string())
        })
        .collect()
}

fn unload_legacy_venturi_sinks() -> Result<(), String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;

    for line in raw.lines() {
        let mut cols = line.split_whitespace();
        let Some(module_id) = cols.next() else {
            continue;
        };
        let Some(module_name) = cols.next() else {
            continue;
        };
        if module_name != "module-null-sink" {
            continue;
        }

        if LEGACY_VENTURI_SINKS
            .iter()
            .any(|legacy| line.contains(&format!("sink_name={legacy}")))
        {
            let unload_args = vec!["unload-module".to_string(), module_id.to_string()];
            let _ = run_pactl(&unload_args)?;
        }
    }

    Ok(())
}

fn ensure_virtual_devices() -> Result<(), String> {
    unload_legacy_venturi_sinks()?;

    let list_sinks_args = vec!["list".to_string(), "short".to_string(), "sinks".to_string()];
    let list_sources_args = vec![
        "list".to_string(),
        "short".to_string(),
        "sources".to_string(),
    ];
    let existing_sinks_raw = run_pactl(&list_sinks_args)?;
    let existing_sources_raw = run_pactl(&list_sources_args)?;

    let existing_sinks = parse_pactl_short_names(&existing_sinks_raw);
    let existing_sources = parse_pactl_short_names(&existing_sources_raw);

    for sink in &VIRTUAL_SINKS {
        if existing_sinks.contains(*sink) {
            continue;
        }
        let args = vec![
            "load-module".to_string(),
            "module-null-sink".to_string(),
            format!("sink_name={sink}"),
            format!("sink_properties=device.description={sink}"),
        ];
        run_pactl(&args)?;
    }

    for source in &VIRTUAL_SOURCES {
        if existing_sources.contains(*source) {
            continue;
        }

        let default_source = current_default_source_name()?.ok_or_else(|| {
            "no default source available to create Venturi virtual mic".to_string()
        })?;
        let args = vec![
            "load-module".to_string(),
            "module-remap-source".to_string(),
            format!("master={default_source}"),
            format!("source_name={source}"),
            format!("source_properties=device.description={source}"),
        ];
        run_pactl(&args)?;
    }

    Ok(())
}

fn current_default_source_name() -> Result<Option<String>, String> {
    let args = vec!["info".to_string()];
    let raw = run_pactl(&args)?;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Default Source:") {
            let name = rest.trim();
            if !name.is_empty() {
                return Ok(Some(name.to_string()));
            }
        }
    }
    Ok(None)
}

fn find_virtual_mic_module_id() -> Result<Option<String>, String> {
    let args = vec![
        "list".to_string(),
        "short".to_string(),
        "modules".to_string(),
    ];
    let raw = run_pactl(&args)?;

    for line in raw.lines() {
        if line.contains("module-remap-source")
            && line.contains(&format!("source_name={}", VIRTUAL_SOURCES[0]))
        {
            let mut cols = line.split_whitespace();
            if let Some(id) = cols.next() {
                return Ok(Some(id.to_string()));
            }
        }
    }

    Ok(None)
}

fn rewire_virtual_mic_source(master_source: &str) -> Result<String, String> {
    if let Some(module_id) = find_virtual_mic_module_id()? {
        unload_pactl_module(&module_id)?;
    }

    let args = vec![
        "load-module".to_string(),
        "module-remap-source".to_string(),
        format!("master={master_source}"),
        format!("source_name={}", VIRTUAL_SOURCES[0]),
        format!(
            "source_properties=device.description={}",
            VIRTUAL_SOURCES[0]
        ),
    ];
    run_pactl(&args).map(|stdout| stdout.trim().to_string())
}

fn resolve_selected_input_name(selected_input: Option<&str>) -> Result<Option<String>, String> {
    match selected_input {
        Some(name) if !name.is_empty() && name != fallback_to_default_device() => {
            Ok(Some(name.to_string()))
        }
        _ => current_default_source_name(),
    }
}

fn load_monitor_loopback_module(output_device: &str) -> Result<String, String> {
    let args = vec![
        "load-module".to_string(),
        "module-loopback".to_string(),
        format!("source={VENTURI_MAIN_MONITOR}"),
        format!("sink={output_device}"),
        "latency_msec=1".to_string(),
    ];
    run_pactl(&args).map(|stdout| stdout.trim().to_string())
}

fn unload_pactl_module(module_id: &str) -> Result<(), String> {
    if module_id.is_empty() {
        return Ok(());
    }
    let args = vec!["unload-module".to_string(), module_id.to_string()];
    run_pactl(&args).map(|_| ())
}

fn apply_channel_volume(
    channel: crate::core::messages::Channel,
    volume: f32,
    _selected_input: Option<&str>,
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, crate::core::messages::Channel>,
    last_sink_volume_by_target: &mut BTreeMap<String, f32>,
    last_source_volume_by_target: &mut BTreeMap<String, f32>,
) {
    use crate::core::messages::Channel;

    match channel {
        Channel::Mic => {
            let target = resolve_input_target(Some(VIRTUAL_SOURCES[0]), &snapshot.input_ids);
            let changed = last_source_volume_by_target
                .get(&target)
                .map(|prev| (*prev - volume).abs() >= 0.01)
                .unwrap_or(true);
            if changed {
                let args = vec!["set-volume".to_string(), target.clone(), volume.to_string()];
                run_wpctl(&args);
                last_source_volume_by_target.insert(target, volume);
            }
        }
        Channel::Main => {
            let target = resolve_output_target(Some(VENTURI_MAIN_OUTPUT), &snapshot.output_ids);
            let changed = last_sink_volume_by_target
                .get(&target)
                .map(|prev| (*prev - volume).abs() >= 0.01)
                .unwrap_or(true);
            if changed {
                let args = vec!["set-volume".to_string(), target.clone(), volume.to_string()];
                run_wpctl(&args);
                last_sink_volume_by_target.insert(target, volume);
            }
        }
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            for stream in snapshot.streams.values() {
                let stream_channel = classify_with_priority(
                    overrides,
                    Some(&stream.app_key),
                    Some(&stream.display_name),
                    stream.media_role.as_deref(),
                );
                if stream_channel == channel {
                    let args = vec![
                        "set-volume".to_string(),
                        stream.id.to_string(),
                        volume.to_string(),
                    ];
                    let _ = run_wpctl_checked(&args);
                }
            }
        }
    }
}

fn apply_channel_mute(
    channel: crate::core::messages::Channel,
    muted: bool,
    _selected_input: Option<&str>,
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, crate::core::messages::Channel>,
    last_sink_mute_by_target: &mut BTreeMap<String, bool>,
    last_source_mute_by_target: &mut BTreeMap<String, bool>,
) {
    use crate::core::messages::Channel;

    let value = if muted { "1" } else { "0" };
    match channel {
        Channel::Mic => {
            let target = resolve_input_target(Some(VIRTUAL_SOURCES[0]), &snapshot.input_ids);
            if last_source_mute_by_target.get(&target) != Some(&muted) {
                let args = vec!["set-mute".to_string(), target.clone(), value.to_string()];
                run_wpctl(&args);
                last_source_mute_by_target.insert(target, muted);
            }
        }
        Channel::Main => {
            let target = resolve_output_target(Some(VENTURI_MAIN_OUTPUT), &snapshot.output_ids);
            if last_sink_mute_by_target.get(&target) != Some(&muted) {
                let args = vec!["set-mute".to_string(), target.clone(), value.to_string()];
                run_wpctl(&args);
                last_sink_mute_by_target.insert(target, muted);
            }
        }
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            for stream in snapshot.streams.values() {
                let stream_channel = classify_with_priority(
                    overrides,
                    Some(&stream.app_key),
                    Some(&stream.display_name),
                    stream.media_role.as_deref(),
                );
                if stream_channel == channel {
                    let args = vec![
                        "set-mute".to_string(),
                        stream.id.to_string(),
                        value.to_string(),
                    ];
                    let _ = run_wpctl_checked(&args);
                }
            }
        }
    }
}

fn run_pw_metadata(args: &[String]) -> Result<(), String> {
    run_command("pw-metadata", args)
}

fn run_pw_link(args: &[String]) -> Result<(), String> {
    run_command("pw-link", args)
}

pub struct PipeWireManager {
    handle: std::thread::JoinHandle<()>,
}

impl PipeWireManager {
    pub fn spawn(command_rx: Receiver<CoreCommand>, event_tx: Sender<CoreEvent>) -> Self {
        let handle = std::thread::spawn(move || {
            let _ = event_tx.send(CoreEvent::Ready);
            let routing_mode =
                routing_mode_from_flag(std::env::var(FORCE_LINK_ROUTING_ENV).ok().as_deref());
            let paths = Paths::resolve();
            if let Err(err) = ensure_dirs(&paths) {
                let _ = event_tx.send(CoreEvent::Error(format!(
                    "failed to prepare config/state directories: {err}"
                )));
            }
            let mut runtime_config = load_config(&paths);
            let hotkey_bindings = HotkeyBindings::from(&runtime_config.hotkeys);
            let mut hotkey_state = HotkeyState {
                main_muted: false,
                mic_muted: false,
            };
            let mut hotkey_adapter =
                build_adapter(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), false);
            let _ = hotkey_adapter.register(&hotkey_bindings);
            let mut last_snapshot = Snapshot::default();
            let mut overrides = deserialize_overrides(&runtime_config.categorizer.overrides);
            let mut selected_input = if runtime_config
                .audio
                .input_device
                .eq_ignore_ascii_case("default")
            {
                Some(fallback_to_default_device().to_string())
            } else {
                Some(runtime_config.audio.input_device.clone())
            };
            let mut output_loopback_module: Option<String> = None;
            let mut virtual_mic_module: Option<String> = None;
            let mut last_sink_volume_by_target: BTreeMap<String, f32> = BTreeMap::new();
            let mut last_source_volume_by_target: BTreeMap<String, f32> = BTreeMap::new();
            let mut last_sink_mute_by_target: BTreeMap<String, bool> = BTreeMap::new();
            let mut last_source_mute_by_target: BTreeMap<String, bool> = BTreeMap::new();

            if let Err(err) = ensure_virtual_devices() {
                let _ = event_tx.send(CoreEvent::Error(format!(
                    "failed to create virtual devices: {err}"
                )));
            }

            if let Ok(Some(source_name)) = resolve_selected_input_name(selected_input.as_deref()) {
                match rewire_virtual_mic_source(&source_name) {
                    Ok(module_id) => virtual_mic_module = Some(module_id),
                    Err(err) => {
                        let _ = event_tx.send(CoreEvent::Error(format!(
                            "failed to route virtual mic from {source_name}: {err}"
                        )));
                    }
                }
            }

            loop {
                match command_rx.recv_timeout(POLL_INTERVAL) {
                    Ok(command) => match command {
                        CoreCommand::Ping => {
                            let _ = event_tx.send(CoreEvent::Pong);
                        }
                        CoreCommand::SetVolume(channel, volume) => {
                            apply_channel_volume(
                                channel,
                                volume,
                                selected_input.as_deref(),
                                &last_snapshot,
                                &overrides,
                                &mut last_sink_volume_by_target,
                                &mut last_source_volume_by_target,
                            );
                        }
                        CoreCommand::SetMute(channel, muted) => {
                            if channel == crate::core::messages::Channel::Main {
                                hotkey_state.main_muted = muted;
                            }
                            if channel == crate::core::messages::Channel::Mic {
                                hotkey_state.mic_muted = muted;
                            }

                            apply_channel_mute(
                                channel,
                                muted,
                                selected_input.as_deref(),
                                &last_snapshot,
                                &overrides,
                                &mut last_sink_mute_by_target,
                                &mut last_source_mute_by_target,
                            );
                        }
                        CoreCommand::MoveStream { stream_id, channel } => {
                            if let Some(stream) = last_snapshot.streams.get(&stream_id) {
                                overrides.insert(stream.app_key.clone(), channel);
                                runtime_config.categorizer.overrides =
                                    serialize_overrides(&overrides);
                                if let Err(err) = save_config(&paths, &runtime_config) {
                                    let _ = event_tx.send(CoreEvent::Error(format!(
                                        "failed to persist categorizer override: {err}"
                                    )));
                                }
                            }

                            let route_result = match routing_mode {
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

                            if let Err(err) = route_result {
                                let _ = event_tx.send(CoreEvent::Error(format!(
                                    "failed to move stream {stream_id}: {err}"
                                )));
                            }
                        }
                        CoreCommand::SetOutputDevice(device) => {
                            if let Some(prev_module) = output_loopback_module.take()
                                && let Err(err) = unload_pactl_module(&prev_module)
                            {
                                let _ = event_tx.send(CoreEvent::Error(format!(
                                    "failed to unload monitor loopback module {prev_module}: {err}"
                                )));
                            }
                            if device != fallback_to_default_device() {
                                match load_monitor_loopback_module(&device) {
                                    Ok(module_id) => {
                                        output_loopback_module = Some(module_id);
                                    }
                                    Err(err) => {
                                        let _ = event_tx.send(CoreEvent::Error(format!(
                                            "failed to route Venturi main mix to {device}: {err}"
                                        )));
                                    }
                                }
                            }
                        }
                        CoreCommand::SetInputDevice(device) => {
                            if selected_input.as_deref() != Some(device.as_str()) {
                                selected_input = Some(device.clone());
                                match resolve_selected_input_name(selected_input.as_deref()) {
                                    Ok(Some(source_name)) => {
                                        if let Some(prev_module) = virtual_mic_module.take()
                                            && let Err(err) = unload_pactl_module(&prev_module)
                                        {
                                            let _ = event_tx.send(CoreEvent::Error(format!(
                                                "failed to unload virtual mic module {prev_module}: {err}"
                                            )));
                                        }
                                        match rewire_virtual_mic_source(&source_name) {
                                            Ok(module_id) => {
                                                virtual_mic_module = Some(module_id);
                                            }
                                            Err(err) => {
                                                let _ = event_tx.send(CoreEvent::Error(format!(
                                                    "failed to route virtual mic from {source_name}: {err}"
                                                )));
                                            }
                                        }
                                    }
                                    Ok(None) => {}
                                    Err(err) => {
                                        let _ = event_tx.send(CoreEvent::Error(format!(
                                            "failed to resolve selected input source: {err}"
                                        )));
                                    }
                                }
                            }
                        }
                        CoreCommand::ToggleWindow => {
                            let _ = event_tx.send(CoreEvent::ToggleWindowRequested);
                        }
                        CoreCommand::Shutdown => break,
                        CoreCommand::PlaySound(_) | CoreCommand::StopSound(_) => {}
                    },
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }

                let hotkey_commands =
                    collect_adapter_commands(&mut *hotkey_adapter, &hotkey_bindings, hotkey_state);
                for command in hotkey_commands {
                    match command {
                        CoreCommand::SetMute(channel, muted) => {
                            if channel == crate::core::messages::Channel::Main {
                                hotkey_state.main_muted = muted;
                            }
                            if channel == crate::core::messages::Channel::Mic {
                                hotkey_state.mic_muted = muted;
                            }
                            apply_channel_mute(
                                channel,
                                muted,
                                selected_input.as_deref(),
                                &last_snapshot,
                                &overrides,
                                &mut last_sink_mute_by_target,
                                &mut last_source_mute_by_target,
                            );
                        }
                        CoreCommand::ToggleWindow => {
                            let _ = event_tx.send(CoreEvent::ToggleWindowRequested);
                        }
                        _ => {}
                    }
                }

                match poll_snapshot() {
                    Ok(snapshot) => {
                        if snapshot.devices != last_snapshot.devices {
                            let _ =
                                event_tx.send(CoreEvent::DevicesChanged(snapshot.devices.clone()));
                        }

                        for (id, stream) in &snapshot.streams {
                            if !last_snapshot.streams.contains_key(id) {
                                let category = classify_with_priority(
                                    &overrides,
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

                        for id in last_snapshot.streams.keys() {
                            if !snapshot.streams.contains_key(id) {
                                let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
                            }
                        }

                        last_snapshot = snapshot;
                    }
                    Err(err) => {
                        let _ = event_tx.send(CoreEvent::Error(err));
                    }
                }
            }
        });
        Self { handle }
    }

    pub fn join(self) -> std::thread::Result<()> {
        self.handle.join()
    }
}

#[cfg(test)]
mod tests {
    use super::parse_pw_dump;

    #[test]
    fn parse_pw_dump_detects_sink_source_variants() {
        let raw = r#"[
          {"id": 12, "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa_output.pci"}}},
          {"id": 13, "info": {"props": {"media.class": "Audio/Source/Virtual", "node.name": "venturi_virtual_mic"}}}
        ]"#;

        let snapshot = parse_pw_dump(raw).expect("parse snapshot");
        assert!(snapshot.devices.iter().any(|d| d == "out:alsa_output.pci"));
        assert!(
            snapshot
                .devices
                .iter()
                .any(|d| d == "in:venturi_virtual_mic")
        );
    }

    #[test]
    fn parse_pw_dump_ignores_monitor_sources() {
        let raw = r#"[
          {"id": 21, "info": {"props": {"media.class": "Audio/Source", "node.name": "Venturi-Output.monitor"}}},
          {"id": 22, "info": {"props": {"media.class": "Audio/Source", "node.name": "Venturi-VirtualMic"}}}
        ]"#;

        let snapshot = parse_pw_dump(raw).expect("parse snapshot");
        assert!(!snapshot.devices.iter().any(|d| d.contains(".monitor")));
        assert!(
            !snapshot
                .devices
                .iter()
                .any(|d| d == "in:Venturi-VirtualMic")
        );
        assert!(snapshot.input_ids.contains_key("Venturi-VirtualMic"));
    }

    #[test]
    fn parse_pw_dump_prefers_process_binary_for_display_name() {
        let raw = r#"[
          {
            "id": 44,
            "info": {
              "props": {
                "media.class": "Stream/Output/Audio",
                "application.name": "WEBRTC VoiceEngine",
                "application.process.binary": "discord"
              }
            }
          }
        ]"#;

        let snapshot = parse_pw_dump(raw).expect("parse snapshot");
        let stream = snapshot.streams.get(&44).expect("stream exists");
        assert_eq!(stream.display_name, "discord");
        assert_eq!(stream.app_key, "discord");
    }
}
