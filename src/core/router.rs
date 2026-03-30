use crate::core::messages::Channel;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRoutePlan {
    pub output_target: String,
    pub input_target: String,
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

pub fn build_device_route_plan(output: Option<&str>, input: Option<&str>) -> DeviceRoutePlan {
    DeviceRoutePlan {
        output_target: output.unwrap_or("default").to_string(),
        input_target: input.unwrap_or("default").to_string(),
    }
}
