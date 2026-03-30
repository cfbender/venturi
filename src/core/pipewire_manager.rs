use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Duration;

use crate::categorizer::learning::{deserialize_overrides, serialize_overrides};
use crate::categorizer::rules::classify_with_priority;
use crate::config::persistence::{Paths, ensure_dirs, load_config, save_config};
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
            .get("node.description")
            .and_then(Value::as_str)
            .or_else(|| props.get("node.nick").and_then(Value::as_str))
            .or_else(|| props.get("node.name").and_then(Value::as_str))
            .unwrap_or_default();

        if media_class.starts_with("Audio/Sink") && !node_name.is_empty() {
            outputs.insert(format!("out:{node_name}"));
            if let Some(node_id) = id {
                output_ids.insert(node_name.to_string(), node_id);
            }
        }
        if media_class.starts_with("Audio/Source") && !node_name.is_empty() {
            inputs.insert(format!("in:{node_name}"));
            if let Some(node_id) = id {
                input_ids.insert(node_name.to_string(), node_id);
            }
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
                .unwrap_or(app_name);
            let role = props
                .get("media.role")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            streams.insert(
                stream_id,
                StreamInfo {
                    id: stream_id,
                    app_key: binary.to_ascii_lowercase(),
                    display_name: app_name.to_string(),
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
            let mut last_snapshot = Snapshot::default();
            let mut overrides = deserialize_overrides(&runtime_config.categorizer.overrides);
            let mut selected_output = Some("Default".to_string());
            let mut selected_input = Some("Default".to_string());
            let mut last_sink_volume_by_target: BTreeMap<String, f32> = BTreeMap::new();
            let mut last_source_volume_by_target: BTreeMap<String, f32> = BTreeMap::new();
            let mut last_sink_mute_by_target: BTreeMap<String, bool> = BTreeMap::new();
            let mut last_source_mute_by_target: BTreeMap<String, bool> = BTreeMap::new();

            if let Ok(snapshot) = poll_snapshot() {
                if snapshot.devices != last_snapshot.devices {
                    let _ = event_tx.send(CoreEvent::DevicesChanged(snapshot.devices.clone()));
                }
                for (id, stream) in &snapshot.streams {
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
                last_snapshot = snapshot;
            }

            loop {
                match command_rx.recv_timeout(POLL_INTERVAL) {
                    Ok(command) => match command {
                        CoreCommand::Ping => {
                            let _ = event_tx.send(CoreEvent::Pong);
                        }
                        CoreCommand::SetVolume(channel, volume) => {
                            if channel == crate::core::messages::Channel::Mic {
                                let target = resolve_input_target(
                                    selected_input.as_deref(),
                                    &last_snapshot.input_ids,
                                );
                                let changed = last_source_volume_by_target
                                    .get(&target)
                                    .map(|prev| (*prev - volume).abs() >= 0.01)
                                    .unwrap_or(true);
                                if changed {
                                    let args = vec![
                                        "set-volume".to_string(),
                                        target.clone(),
                                        volume.to_string(),
                                    ];
                                    run_wpctl(&args);
                                    last_source_volume_by_target.insert(target, volume);
                                }
                            } else {
                                let target = resolve_output_target(
                                    selected_output.as_deref(),
                                    &last_snapshot.output_ids,
                                );
                                let changed = last_sink_volume_by_target
                                    .get(&target)
                                    .map(|prev| (*prev - volume).abs() >= 0.01)
                                    .unwrap_or(true);
                                if changed {
                                    let args = vec![
                                        "set-volume".to_string(),
                                        target.clone(),
                                        volume.to_string(),
                                    ];
                                    run_wpctl(&args);
                                    last_sink_volume_by_target.insert(target, volume);
                                }
                            }
                        }
                        CoreCommand::SetMute(channel, muted) => {
                            let value = if muted { "1" } else { "0" };
                            if channel == crate::core::messages::Channel::Mic {
                                let target = resolve_input_target(
                                    selected_input.as_deref(),
                                    &last_snapshot.input_ids,
                                );
                                if last_source_mute_by_target.get(&target) != Some(&muted) {
                                    let args = vec![
                                        "set-mute".to_string(),
                                        target.clone(),
                                        value.to_string(),
                                    ];
                                    run_wpctl(&args);
                                    last_source_mute_by_target.insert(target, muted);
                                }
                            } else {
                                let target = resolve_output_target(
                                    selected_output.as_deref(),
                                    &last_snapshot.output_ids,
                                );
                                if last_sink_mute_by_target.get(&target) != Some(&muted) {
                                    let args = vec![
                                        "set-mute".to_string(),
                                        target.clone(),
                                        value.to_string(),
                                    ];
                                    run_wpctl(&args);
                                    last_sink_mute_by_target.insert(target, muted);
                                }
                            }
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
                            selected_output = Some(device);
                            let target = resolve_output_target(
                                selected_output.as_deref(),
                                &last_snapshot.output_ids,
                            );
                            let args = vec!["set-default".to_string(), target];
                            run_wpctl(&args);
                        }
                        CoreCommand::SetInputDevice(device) => {
                            selected_input = Some(device);
                            let target = resolve_input_target(
                                selected_input.as_deref(),
                                &last_snapshot.input_ids,
                            );
                            let args = vec!["set-default".to_string(), target];
                            run_wpctl(&args);
                        }
                        CoreCommand::Shutdown => break,
                        CoreCommand::ToggleWindow
                        | CoreCommand::PlaySound(_)
                        | CoreCommand::StopSound(_) => {}
                    },
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
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
