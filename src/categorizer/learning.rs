use std::collections::BTreeMap;

use crate::core::messages::Channel;

#[derive(Debug, Default, Clone)]
pub struct Overrides {
    map: BTreeMap<String, Channel>,
}

impl Overrides {
    pub fn insert(&mut self, key: impl Into<String>, channel: Channel) {
        self.map.insert(key.into(), channel);
    }

    pub fn get(&self, key: &str) -> Option<Channel> {
        self.map.get(key).copied()
    }

    pub fn as_map(&self) -> &BTreeMap<String, Channel> {
        &self.map
    }
}

pub fn serialize_overrides(overrides: &BTreeMap<String, Channel>) -> BTreeMap<String, String> {
    overrides
        .iter()
        .map(|(key, channel)| (key.clone(), channel_to_config(*channel).to_string()))
        .collect()
}

pub fn deserialize_overrides(stored: &BTreeMap<String, String>) -> BTreeMap<String, Channel> {
    stored
        .iter()
        .filter_map(|(key, value)| channel_from_config(value).map(|channel| (key.clone(), channel)))
        .collect()
}

fn channel_to_config(channel: Channel) -> &'static str {
    match channel {
        Channel::Main => "main",
        Channel::Game => "game",
        Channel::Media => "media",
        Channel::Chat => "chat",
        Channel::Aux => "aux",
        Channel::Mic => "mic",
    }
}

fn channel_from_config(raw: &str) -> Option<Channel> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "main" => Some(Channel::Main),
        "game" => Some(Channel::Game),
        "media" => Some(Channel::Media),
        "chat" => Some(Channel::Chat),
        "aux" => Some(Channel::Aux),
        "mic" => Some(Channel::Mic),
        _ => None,
    }
}
