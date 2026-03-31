#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Main,
    Game,
    Media,
    Chat,
    Aux,
    Mic,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreCommand {
    SetVolume(Channel, f32),
    SetMute(Channel, bool),
    MoveStream { stream_id: u32, channel: Channel },
    SetOutputDevice(String),
    SetInputDevice(String),
    ToggleWindow,
    PlaySound(u32),
    StopSound(u32),
    Ping,
    SetMeteringEnabled(bool),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    Ready,
    Pong,
    ToggleWindowRequested,
    StreamAppeared {
        id: u32,
        name: String,
        category: Channel,
    },
    StreamRemoved(u32),
    LevelsUpdate(Vec<(Channel, (f32, f32))>),
    DevicesChanged(Vec<DeviceEntry>),
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKind {
    Output,
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceEntry {
    pub kind: DeviceKind,
    pub id: String,
    pub label: String,
}
