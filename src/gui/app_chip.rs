use crate::core::messages::Channel;
use gtk::prelude::*;

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

pub fn build_chip_widget(chip: &AppChip) -> gtk::Button {
    let label = gtk::Label::new(Some(&chip.display_name));
    label.add_css_class("chip-text");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(16);

    let status_dot = gtk::Label::new(Some(match chip.status {
        ChipStatus::Playing => "🟢",
        ChipStatus::Idle => "⚪",
        ChipStatus::Muted => "🔇",
    }));
    status_dot.add_css_class("chip-status");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&status_dot);
    row.append(&label);

    let button = gtk::Button::builder().child(&row).build();
    button.add_css_class("flat");
    match chip.channel {
        Channel::Main => button.add_css_class("chip-main"),
        Channel::Mic => button.add_css_class("chip-mic"),
        Channel::Game => button.add_css_class("chip-game"),
        Channel::Media => button.add_css_class("chip-media"),
        Channel::Chat => button.add_css_class("chip-chat"),
        Channel::Aux => button.add_css_class("chip-aux"),
    }
    button.set_halign(gtk::Align::Fill);
    button.set_margin_start(2);
    button.set_margin_end(2);
    button.set_margin_top(1);
    button.set_margin_bottom(1);

    let payload = DndPayload {
        stream_id: chip.stream_id,
        app_key: chip.app_key.clone(),
        origin: chip.channel,
    };

    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::MOVE)
        .build();
    drag.connect_prepare(move |_, _, _| {
        let encoded = payload.encode();
        Some(gtk::gdk::ContentProvider::for_value(&encoded.to_value()))
    });
    button.add_controller(drag);
    button
}
