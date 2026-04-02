use crate::Channel;
use crate::StableDeviceId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    pub stream: StableDeviceId,
    pub target: StableDeviceId,
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
