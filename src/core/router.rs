use std::collections::BTreeMap;

use crate::core::messages::Channel;
use venturi_domain::Channel as DomainChannel;

pub const FORCE_LINK_ROUTING_ENV: &str = "VENTURI_FORCE_LINK_ROUTING";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingMode {
    MetadataFirst,
    FallbackLinks,
}

pub fn choose_routing_mode(force_links: bool) -> RoutingMode {
    if force_links {
        RoutingMode::FallbackLinks
    } else {
        RoutingMode::MetadataFirst
    }
}

pub fn force_link_routing_enabled(raw: Option<&str>) -> bool {
    raw.is_some_and(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
}

pub fn routing_mode_from_flag(raw: Option<&str>) -> RoutingMode {
    choose_routing_mode(force_link_routing_enabled(raw))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRoutePlan {
    pub output_target: String,
    pub input_target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataRoutePlan {
    pub stream_id: u32,
    pub target_node: String,
}

pub fn build_metadata_route_plan(stream_id: u32, channel: Channel) -> MetadataRoutePlan {
    MetadataRoutePlan {
        stream_id,
        target_node: channel_node_name(channel).to_string(),
    }
}

pub fn build_metadata_target_args(stream_id: u32, channel: Channel) -> Vec<String> {
    let plan = build_metadata_route_plan(stream_id, channel);
    vec![
        "-n".to_string(),
        "settings".to_string(),
        plan.stream_id.to_string(),
        "target.node".to_string(),
        plan.target_node,
    ]
}

pub fn build_fallback_link_commands(stream_id: u32, channel: Channel) -> Vec<Vec<String>> {
    let node = channel_node_name(channel);
    vec![
        vec![
            "--passive".to_string(),
            format!("{stream_id}:output_FL"),
            format!("{node}:input_FL"),
        ],
        vec![
            "--passive".to_string(),
            format!("{stream_id}:output_FR"),
            format!("{node}:input_FR"),
        ],
    ]
}

pub fn channel_node_name(channel: Channel) -> &'static str {
    venturi_domain::channel_node_name(domain_channel(channel))
}

fn domain_channel(channel: Channel) -> DomainChannel {
    match channel {
        Channel::Main => DomainChannel::Main,
        Channel::Game => DomainChannel::Game,
        Channel::Media => DomainChannel::Media,
        Channel::Chat => DomainChannel::Chat,
        Channel::Aux => DomainChannel::Aux,
        Channel::Mic => DomainChannel::Mic,
    }
}

pub fn build_device_route_plan(output: Option<&str>, input: Option<&str>) -> DeviceRoutePlan {
    DeviceRoutePlan {
        output_target: output.unwrap_or("default").to_string(),
        input_target: input.unwrap_or("default").to_string(),
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
