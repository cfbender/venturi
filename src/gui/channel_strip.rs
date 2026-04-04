use crate::core::messages::{Channel, CoreCommand};
use crate::core::volume::apply_mute;
use crossbeam_channel::Sender;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const TRACK_TOP_INSET_PX: i32 = 8;
const METER_TOP_INSET_PX: i32 = 12;
const TRACK_BOTTOM_INSET_PX: i32 = 0;
const SLIDER_BOTTOM_OFFSET_ADJUST_PX: i32 = -10;

#[derive(Debug, Clone)]
pub struct ChannelStrip {
    pub channel: Channel,
    pub icon: &'static str,
    pub label: &'static str,
    pub volume_linear: f32,
    pub muted: bool,
}

pub(crate) fn linear_to_slider_fraction(volume_linear: f32) -> f32 {
    volume_linear.clamp(0.0, 1.0)
}

fn slider_fraction_to_linear(slider_fraction: f64) -> f32 {
    slider_fraction.clamp(0.0, 1.0) as f32
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

/// Handle to a channel strip's slider widget with suppression flags for
/// coordinating programmatic updates vs user interaction.
pub struct SliderHandle {
    pub scale: gtk::Scale,
    pub value_label: gtk::Label,
    pub suppress_signal: Rc<Cell<bool>>,
    pub is_dragging: Rc<Cell<bool>>,
}

fn default_slider_flags() -> (Rc<Cell<bool>>, Rc<Cell<bool>>) {
    (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)))
}

pub fn build_strip_widget(strip: ChannelStrip, command_tx: Sender<CoreCommand>) -> gtk::Box {
    build_strip_widget_with_meter(strip, command_tx).0
}

pub fn build_strip_widget_with_meter(
    strip: ChannelStrip,
    command_tx: Sender<CoreCommand>,
) -> (gtk::Box, gtk::ProgressBar, SliderHandle) {
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
    meter.set_orientation(gtk::Orientation::Vertical);
    meter.set_inverted(true);
    meter.set_vexpand(true);
    meter.set_halign(gtk::Align::Center);
    meter.set_valign(gtk::Align::Fill);
    meter.set_size_request(6, -1);
    meter.set_margin_top(METER_TOP_INSET_PX);
    meter.set_margin_bottom(TRACK_BOTTOM_INSET_PX);
    meter.set_can_target(false);
    meter.set_visible(false);
    meter.add_css_class("slider-meter");
    meter.add_css_class(meter_css_class_for(channel));

    let slider = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, 1.0, 0.01);
    slider.set_value(linear_to_slider_fraction(state.borrow().volume_linear) as f64);
    slider.set_inverted(true);
    slider.set_vexpand(true);
    slider.set_margin_top(TRACK_TOP_INSET_PX);
    slider.set_margin_bottom(SLIDER_BOTTOM_OFFSET_ADJUST_PX);

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
    let (suppress_signal, is_dragging) = default_slider_flags();

    let mute = gtk::ToggleButton::with_label("Mute");
    mute.set_active(state.borrow().muted);

    {
        let state = state.clone();
        let tx = command_tx.clone();
        let db_label = db_label.clone();
        let last_sent_at = last_sent_at.clone();
        let suppress_clone = suppress_signal.clone();
        slider.connect_value_changed(move |scale| {
            if suppress_clone.get() {
                return;
            }
            let mut state = state.borrow_mut();
            let now = Instant::now();
            let cmd = state.set_volume_command(slider_fraction_to_linear(scale.value()));
            db_label.set_text(&state.volume_text());

            if should_emit_volume_update(*last_sent_at.borrow(), now, false) {
                let _ = tx.send(cmd);
                *last_sent_at.borrow_mut() = now;
            }
        });
    }

    {
        let state = state.clone();
        let tx = command_tx.clone();
        let db_label = db_label.clone();
        let last_sent_at = last_sent_at.clone();
        let slider_for_release = slider.clone();
        let is_dragging_press = is_dragging.clone();
        let is_dragging_release = is_dragging.clone();
        let release = gtk::GestureClick::new();
        release.connect_pressed(move |_, _, _, _| {
            is_dragging_press.set(true);
        });
        release.connect_released(move |_, _, _, _| {
            is_dragging_release.set(false);
            let now = Instant::now();
            let mut state = state.borrow_mut();
            let cmd = release_volume_command(&mut state, slider_for_release.value());
            if should_emit_volume_update(*last_sent_at.borrow(), now, true) {
                let _ = tx.send(cmd);
                *last_sent_at.borrow_mut() = now;
            }
            db_label.set_text(&state.volume_text());
        });
        slider.add_controller(release);
    }

    {
        let state = state.clone();
        let tx = command_tx.clone();
        let db_label = db_label.clone();
        mute.connect_toggled(move |btn| {
            let mut state = state.borrow_mut();
            let cmd = state.set_mute_command(btn.is_active());
            db_label.set_text(&state.volume_text());
            let _ = tx.send(cmd);
        });
    }

    let slider_overlay = gtk::Overlay::new();
    slider_overlay.set_hexpand(true);
    slider_overlay.set_vexpand(true);
    slider_overlay.set_child(Some(&meter));
    slider_overlay.add_overlay(&slider);
    slider_overlay.set_measure_overlay(&slider, true);

    root.append(&header);
    root.append(&slider_overlay);
    root.append(&db_label);
    root.append(&mute);

    let handle = SliderHandle {
        scale: slider.clone(),
        value_label: db_label.clone(),
        suppress_signal,
        is_dragging,
    };

    (root, meter, handle)
}

fn should_emit_volume_update(_last_sent_at: Instant, _now: Instant, _is_release: bool) -> bool {
    true
}

fn release_volume_command(state: &mut ChannelStrip, slider_value: f64) -> CoreCommand {
    state.set_volume_command(slider_fraction_to_linear(slider_value))
}

fn meter_css_class_for(channel: Channel) -> &'static str {
    match channel {
        Channel::Main => "meter-main",
        Channel::Mic => "meter-mic",
        Channel::Game => "meter-game",
        Channel::Media => "meter-media",
        Channel::Chat => "meter-chat",
        Channel::Aux => "meter-aux",
    }
}

#[cfg(test)]
mod tests {
    use crate::core::messages::{Channel, CoreCommand};
    use std::time::{Duration, Instant};

    #[test]
    fn maps_channel_to_meter_css_class() {
        assert_eq!(super::meter_css_class_for(Channel::Main), "meter-main");
        assert_eq!(super::meter_css_class_for(Channel::Mic), "meter-mic");
        assert_eq!(super::meter_css_class_for(Channel::Game), "meter-game");
        assert_eq!(super::meter_css_class_for(Channel::Media), "meter-media");
        assert_eq!(super::meter_css_class_for(Channel::Chat), "meter-chat");
        assert_eq!(super::meter_css_class_for(Channel::Aux), "meter-aux");
    }

    #[test]
    fn emits_volume_update_during_fast_drag_without_waiting_for_release() {
        let now = Instant::now();
        let just_sent = now - Duration::from_millis(20);

        assert!(super::should_emit_volume_update(just_sent, now, false));
        assert!(super::should_emit_volume_update(just_sent, now, true));
    }

    #[test]
    fn slider_handle_suppression_flags_default_to_false() {
        let (suppress_signal, is_dragging) = super::default_slider_flags();
        assert!(!suppress_signal.get());
        assert!(!is_dragging.get());
    }

    #[test]
    fn release_volume_command_uses_current_slider_value() {
        let mut strip = super::ChannelStrip::new(Channel::Main, "🔊", "Main");
        strip.volume_linear = 0.98;

        let cmd = super::release_volume_command(&mut strip, 0.42);

        assert!(
            matches!(cmd, CoreCommand::SetVolume(Channel::Main, v) if (v - 0.42).abs() < 0.001)
        );
        assert!((strip.volume_linear - 0.42).abs() < 0.001);
    }

    #[test]
    fn volume_text_matches_linear_percentage_scale() {
        let mut strip = super::ChannelStrip::new(Channel::Media, "🎮", "Media");
        strip.volume_linear = 0.421_875;

        assert_eq!(strip.volume_text(), "42%");
    }
}
