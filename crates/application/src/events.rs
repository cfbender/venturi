use crate::{Channel, StableDeviceId};

#[derive(Debug, Clone, PartialEq)]
pub struct MeterSnapshot {
    pub channel: Channel,
    pub level: f32,
    pub peak: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteCommand {
    Connect {
        source: StableDeviceId,
        target: StableDeviceId,
    },
    Disconnect {
        source: StableDeviceId,
        target: StableDeviceId,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    MeterUpdated(MeterSnapshot),
    RouteRequested(RouteCommand),
    SessionChannelChanged(Channel),
}
