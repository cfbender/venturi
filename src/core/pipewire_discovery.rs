use serde_json::Value;
use std::collections::BTreeMap;
use std::process::Command;

use crate::core::messages::{DeviceEntry, DeviceKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamInfo {
    pub id: u32,
    pub meter_target: u32,
    pub app_key: String,
    pub display_name: String,
    pub media_role: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub devices: Vec<DeviceEntry>,
    pub output_ids: BTreeMap<String, u32>,
    pub input_ids: BTreeMap<String, u32>,
    pub output_meter_targets: BTreeMap<String, u32>,
    pub input_meter_targets: BTreeMap<String, u32>,
    pub streams: BTreeMap<u32, StreamInfo>,
}

pub(crate) fn poll_snapshot(
    hidden_outputs: &[&str],
    hidden_inputs: &[&str],
) -> Result<Snapshot, String> {
    let output = Command::new("pw-dump")
        .output()
        .map_err(|e| format!("failed to run pw-dump: {e}"))?;
    if !output.status.success() {
        return Err(format!("pw-dump exited with {}", output.status));
    }
    let raw = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    parse_pw_dump(&raw, hidden_outputs, hidden_inputs)
}

pub(crate) fn parse_pw_dump(
    raw: &str,
    hidden_outputs: &[&str],
    hidden_inputs: &[&str],
) -> Result<Snapshot, String> {
    let value: Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let arr = value
        .as_array()
        .ok_or_else(|| "pw-dump root is not array".to_string())?;

    let mut outputs = BTreeMap::new();
    let mut inputs = BTreeMap::new();
    let mut output_ids = BTreeMap::new();
    let mut input_ids = BTreeMap::new();
    let mut output_meter_targets = BTreeMap::new();
    let mut input_meter_targets = BTreeMap::new();
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

        let meter_target = parse_object_serial(props).or(id);

        let media_class = props
            .get("media.class")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let node_name = props
            .get("node.name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let stream_node_name = props
            .get("node.name")
            .and_then(Value::as_str)
            .or_else(|| props.get("node.nick").and_then(Value::as_str))
            .or_else(|| props.get("node.description").and_then(Value::as_str))
            .unwrap_or_default();

        if media_class.contains("Sink") && !node_name.is_empty() {
            if is_loopback_name(node_name) {
                continue;
            }
            if !hidden_outputs.contains(&node_name) {
                outputs.insert(
                    node_name.to_string(),
                    DeviceEntry {
                        kind: DeviceKind::Output,
                        id: node_name.to_string(),
                        label: preferred_device_label(props, node_name),
                    },
                );
            }
            if let Some(node_id) = id {
                output_ids.insert(node_name.to_string(), node_id);
            }
            if let Some(target_id) = meter_target {
                output_meter_targets.insert(node_name.to_string(), target_id);
            }
        }

        if media_class.contains("Source") && !node_name.is_empty() {
            if node_name.ends_with(".monitor") {
                continue;
            }
            if is_loopback_name(node_name) {
                continue;
            }
            if let Some(node_id) = id {
                input_ids.insert(node_name.to_string(), node_id);
            }
            if let Some(target_id) = meter_target {
                input_meter_targets.insert(node_name.to_string(), target_id);
            }
            if hidden_inputs.contains(&node_name) {
                continue;
            }
            inputs.insert(
                node_name.to_string(),
                DeviceEntry {
                    kind: DeviceKind::Input,
                    id: node_name.to_string(),
                    label: preferred_device_label(props, node_name),
                },
            );
        }

        if media_class == "Stream/Output/Audio" || media_class == "Audio/Stream/Output" {
            let Some(stream_id) = id else {
                continue;
            };
            let app_name = props
                .get("application.name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let binary = props
                .get("application.process.binary")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let media_name = props
                .get("media.name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if is_loopback_stream(app_name, binary, media_name, node_name) {
                continue;
            }
            let display_name =
                preferred_display_name(app_name, binary, media_name, stream_node_name);
            let role = props
                .get("media.role")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);

            streams.insert(
                stream_id,
                StreamInfo {
                    id: stream_id,
                    meter_target: meter_target.unwrap_or(stream_id),
                    app_key: if binary.is_empty() {
                        display_name.to_ascii_lowercase()
                    } else {
                        binary.to_ascii_lowercase()
                    },
                    display_name,
                    media_role: role,
                },
            );
        }
    }

    let mut devices = Vec::with_capacity(outputs.len() + inputs.len());
    devices.extend(outputs.into_values());
    devices.extend(inputs.into_values());

    Ok(Snapshot {
        devices,
        output_ids,
        input_ids,
        output_meter_targets,
        input_meter_targets,
        streams,
    })
}

fn preferred_display_name(
    app_name: &str,
    binary: &str,
    media_name: &str,
    node_name: &str,
) -> String {
    if !binary.is_empty() && !is_generic_name(binary) {
        return prettify_binary(binary);
    }
    if !app_name.is_empty() && !is_generic_name(app_name) {
        return app_name.to_string();
    }
    if !media_name.is_empty() && !is_generic_name(media_name) {
        return media_name.to_string();
    }
    if !node_name.is_empty() {
        return prettify_node_name(node_name);
    }
    "Unknown App".to_string()
}

fn parse_object_serial(props: &serde_json::Map<String, Value>) -> Option<u32> {
    let value = props.get("object.serial")?;
    value
        .as_u64()
        .and_then(|raw| u32::try_from(raw).ok())
        .or_else(|| value.as_str().and_then(|raw| raw.parse::<u32>().ok()))
}

fn is_generic_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.is_empty()
        || lower == "unknown app"
        || lower.contains("webrtc voiceengine")
        || lower == "voiceengine"
        || lower == "webrtc"
}

fn is_loopback_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("loopback")
}

fn is_loopback_stream(app_name: &str, binary: &str, media_name: &str, node_name: &str) -> bool {
    let haystack = format!("{app_name} {binary} {media_name} {node_name}").to_ascii_lowercase();
    haystack.contains("loopback")
}

fn prettify_binary(binary: &str) -> String {
    let base = binary.rsplit('/').next().unwrap_or(binary);
    let normalized = base.replace(['_', '-', '.'], " ");
    normalized
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    format!(
                        "{}{}",
                        first.to_ascii_uppercase(),
                        chars.as_str().to_ascii_lowercase()
                    )
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn prettify_node_name(node_name: &str) -> String {
    node_name
        .replace("alsa_output.", "")
        .replace("alsa_input.", "")
        .replace(".analog-stereo", "")
        .replace(".monitor", "")
        .replace('_', " ")
}

fn preferred_device_label(props: &serde_json::Map<String, Value>, node_name: &str) -> String {
    props
        .get("node.description")
        .and_then(Value::as_str)
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            props
                .get("device.description")
                .and_then(Value::as_str)
                .filter(|label| !label.trim().is_empty())
        })
        .or_else(|| {
            props
                .get("node.nick")
                .and_then(Value::as_str)
                .filter(|label| !label.trim().is_empty())
        })
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| prettify_node_name(node_name))
}

#[cfg(test)]
mod tests {
    use super::parse_pw_dump;
    use crate::core::messages::DeviceKind;

    #[test]
    fn parse_pw_dump_detects_sink_source_variants() {
        let empty: [&'static str; 0] = [];
        let raw = r#"[
          {"id": 12, "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa_output.pci"}}},
          {"id": 13, "info": {"props": {"media.class": "Audio/Source/Virtual", "node.name": "venturi_virtual_mic"}}}
        ]"#;

        let snapshot =
            parse_pw_dump(raw, empty.as_slice(), empty.as_slice()).expect("parse snapshot");
        assert!(snapshot
            .devices
            .iter()
            .any(|d| d.kind == DeviceKind::Output && d.id == "alsa_output.pci"));
        assert!(snapshot
            .devices
            .iter()
            .any(|d| d.kind == DeviceKind::Input && d.id == "venturi_virtual_mic"));
    }

    #[test]
    fn parse_pw_dump_ignores_monitor_sources_and_hidden_input() {
        let empty: [&'static str; 0] = [];
        let hidden: [&'static str; 1] = ["Venturi-VirtualMic"];
        let raw = r#"[
          {"id": 21, "info": {"props": {"media.class": "Audio/Source", "node.name": "Venturi-Output.monitor"}}},
          {"id": 22, "info": {"props": {"media.class": "Audio/Source", "node.name": "Venturi-VirtualMic"}}}
        ]"#;

        let snapshot =
            parse_pw_dump(raw, empty.as_slice(), hidden.as_slice()).expect("parse snapshot");
        assert!(!snapshot.devices.iter().any(|d| d.id.contains(".monitor")));
        assert!(!snapshot
            .devices
            .iter()
            .any(|d| d.kind == DeviceKind::Input && d.id == "Venturi-VirtualMic"));
        assert!(snapshot.input_ids.contains_key("Venturi-VirtualMic"));
    }

    #[test]
    fn parse_pw_dump_prefers_process_binary_for_display_name() {
        let empty: [&'static str; 0] = [];
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

        let snapshot =
            parse_pw_dump(raw, empty.as_slice(), empty.as_slice()).expect("parse snapshot");
        let stream = snapshot.streams.get(&44).expect("stream exists");
        assert_eq!(stream.display_name, "Discord");
        assert_eq!(stream.app_key, "discord");
    }

    #[test]
    fn parse_pw_dump_filters_loopback_entries() {
        let empty: [&'static str; 0] = [];
        let raw = r#"[
          {"id": 10, "info": {"props": {"media.class": "Audio/Sink", "node.name": "loopback_output.test"}}},
          {"id": 11, "info": {"props": {"media.class": "Audio/Source", "node.name": "loopback_input.test"}}},
          {"id": 12, "info": {"props": {"media.class": "Audio/Sink", "node.name": "alsa_output.real"}}},
          {
            "id": 50,
            "info": {
              "props": {
                "media.class": "Stream/Output/Audio",
                "application.name": "Loopback",
                "application.process.binary": "pw-loopback"
              }
            }
          }
        ]"#;

        let snapshot =
            parse_pw_dump(raw, empty.as_slice(), empty.as_slice()).expect("parse snapshot");
        assert!(snapshot
            .devices
            .iter()
            .any(|d| d.kind == DeviceKind::Output && d.id == "alsa_output.real"));
        assert!(!snapshot.devices.iter().any(|d| d.id.contains("loopback")));
        assert!(snapshot.streams.is_empty());
    }

    #[test]
    fn parse_pw_dump_prefers_descriptive_device_labels() {
        let empty: [&'static str; 0] = [];
        let raw = r#"[
          {
            "id": 91,
            "info": {
              "props": {
                "media.class": "Audio/Sink",
                "node.name": "alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1",
                "node.description": "Navi 48 HDMI/DP Audio Controller Digital Stereo (HDMI 2)",
                "device.description": "Fallback Device Label"
              }
            }
          }
        ]"#;

        let snapshot =
            parse_pw_dump(raw, empty.as_slice(), empty.as_slice()).expect("parse snapshot");
        let output = snapshot
            .devices
            .iter()
            .find(|d| d.kind == DeviceKind::Output)
            .expect("output device exists");
        assert_eq!(output.id, "alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1");
        assert_eq!(
            output.label,
            "Navi 48 HDMI/DP Audio Controller Digital Stereo (HDMI 2)"
        );
    }

    #[test]
    fn parse_pw_dump_uses_object_serial_for_meter_targets() {
        let empty: [&'static str; 0] = [];
        let raw = r#"[
          {
            "id": 128,
            "info": {
              "props": {
                "media.class": "Audio/Sink",
                "node.name": "Venturi-Output",
                "object.serial": "37284"
              }
            }
          },
          {
            "id": 197,
            "info": {
              "props": {
                "media.class": "Stream/Output/Audio",
                "object.serial": "42330",
                "application.name": "paplay",
                "application.process.binary": "paplay"
              }
            }
          }
        ]"#;

        let snapshot =
            parse_pw_dump(raw, empty.as_slice(), empty.as_slice()).expect("parse snapshot");

        assert_eq!(snapshot.output_ids.get("Venturi-Output"), Some(&128));
        assert_eq!(
            snapshot.output_meter_targets.get("Venturi-Output"),
            Some(&37284)
        );
        let stream = snapshot.streams.get(&197).expect("stream exists");
        assert_eq!(stream.meter_target, 42330);
    }
}
