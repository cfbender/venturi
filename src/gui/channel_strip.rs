use crate::core::messages::{Channel, CoreCommand};
use crate::core::volume::apply_mute;
use crossbeam_channel::Sender;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

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

    pub fn volume_text(&self) -> String {
        let value = apply_mute(self.volume_linear, self.muted);
        format!("{:.0}%", (value * 100.0).clamp(0.0, 100.0))
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

pub fn build_strip_widget(strip: ChannelStrip, command_tx: Sender<CoreCommand>) -> gtk::Box {
    let state = Rc::new(RefCell::new(strip));
    let channel = state.borrow().channel;

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let header = gtk::Label::new(Some(&format!(
        "{} {}",
        state.borrow().icon,
        state.borrow().label
    )));
    header.add_css_class("title-4");

    let meter = gtk::ProgressBar::new();
    meter.set_fraction(0.0);
    meter.set_show_text(false);

    let slider = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, 1.0, 0.01);
    slider.set_value(state.borrow().volume_linear as f64);
    slider.set_inverted(true);
    slider.set_vexpand(true);

    match channel {
        Channel::Main => slider.add_css_class("slider-main"),
        Channel::Mic => slider.add_css_class("slider-mic"),
        Channel::Game => slider.add_css_class("slider-game"),
        Channel::Media => slider.add_css_class("slider-media"),
        Channel::Chat => slider.add_css_class("slider-chat"),
        Channel::Aux => slider.add_css_class("slider-aux"),
    }

    let db_label = gtk::Label::new(Some(&state.borrow().volume_text()));
    let last_sent_at = Rc::new(RefCell::new(Instant::now() - Duration::from_secs(1)));

    let mute = gtk::ToggleButton::with_label("Mute");
    mute.set_active(state.borrow().muted);

    {
        let state = state.clone();
        let tx = command_tx.clone();
        let db_label = db_label.clone();
        let last_sent_at = last_sent_at.clone();
        slider.connect_value_changed(move |scale| {
            let mut state = state.borrow_mut();
            let now = Instant::now();
            let cmd = state.set_volume_command(scale.value() as f32);
            db_label.set_text(&state.volume_text());

            if now.duration_since(*last_sent_at.borrow()) >= Duration::from_millis(120) {
                let _ = tx.send(cmd);
                *last_sent_at.borrow_mut() = now;
            }
        });
    }

    {
        let state = state.clone();
        let tx = command_tx;
        let db_label = db_label.clone();
        mute.connect_toggled(move |btn| {
            let mut state = state.borrow_mut();
            let cmd = state.set_mute_command(btn.is_active());
            db_label.set_text(&state.volume_text());
            let _ = tx.send(cmd);
        });
    }

    root.append(&header);
    root.append(&meter);
    root.append(&slider);
    root.append(&db_label);
    root.append(&mute);
    root
}
