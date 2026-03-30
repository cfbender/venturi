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
