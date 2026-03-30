use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::schema::{Config, State};

pub const CURRENT_CONFIG_VERSION: u32 = 1;
pub const SAVE_DEBOUNCE_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let config_base =
            std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
        let state_base =
            std::env::var("XDG_STATE_HOME").unwrap_or_else(|_| format!("{home}/.local/state"));
        Self::from_bases(PathBuf::from(config_base), PathBuf::from(state_base))
    }

    pub fn from_bases(config_base: PathBuf, state_base: PathBuf) -> Self {
        Self {
            config_dir: config_base.join("venturi"),
            state_dir: state_base.join("venturi"),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.toml")
    }
}

#[derive(Debug, Clone)]
pub struct DebouncedSaver {
    debounce: Duration,
    pending: bool,
    last_change: Option<Instant>,
}

impl DebouncedSaver {
    pub fn new() -> Self {
        Self {
            debounce: Duration::from_millis(SAVE_DEBOUNCE_MS),
            pending: false,
            last_change: None,
        }
    }

    pub fn mark_dirty(&mut self, now: Instant) {
        self.pending = true;
        self.last_change = Some(now);
    }

    pub fn should_flush(&self, now: Instant) -> bool {
        self.pending
            && self
                .last_change
                .is_some_and(|at| now.duration_since(at) >= self.debounce)
    }

    pub fn did_flush(&mut self) {
        self.pending = false;
        self.last_change = None;
    }
}

impl Default for DebouncedSaver {
    fn default() -> Self {
        Self::new()
    }
}

pub fn ensure_dirs(paths: &Paths) -> Result<(), String> {
    fs::create_dir_all(&paths.config_dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(&paths.state_dir).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load_config(paths: &Paths) -> Config {
    load_or_default(paths.config_file(), Config::default(), migrate_config)
}

pub fn load_state(paths: &Paths) -> State {
    load_or_default(paths.state_file(), State::default(), |state| state)
}

pub fn save_config(paths: &Paths, config: &Config) -> Result<(), String> {
    save_toml(paths.config_file(), config)
}

pub fn save_state(paths: &Paths, state: &State) -> Result<(), String> {
    save_toml(paths.state_file(), state)
}

pub fn migrate_config(mut config: Config) -> Config {
    if config.general.version < CURRENT_CONFIG_VERSION {
        config.general.version = CURRENT_CONFIG_VERSION;
    }
    config
}

fn load_or_default<T, F>(path: PathBuf, default_value: T, migrate: F) -> T
where
    T: serde::de::DeserializeOwned,
    F: Fn(T) -> T,
{
    let raw = fs::read_to_string(path);
    match raw {
        Ok(content) => toml::from_str::<T>(&content)
            .map(migrate)
            .unwrap_or(default_value),
        Err(_) => default_value,
    }
}

fn save_toml<T>(path: impl AsRef<Path>, value: &T) -> Result<(), String>
where
    T: serde::Serialize,
{
    let toml_text = toml::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, toml_text).map_err(|e| e.to_string())
}
