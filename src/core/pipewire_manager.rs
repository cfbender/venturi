use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;
use std::time::Duration;

use crate::categorizer::rules::classify_with_priority;
use crate::core::messages::{CoreCommand, CoreEvent};

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

fn parse_pw_dump(raw: &str) -> Result<(Vec<String>, BTreeMap<u32, StreamInfo>), String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let arr = value
        .as_array()
        .ok_or_else(|| "pw-dump root is not array".to_string())?;

    let mut outputs = BTreeSet::new();
    let mut inputs = BTreeSet::new();
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

        if media_class == "Audio/Sink" && !node_name.is_empty() {
            outputs.insert(format!("out:{node_name}"));
        }
        if media_class == "Audio/Source" && !node_name.is_empty() {
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

    Ok((devices, streams))
}

fn poll_snapshot() -> Result<(Vec<String>, BTreeMap<u32, StreamInfo>), String> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|e| format!("failed to run pw-dump: {e}"))?;
    if !output.status.success() {
        return Err(format!("pw-dump exited with {}", output.status));
    }
    let raw = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_pw_dump(&raw)
}

fn run_wpctl(args: &[&str]) {
    let _ = Command::new("wpctl").args(args).status();
}

pub struct PipeWireManager {
    handle: std::thread::JoinHandle<()>,
}

impl PipeWireManager {
    pub fn spawn(command_rx: Receiver<CoreCommand>, event_tx: Sender<CoreEvent>) -> Self {
        let handle = std::thread::spawn(move || {
            let _ = event_tx.send(CoreEvent::Ready);
            let mut last_devices: Vec<String> = Vec::new();
            let mut last_streams: BTreeMap<u32, StreamInfo> = BTreeMap::new();
            let overrides = BTreeMap::new();
            let mut last_sink_volume: Option<f32> = None;
            let mut last_source_volume: Option<f32> = None;
            let mut last_sink_mute: Option<bool> = None;
            let mut last_source_mute: Option<bool> = None;

            loop {
                match command_rx.recv_timeout(POLL_INTERVAL) {
                    Ok(command) => {
                        match command {
                            CoreCommand::Ping => {
                                let _ = event_tx.send(CoreEvent::Pong);
                            }
                            CoreCommand::SetVolume(channel, volume) => {
                                // Temporary bridge until virtual channel sinks are active.
                                if channel == crate::core::messages::Channel::Mic {
                                    let changed = last_source_volume
                                        .map(|prev| (prev - volume).abs() >= 0.01)
                                        .unwrap_or(true);
                                    if changed {
                                        run_wpctl(&[
                                            "set-volume",
                                            "@DEFAULT_AUDIO_SOURCE@",
                                            &volume.to_string(),
                                        ]);
                                        last_source_volume = Some(volume);
                                    }
                                } else {
                                    let changed = last_sink_volume
                                        .map(|prev| (prev - volume).abs() >= 0.01)
                                        .unwrap_or(true);
                                    if changed {
                                        run_wpctl(&[
                                            "set-volume",
                                            "@DEFAULT_AUDIO_SINK@",
                                            &volume.to_string(),
                                        ]);
                                        last_sink_volume = Some(volume);
                                    }
                                }
                            }
                            CoreCommand::SetMute(channel, muted) => {
                                let value = if muted { "1" } else { "0" };
                                if channel == crate::core::messages::Channel::Mic {
                                    if last_source_mute != Some(muted) {
                                        run_wpctl(&["set-mute", "@DEFAULT_AUDIO_SOURCE@", value]);
                                        last_source_mute = Some(muted);
                                    }
                                } else {
                                    if last_sink_mute != Some(muted) {
                                        run_wpctl(&["set-mute", "@DEFAULT_AUDIO_SINK@", value]);
                                        last_sink_mute = Some(muted);
                                    }
                                }
                            }
                            CoreCommand::Shutdown => break,
                            CoreCommand::MoveStream { .. }
                            | CoreCommand::SetOutputDevice(_)
                            | CoreCommand::SetInputDevice(_)
                            | CoreCommand::PlaySound(_)
                            | CoreCommand::StopSound(_) => {}
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }

                match poll_snapshot() {
                    Ok((devices, streams)) => {
                        if devices != last_devices {
                            let _ = event_tx.send(CoreEvent::DevicesChanged(devices.clone()));
                            last_devices = devices;
                        }

                        for (id, stream) in &streams {
                            if !last_streams.contains_key(id) {
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

                        for id in last_streams.keys() {
                            if !streams.contains_key(id) {
                                let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
                            }
                        }

                        last_streams = streams;
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
