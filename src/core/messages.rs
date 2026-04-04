#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Main,
    Game,
    Media,
    Chat,
    Aux,
    Mic,
}

impl std::str::FromStr for Channel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "main" => Ok(Self::Main),
            "game" => Ok(Self::Game),
            "media" => Ok(Self::Media),
            "chat" => Ok(Self::Chat),
            "aux" => Ok(Self::Aux),
            "mic" => Ok(Self::Mic),
            _ => Err(()),
        }
    }
}

impl Channel {
    /// Returns the CSS class suffix for this channel (e.g., "main", "game", etc.)
    pub const fn css_class(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Game => "game",
            Self::Media => "media",
            Self::Chat => "chat",
            Self::Aux => "aux",
            Self::Mic => "mic",
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
    /// Ask the core to re-send the current snapshot (devices, streams, volumes).
    /// Used after the GUI event loop is running to ensure it receives initial state
    /// that may have been emitted (and dropped) before the UI was ready.
    RequestSnapshot,
    SetMeteringEnabled(bool),
    Shutdown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoreEvent {
    Ready,
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
