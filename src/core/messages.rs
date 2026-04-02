#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Main,
    Game,
    Media,
    Chat,
    Aux,
    Mic,
}

impl From<venturi_domain::Channel> for Channel {
    fn from(value: venturi_domain::Channel) -> Self {
        match value {
            venturi_domain::Channel::Main => Self::Main,
            venturi_domain::Channel::Game => Self::Game,
            venturi_domain::Channel::Media => Self::Media,
            venturi_domain::Channel::Chat => Self::Chat,
            venturi_domain::Channel::Aux => Self::Aux,
            venturi_domain::Channel::Mic => Self::Mic,
        }
    }
}

impl From<Channel> for venturi_domain::Channel {
    fn from(value: Channel) -> Self {
        match value {
            Channel::Main => Self::Main,
            Channel::Game => Self::Game,
            Channel::Media => Self::Media,
            Channel::Chat => Self::Chat,
            Channel::Aux => Self::Aux,
            Channel::Mic => Self::Mic,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreCommand {
    SetVolume(Channel, f32),
    SetMute(Channel, bool),
    MoveStream {
        stream_id: u32,
        channel: Channel,
    },
    SetOutputDevice(String),
    SetInputDevice(String),
    ToggleWindow,
    PlaySound {
        pad_id: u32,
        file: String,
    },
    PreviewSound {
        pad_id: u32,
        file: String,
    },
    StopSound(u32),
    Ping,
    /// Ask the core to re-send the current snapshot (devices, streams, volumes).
    /// Used after the GUI event loop is running to ensure it receives initial state
    /// that may have been emitted (and dropped) before the UI was ready.
    RequestSnapshot,
    SetMeteringEnabled(bool),
    Shutdown,
}

impl CoreCommand {
    pub fn typed_toggle_window() -> Self {
        Self::ToggleWindow
    }

    pub fn typed_shutdown() -> Self {
        Self::Shutdown
    }

    pub fn typed_set_volume(channel: venturi_domain::Channel, value: f32) -> Self {
        Self::SetVolume(channel.into(), value)
    }

    pub fn typed_select_output(output: venturi_domain::StableDeviceId) -> Self {
        Self::SetOutputDevice(output.0)
    }

    pub fn typed_select_input(input: venturi_domain::StableDeviceId) -> Self {
        Self::SetInputDevice(input.0)
    }

    pub fn typed_play_sound(pad_id: u32, file: String) -> Self {
        Self::PlaySound { pad_id, file }
    }

    pub fn typed_preview_sound(pad_id: u32, file: String) -> Self {
        Self::PreviewSound { pad_id, file }
    }

    pub fn typed_stop_sound(pad_id: u32) -> Self {
        Self::StopSound(pad_id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    Ready,
    Pong,
    ToggleWindowRequested,
    ShutdownRequested,
    StreamAppeared {
        id: u32,
        app_key: String,
        name: String,
        category: Channel,
    },
    StreamRemoved(u32),
    LevelsUpdate(Vec<(Channel, (f32, f32))>),
    VolumeChanged(Channel, f32),
    DevicesChanged(Vec<DeviceEntry>),
    DeviceSelectionChanged {
        selected_output: Option<String>,
        selected_input: Option<String>,
    },
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
