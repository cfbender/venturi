#[derive(Debug, Clone)]
pub struct SettingsTab {
    pub noise_gate_enabled: bool,
    pub noise_gate_threshold_db: f32,
    pub mute_main_hotkey: String,
    pub mute_mic_hotkey: String,
    pub push_to_talk_hotkey: String,
    pub toggle_window_hotkey: String,
    pub config_path_label: String,
    pub about_label: String,
}

impl SettingsTab {
    pub fn new(config_path: String, runtime_versions: String) -> Self {
        Self {
            noise_gate_enabled: true,
            noise_gate_threshold_db: -40.0,
            mute_main_hotkey: "Ctrl+Shift+M".to_string(),
            mute_mic_hotkey: "Ctrl+Shift+N".to_string(),
            push_to_talk_hotkey: String::new(),
            toggle_window_hotkey: "Ctrl+Shift+V".to_string(),
            config_path_label: config_path,
            about_label: runtime_versions,
        }
    }
}
