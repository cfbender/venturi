use crate::core::messages::{Channel, CoreCommand};
use crate::core::volume::{apply_mute, linear_to_db};

#[derive(Debug, Clone)]
pub struct ChannelStrip {
    pub channel: Channel,
    pub icon: &'static str,
    pub label: &'static str,
    pub volume_linear: f32,
    pub muted: bool,
}

impl ChannelStrip {
    pub fn new(channel: Channel, icon: &'static str, label: &'static str) -> Self {
        Self {
            channel,
            icon,
            label,
            volume_linear: 1.0,
            muted: false,
        }
    }

    pub fn db_text(&self) -> String {
        let value = apply_mute(self.volume_linear, self.muted);
        if value <= 0.0 {
            "-∞ dB".to_string()
        } else {
            format!("{:.1} dB", linear_to_db(value))
        }
    }

    pub fn set_volume_command(&mut self, volume_linear: f32) -> CoreCommand {
        self.volume_linear = volume_linear;
        CoreCommand::SetVolume(self.channel, volume_linear)
    }

    pub fn set_mute_command(&mut self, muted: bool) -> CoreCommand {
        self.muted = muted;
        CoreCommand::SetMute(self.channel, muted)
    }
}
