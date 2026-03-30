use std::fs;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use venturi::config::persistence::{
    CURRENT_CONFIG_VERSION, DebouncedSaver, Paths, ensure_dirs, load_config, load_state,
    save_config, save_state,
};
use venturi::config::schema::{Config, Palette, State};

#[test]
fn resolves_xdg_paths() {
    let tmp = tempdir().expect("tempdir");
    let cfg = tmp.path().join("cfg");
    let state = tmp.path().join("state");

    let paths = Paths::from_bases(cfg, state);
    assert!(paths.config_dir.ends_with("venturi"));
    assert!(paths.state_dir.ends_with("venturi"));
    assert!(paths.config_file().ends_with("config.toml"));
    assert!(paths.state_file().ends_with("state.toml"));
}

#[test]
fn malformed_toml_falls_back_to_defaults() {
    let tmp = tempdir().expect("tempdir");
    let paths = Paths {
        config_dir: tmp.path().join("config").join("venturi"),
        state_dir: tmp.path().join("state").join("venturi"),
    };
    ensure_dirs(&paths).expect("ensure dirs");

    fs::write(paths.config_file(), "[general\nversion = ???").expect("write malformed config");
    fs::write(paths.state_file(), "[volumes\nmain = ???").expect("write malformed state");

    let config = load_config(&paths);
    let state = load_state(&paths);

    assert_eq!(config, Config::default());
    assert_eq!(state, State::default());
}

#[test]
fn runtime_volume_and_mute_live_only_in_state_file() {
    let tmp = tempdir().expect("tempdir");
    let paths = Paths {
        config_dir: tmp.path().join("config").join("venturi"),
        state_dir: tmp.path().join("state").join("venturi"),
    };
    ensure_dirs(&paths).expect("ensure dirs");

    let config = Config::default();
    let mut state = State::default();
    state.volumes.game = 0.45;
    state.muted.chat = true;

    save_config(&paths, &config).expect("save config");
    save_state(&paths, &state).expect("save state");

    let config_raw = fs::read_to_string(paths.config_file()).expect("read config");
    let state_raw = fs::read_to_string(paths.state_file()).expect("read state");

    assert!(!config_raw.contains("[volumes]"));
    assert!(!config_raw.contains("[muted]"));
    assert!(state_raw.contains("[volumes]"));
    assert!(state_raw.contains("[muted]"));
}

#[test]
fn migrates_old_config_version_to_current() {
    let tmp = tempdir().expect("tempdir");
    let paths = Paths {
        config_dir: tmp.path().join("config").join("venturi"),
        state_dir: tmp.path().join("state").join("venturi"),
    };
    ensure_dirs(&paths).expect("ensure dirs");

    let mut old = Config::default();
    old.general.version = 0;
    save_config(&paths, &old).expect("save old config");

    let migrated = load_config(&paths);
    assert_eq!(migrated.general.version, CURRENT_CONFIG_VERSION);
}

#[test]
fn debounce_waits_500ms_from_last_change() {
    let mut saver = DebouncedSaver::new();
    let start = Instant::now();
    saver.mark_dirty(start);

    assert!(!saver.should_flush(start + Duration::from_millis(499)));
    assert!(saver.should_flush(start + Duration::from_millis(500)));

    saver.did_flush();
    assert!(!saver.should_flush(start + Duration::from_millis(1000)));
}

#[test]
fn palette_overrides_roundtrip_via_config_file() {
    let tmp = tempdir().expect("tempdir");
    let paths = Paths {
        config_dir: tmp.path().join("config").join("venturi"),
        state_dir: tmp.path().join("state").join("venturi"),
    };
    ensure_dirs(&paths).expect("ensure dirs");

    let config = Config {
        palette: Some(Palette {
            main: "#938AA9".to_string(),
            mic: "#7AA89F".to_string(),
            game: "#87A987".to_string(),
            media: "#E46876".to_string(),
            chat: "#7FB4CA".to_string(),
            aux: "#E6C384".to_string(),
        }),
        ..Config::default()
    };

    save_config(&paths, &config).expect("save config");
    let loaded = load_config(&paths);

    assert_eq!(loaded.palette, config.palette);
}
