use std::time::Duration;

use crate::core::messages::CoreEvent;
use crate::gui::mixer_tab::MixerTab;
use crate::gui::settings_tab::SettingsTab;
use crate::gui::soundboard_tab::SoundboardTab;

pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    Mixer,
    Soundboard,
    Settings,
}

#[derive(Debug, Clone)]
pub struct MainWindow {
    pub width: u32,
    pub height: u32,
    pub active_tab: Tab,
    pub mixer: MixerTab,
    pub soundboard: SoundboardTab,
    pub settings: SettingsTab,
    pub reconnect_every: Duration,
}

impl MainWindow {
    pub fn new(config_path: String, runtime_versions: String) -> Self {
        Self {
            width: 900,
            height: 600,
            active_tab: Tab::Mixer,
            mixer: MixerTab::new(),
            soundboard: SoundboardTab::default(),
            settings: SettingsTab::new(config_path, runtime_versions),
            reconnect_every: RECONNECT_INTERVAL,
        }
    }

    pub fn apply_core_event(&mut self, event: &CoreEvent) {
        self.mixer.apply_event(event);
    }

    pub fn on_pipewire_disconnect(&mut self) {
        self.mixer.banner = Some("Connection to PipeWire lost. Reconnecting...".to_string());
    }

    pub fn on_pipewire_not_running(&mut self) {
        self.mixer.banner =
            Some("PipeWire not detected. Start PipeWire and restart Venturi.".to_string());
    }

    pub fn on_config_corrupt(&mut self) {
        self.mixer.toast = Some("Config was reset due to errors.".to_string());
    }
}
