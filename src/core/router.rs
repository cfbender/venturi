use std::collections::BTreeMap;

use crate::core::messages::Channel;

pub fn build_metadata_target_args(stream_id: u32, channel: Channel) -> Vec<String> {
    vec![
        stream_id.to_string(),
        "target.object".to_string(),
        channel_node_name(channel).to_string(),
    ]
}

pub fn build_metadata_legacy_target_args(stream_id: u32, channel: Channel) -> Vec<String> {
    vec![
        stream_id.to_string(),
        "target.node".to_string(),
        channel_node_name(channel).to_string(),
    ]
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

pub fn resolve_output_target(
    selected_output: Option<&str>,
    output_ids: &BTreeMap<String, u32>,
) -> String {
    let Some(selected) = selected_output else {
        return "@DEFAULT_AUDIO_SINK@".to_string();
    };
    if selected.eq_ignore_ascii_case("default") {
        return "@DEFAULT_AUDIO_SINK@".to_string();
    }

    output_ids
        .get(selected)
        .map(u32::to_string)
        .unwrap_or_else(|| "@DEFAULT_AUDIO_SINK@".to_string())
}

pub fn resolve_input_target(
    selected_input: Option<&str>,
    input_ids: &BTreeMap<String, u32>,
) -> String {
    let Some(selected) = selected_input else {
        return "@DEFAULT_AUDIO_SOURCE@".to_string();
    };
    if selected.eq_ignore_ascii_case("default") {
        return "@DEFAULT_AUDIO_SOURCE@".to_string();
    }

    input_ids
        .get(selected)
        .map(u32::to_string)
        .unwrap_or_else(|| "@DEFAULT_AUDIO_SOURCE@".to_string())
}
