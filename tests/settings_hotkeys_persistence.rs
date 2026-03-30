use tempfile::tempdir;
use venturi::config::persistence::{Paths, ensure_dirs, load_config, save_config};
use venturi::config::schema::Config;
use venturi::gui::settings_tab::persist_hotkeys_to_config;

#[test]
fn settings_hotkey_updates_are_persisted_to_config_file() {
    let tmp = tempdir().expect("tempdir");
    let paths = Paths {
        config_dir: tmp.path().join("config").join("venturi"),
        state_dir: tmp.path().join("state").join("venturi"),
    };
    ensure_dirs(&paths).expect("ensure dirs");

    let mut cfg = Config::default();
    cfg.hotkeys.mute_main = "ctrl+shift+m".to_string();
    save_config(&paths, &cfg).expect("save config");

    let mut new_hotkeys = cfg.hotkeys.clone();
    new_hotkeys.mute_main = "super+m".to_string();
    new_hotkeys.toggle_window = "super+v".to_string();

    persist_hotkeys_to_config(&paths.config_file(), &new_hotkeys).expect("persist hotkeys");

    let loaded = load_config(&paths);
    assert_eq!(loaded.hotkeys.mute_main, "super+m");
    assert_eq!(loaded.hotkeys.toggle_window, "super+v");
}
