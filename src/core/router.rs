use std::collections::BTreeMap;

use crate::core::messages::Channel;

fn build_metadata_args(stream_id: u32, channel: Channel, target_key: &str) -> Vec<String> {
    vec![
        stream_id.to_string(),
        target_key.to_string(),
        channel_node_name(channel).to_string(),
    ]
}

pub fn build_metadata_target_args(stream_id: u32, channel: Channel) -> Vec<String> {
    build_metadata_args(stream_id, channel, "target.object")
}

pub fn build_metadata_legacy_target_args(stream_id: u32, channel: Channel) -> Vec<String> {
    build_metadata_args(stream_id, channel, "target.node")
}

pub fn channel_node_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Main => "Venturi-Output",
        Channel::Game => "Venturi-Game",
        Channel::Media => "Venturi-Media",
        Channel::Chat => "Venturi-Chat",
        Channel::Aux => "Venturi-Aux",
        Channel::Mic => "Venturi-Mic",
    }
}

pub fn category_mix_output_node_name(channel: Channel) -> Option<&'static str> {
    match channel {
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            Some(channel_node_name(channel))
        }
        Channel::Main | Channel::Mic => None,
    }
}

pub fn resolve_output_target(
    selected_output: Option<&str>,
    output_ids: &BTreeMap<String, u32>,
) -> String {
    let is_default = selected_output
        .map(|s| s.eq_ignore_ascii_case("default"))
        .unwrap_or(true);

    if is_default {
        return "@DEFAULT_AUDIO_SINK@".to_string();
    }

    output_ids
        .get(selected_output.unwrap())
        .map_or_else(|| "@DEFAULT_AUDIO_SINK@".to_string(), |id| id.to_string())
}

pub fn resolve_input_target(
    selected_input: Option<&str>,
    input_ids: &BTreeMap<String, u32>,
) -> String {
    let is_default = selected_input
        .map(|s| s.eq_ignore_ascii_case("default"))
        .unwrap_or(true);

    if is_default {
        return "@DEFAULT_AUDIO_SOURCE@".to_string();
    }

    input_ids
        .get(selected_input.unwrap())
        .map_or_else(|| "@DEFAULT_AUDIO_SOURCE@".to_string(), |id| id.to_string())
}
