use crate::core::messages::Channel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChipStatus {
    Playing,
    Idle,
    Muted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppChip {
    pub stream_id: u32,
    pub app_key: String,
    pub display_name: String,
    pub channel: Channel,
    pub status: ChipStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DndPayload {
    pub stream_id: u32,
    pub app_key: String,
    pub origin: Channel,
}

impl DndPayload {
    pub fn encode(&self) -> String {
        format!("{}|{}|{:?}", self.stream_id, self.app_key, self.origin)
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let mut parts = raw.split('|');
        let stream_id = parts.next()?.parse::<u32>().ok()?;
        let app_key = parts.next()?.to_string();
        let channel = match parts.next()? {
            "Main" => Channel::Main,
            "Game" => Channel::Game,
            "Media" => Channel::Media,
            "Chat" => Channel::Chat,
            "Aux" => Channel::Aux,
            "Mic" => Channel::Mic,
            _ => return None,
        };

        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            stream_id,
            app_key,
            origin: channel,
        })
    }
}
