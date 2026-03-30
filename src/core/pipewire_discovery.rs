use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamInfo {
    pub id: u32,
    pub app_key: String,
    pub display_name: String,
    pub media_role: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub devices: Vec<String>,
    pub output_ids: BTreeMap<String, u32>,
    pub input_ids: BTreeMap<String, u32>,
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
            if !hidden_outputs.contains(&node_name) {
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
            if hidden_inputs.contains(&node_name) {
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
            let display_name = preferred_display_name(app_name, binary);
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

fn preferred_display_name<'a>(app_name: &'a str, binary: &'a str) -> &'a str {
    if !binary.is_empty()
        && !binary.eq_ignore_ascii_case("webrtc")
        && !binary.eq_ignore_ascii_case("voiceengine")
        && !binary.eq_ignore_ascii_case("webrtc voiceengine")
    {
        binary
    } else {
        app_name
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

        let snapshot = parse_pw_dump(raw, &[], &[]).expect("parse snapshot");
        assert!(snapshot.devices.iter().any(|d| d == "out:alsa_output.pci"));
        assert!(
            snapshot
                .devices
                .iter()
                .any(|d| d == "in:venturi_virtual_mic")
        );
    }

    #[test]
    fn parse_pw_dump_ignores_monitor_sources_and_hidden_input() {
        let raw = r#"[
          {"id": 21, "info": {"props": {"media.class": "Audio/Source", "node.name": "Venturi-Output.monitor"}}},
          {"id": 22, "info": {"props": {"media.class": "Audio/Source", "node.name": "Venturi-VirtualMic"}}}
        ]"#;

        let snapshot = parse_pw_dump(raw, &[], &["Venturi-VirtualMic"]).expect("parse snapshot");
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

        let snapshot = parse_pw_dump(raw, &[], &[]).expect("parse snapshot");
        let stream = snapshot.streams.get(&44).expect("stream exists");
        assert_eq!(stream.display_name, "discord");
        assert_eq!(stream.app_key, "discord");
    }
}
