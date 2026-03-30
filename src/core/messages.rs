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
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    Ready,
    Pong,
    StreamAppeared {
        id: u32,
        name: String,
        category: Channel,
    },
    StreamRemoved(u32),
    LevelsUpdate(Vec<(Channel, (f32, f32))>),
    DevicesChanged(Vec<String>),
    Error(String),
}
