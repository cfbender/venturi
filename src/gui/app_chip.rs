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
    pub stream_ids: Vec<u32>,
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
        let channel = parts.next()?.parse().ok()?;

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
    let chip_label = if chip.stream_ids.len() > 1 {
        format!("{} ({})", chip.display_name, chip.stream_ids.len())
    } else {
        chip.display_name.clone()
    };
    let label = gtk::Label::new(Some(&chip_label));
    label.add_css_class("chip-text");
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(7);

    let status_dot = gtk::Label::new(Some(match chip.status {
        ChipStatus::Playing => "🟢",
        ChipStatus::Idle => "⚪",
        ChipStatus::Muted => "🔇",
    }));
    status_dot.add_css_class("chip-status");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    row.append(&status_dot);
    row.append(&label);

    let button = gtk::Button::builder().child(&row).build();
    button.add_css_class("flat");
    button.add_css_class("chip-button");
    button.add_css_class(&format!("chip-{}", chip.channel.css_class()));
    button.set_halign(gtk::Align::Center);
    button.set_valign(gtk::Align::Start);
    button.set_hexpand(false);
    button.set_vexpand(false);
    button.set_margin_start(0);
    button.set_margin_end(0);
    button.set_margin_top(0);
    button.set_margin_bottom(0);

    let payload = DndPayload {
        stream_id: chip.stream_id,
        app_key: chip.app_key.clone(),
        origin: chip.channel,
    };

    let drag = gtk::DragSource::builder()
        .actions(gtk::gdk::DragAction::MOVE)
        .build();

    {
        let button_for_icon = button.clone();
        drag.connect_drag_begin(move |source, _drag| {
            let paintable = gtk::WidgetPaintable::new(Some(&button_for_icon));
            source.set_icon(Some(&paintable), 8, 8);
            button_for_icon.set_opacity(0.82);
        });
    }

    {
        let button_for_end = button.clone();
        drag.connect_drag_end(move |_, _drag, _delete_data| {
            button_for_end.set_opacity(1.0);
        });
    }

    {
        let button_for_cancel = button.clone();
        drag.connect_drag_cancel(move |_, _drag, _reason| {
            button_for_cancel.set_opacity(1.0);
            false
        });
    }

    drag.connect_prepare(move |_, _, _| {
        let encoded = payload.encode();
        Some(gtk::gdk::ContentProvider::for_value(&encoded.to_value()))
    });
    button.add_controller(drag);
    button
}
