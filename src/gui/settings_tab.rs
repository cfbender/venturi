use std::path::Path;

use crate::config::persistence::{Paths, load_config, save_config};
use crate::config::schema::Hotkeys;

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

    pub fn hotkeys(&self) -> Hotkeys {
        Hotkeys {
            mute_main: self.mute_main_hotkey.clone(),
            mute_mic: self.mute_mic_hotkey.clone(),
            push_to_talk: self.push_to_talk_hotkey.clone(),
            toggle_window: self.toggle_window_hotkey.clone(),
        }
    }
}

pub fn persist_hotkeys_to_config(config_file: &Path, hotkeys: &Hotkeys) -> Result<(), String> {
    let config_dir = config_file
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?
        .to_path_buf();

    std::fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;

    let paths = Paths {
        config_dir,
        state_dir: Paths::resolve().state_dir,
    };

    let mut config = load_config(&paths);
    config.hotkeys = hotkeys.clone();
    save_config(&paths, &config)
}

pub fn build_settings_widget(model: std::sync::Arc<std::sync::Mutex<SettingsTab>>) -> gtk::Box {
    use gtk::prelude::*;

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);

    let noise_group = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let noise_title = gtk::Label::new(Some("Mic Processing"));
    noise_title.add_css_class("title-4");

    let gate_toggle = gtk::Switch::new();
    let gate_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    gate_row.append(&gtk::Label::new(Some("Enable noise gate")));
    gate_row.append(&gate_toggle);

    let threshold = gtk::Scale::with_range(gtk::Orientation::Horizontal, -80.0, 0.0, 1.0);
    let threshold_label = gtk::Label::new(Some("Threshold (dB)"));

    {
        let state = model.lock().expect("settings lock");
        gate_toggle.set_active(state.noise_gate_enabled);
        threshold.set_value(state.noise_gate_threshold_db as f64);
    }

    {
        let model = model.clone();
        gate_toggle.connect_state_set(move |_, active| {
            if let Ok(mut state) = model.lock() {
                state.noise_gate_enabled = active;
            }
            false.into()
        });
    }

    {
        let model = model.clone();
        threshold.connect_value_changed(move |scale| {
            if let Ok(mut state) = model.lock() {
                state.noise_gate_threshold_db = scale.value() as f32;
            }
        });
    }

    noise_group.append(&noise_title);
    noise_group.append(&gate_row);
    noise_group.append(&threshold_label);
    noise_group.append(&threshold);

    let hotkeys_group = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let hotkeys_title = gtk::Label::new(Some("Hotkeys"));
    hotkeys_title.add_css_class("title-4");
    hotkeys_group.append(&hotkeys_title);

    let mute_main_entry = gtk::Entry::new();
    let mute_mic_entry = gtk::Entry::new();
    let ptt_entry = gtk::Entry::new();
    let toggle_window_entry = gtk::Entry::new();

    if let Ok(state) = model.lock() {
        mute_main_entry.set_text(&state.mute_main_hotkey);
        mute_mic_entry.set_text(&state.mute_mic_hotkey);
        ptt_entry.set_text(&state.push_to_talk_hotkey);
        toggle_window_entry.set_text(&state.toggle_window_hotkey);
    }

    for (label, entry) in [
        ("Mute Main", &mute_main_entry),
        ("Mute Mic", &mute_mic_entry),
        ("Push to talk", &ptt_entry),
        ("Toggle Window", &toggle_window_entry),
    ] {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&gtk::Label::new(Some(label)));
        row.append(entry);
        hotkeys_group.append(&row);
    }

    {
        let model = model.clone();
        mute_main_entry.connect_changed(move |entry| {
            let mut persist: Option<(String, Hotkeys)> = None;
            if let Ok(mut state) = model.lock() {
                state.mute_main_hotkey = entry.text().to_string();
                persist = Some((state.config_path_label.clone(), state.hotkeys()));
            }
            if let Some((path, hotkeys)) = persist {
                let _ = persist_hotkeys_to_config(Path::new(&path), &hotkeys);
            }
        });
    }
    {
        let model = model.clone();
        mute_mic_entry.connect_changed(move |entry| {
            let mut persist: Option<(String, Hotkeys)> = None;
            if let Ok(mut state) = model.lock() {
                state.mute_mic_hotkey = entry.text().to_string();
                persist = Some((state.config_path_label.clone(), state.hotkeys()));
            }
            if let Some((path, hotkeys)) = persist {
                let _ = persist_hotkeys_to_config(Path::new(&path), &hotkeys);
            }
        });
    }
    {
        let model = model.clone();
        ptt_entry.connect_changed(move |entry| {
            let mut persist: Option<(String, Hotkeys)> = None;
            if let Ok(mut state) = model.lock() {
                state.push_to_talk_hotkey = entry.text().to_string();
                persist = Some((state.config_path_label.clone(), state.hotkeys()));
            }
            if let Some((path, hotkeys)) = persist {
                let _ = persist_hotkeys_to_config(Path::new(&path), &hotkeys);
            }
        });
    }
    {
        let model = model.clone();
        toggle_window_entry.connect_changed(move |entry| {
            let mut persist: Option<(String, Hotkeys)> = None;
            if let Ok(mut state) = model.lock() {
                state.toggle_window_hotkey = entry.text().to_string();
                persist = Some((state.config_path_label.clone(), state.hotkeys()));
            }
            if let Some((path, hotkeys)) = persist {
                let _ = persist_hotkeys_to_config(Path::new(&path), &hotkeys);
            }
        });
    }

    let config_group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    config_group.append(&gtk::Label::new(Some("Config")));
    if let Ok(state) = model.lock() {
        config_group.append(&gtk::Label::new(Some(&format!(
            "Path: {}",
            state.config_path_label
        ))));
    }

    let about_group = gtk::Box::new(gtk::Orientation::Vertical, 4);
    about_group.append(&gtk::Label::new(Some("About")));
    if let Ok(state) = model.lock() {
        about_group.append(&gtk::Label::new(Some(&state.about_label)));
    }

    root.append(&noise_group);
    root.append(&hotkeys_group);
    root.append(&config_group);
    root.append(&about_group);
    root
}
