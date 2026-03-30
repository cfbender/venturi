use std::collections::BTreeMap;

use crate::core::messages::{Channel, CoreEvent};
use crate::gui::app_chip::AppChip;
use crate::gui::channel_strip::ChannelStrip;

pub const NO_DEVICES_FOUND: &str = "No devices found";

#[derive(Debug, Clone, Default)]
pub struct DeviceListModel {
    pub output_devices: Vec<String>,
    pub input_devices: Vec<String>,
}

impl DeviceListModel {
    pub fn output_label(&self) -> &str {
        self.output_devices
            .first()
            .map(String::as_str)
            .unwrap_or(NO_DEVICES_FOUND)
    }

    pub fn input_label(&self) -> &str {
        self.input_devices
            .first()
            .map(String::as_str)
            .unwrap_or(NO_DEVICES_FOUND)
    }

    pub fn set_from_devices_changed(&mut self, devices: &[String]) {
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();

        for device in devices {
            if let Some(rest) = device.strip_prefix("out:") {
                outputs.push(rest.to_string());
            } else if let Some(rest) = device.strip_prefix("in:") {
                inputs.push(rest.to_string());
            }
        }

        self.output_devices = outputs;
        self.input_devices = inputs;
    }

    pub fn reset_to_default_on_disconnect(&mut self) {
        self.output_devices = vec!["Default".to_string()];
        self.input_devices = vec!["Default".to_string()];
    }
}

#[derive(Debug, Clone)]
pub struct MixerTab {
    pub strips: BTreeMap<Channel, ChannelStrip>,
    pub chips: BTreeMap<Channel, Vec<AppChip>>,
    pub devices: DeviceListModel,
    pub banner: Option<String>,
    pub toast: Option<String>,
}

impl MixerTab {
    pub fn new() -> Self {
        let mut strips = BTreeMap::new();
        strips.insert(
            Channel::Main,
            ChannelStrip::new(Channel::Main, "🔊", "Main"),
        );
        strips.insert(
            Channel::Game,
            ChannelStrip::new(Channel::Game, "🎮", "Game"),
        );
        strips.insert(
            Channel::Media,
            ChannelStrip::new(Channel::Media, "🎵", "Media"),
        );
        strips.insert(
            Channel::Chat,
            ChannelStrip::new(Channel::Chat, "💬", "Chat"),
        );
        strips.insert(Channel::Aux, ChannelStrip::new(Channel::Aux, "📦", "Aux"));
        strips.insert(Channel::Mic, ChannelStrip::new(Channel::Mic, "🎤", "Mic"));

        Self {
            strips,
            chips: BTreeMap::new(),
            devices: DeviceListModel::default(),
            banner: None,
            toast: None,
        }
    }

    pub fn apply_event(&mut self, event: &CoreEvent) {
        match event {
            CoreEvent::StreamAppeared { id, name, category } => {
                let chip = AppChip {
                    stream_id: *id,
                    app_key: name.to_ascii_lowercase(),
                    display_name: name.clone(),
                    channel: *category,
                    status: crate::gui::app_chip::ChipStatus::Idle,
                };
                self.chips.entry(*category).or_default().push(chip);
            }
            CoreEvent::StreamRemoved(id) => {
                for chips in self.chips.values_mut() {
                    chips.retain(|chip| chip.stream_id != *id);
                }
            }
            CoreEvent::DevicesChanged(devices) => {
                self.devices.set_from_devices_changed(devices);
                if self.devices.output_devices.is_empty() && self.devices.input_devices.is_empty() {
                    self.toast = Some("No audio devices found".to_string());
                }
            }
            CoreEvent::Error(msg) => {
                self.banner = Some(msg.clone());
            }
            _ => {}
        }
    }

    pub fn on_device_disconnect(&mut self) {
        self.devices.reset_to_default_on_disconnect();
        self.toast = Some("Device disconnected. Reset to Default.".to_string());
    }
}

impl Default for MixerTab {
    fn default() -> Self {
        Self::new()
    }
}
