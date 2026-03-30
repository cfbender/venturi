use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    pub general: General,
    pub audio: Audio,
    pub mic_processing: MicProcessing,
    pub categorizer: Categorizer,
    pub hotkeys: Hotkeys,
    pub soundboard: Soundboard,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct General {
    pub version: u32,
    pub start_minimized: bool,
    pub show_tray_icon: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Audio {
    pub output_device: String,
    pub input_device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MicProcessing {
    pub noise_gate_enabled: bool,
    pub noise_gate_threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Categorizer {
    pub overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hotkeys {
    pub mute_main: String,
    pub mute_mic: String,
    pub push_to_talk: String,
    pub toggle_window: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Soundboard {
    pub pads: Vec<SoundPad>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SoundPad {
    pub name: String,
    pub file: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub volumes: Volumes,
    pub muted: Muted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Volumes {
    pub main: f32,
    pub game: f32,
    pub media: f32,
    pub chat: f32,
    pub aux: f32,
    pub mic: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Muted {
    pub main: bool,
    pub game: bool,
    pub media: bool,
    pub chat: bool,
    pub aux: bool,
    pub mic: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General {
                version: 1,
                start_minimized: false,
                show_tray_icon: true,
            },
            audio: Audio {
                output_device: "default".to_string(),
                input_device: "default".to_string(),
            },
            mic_processing: MicProcessing {
                noise_gate_enabled: true,
                noise_gate_threshold: -40.0,
            },
            categorizer: Categorizer {
                overrides: BTreeMap::new(),
            },
            hotkeys: Hotkeys {
                mute_main: "ctrl+shift+m".to_string(),
                mute_mic: "ctrl+shift+n".to_string(),
                push_to_talk: String::new(),
                toggle_window: "ctrl+shift+v".to_string(),
            },
            soundboard: Soundboard { pads: Vec::new() },
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            volumes: Volumes {
                main: 1.0,
                game: 1.0,
                media: 1.0,
                chat: 1.0,
                aux: 1.0,
                mic: 1.0,
            },
            muted: Muted {
                main: false,
                game: false,
                media: false,
                chat: false,
                aux: false,
                mic: false,
            },
        }
    }
}
