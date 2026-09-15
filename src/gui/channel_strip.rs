use crate::core::messages::{Channel, CoreCommand};
use crate::core::meter::apply_mute;
use crate::gui::mixer_tab::MixerTab;
use crossbeam_channel::Sender;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
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

/// Handle to a channel strip's widgets with suppression flags for
/// coordinating programmatic updates vs user interaction.
pub struct SliderHandle {
    pub scale: gtk::Scale,
    pub value_label: gtk::Label,
    pub mute_button: gtk::ToggleButton,
    /// Set while the refresh tick writes widget state so the change handlers
    /// don't echo it back as a user action.
    pub suppress_signal: Rc<Cell<bool>>,
    pub is_dragging: Rc<Cell<bool>>,
    /// Last time the user touched this strip (slider, wheel, keys or mute).
    /// The refresh tick leaves the widgets alone for a short grace period after
    /// this so the core's echo can't yank them back to the previous value.
    pub last_user_input_at: Rc<Cell<Instant>>,
}

/// How long after a user interaction the refresh tick must not overwrite the
/// strip's widgets from the model. Long enough for the core to apply the change
/// and echo the resulting `VolumeChanged`/`MuteChanged` back.
pub(crate) const USER_INPUT_GRACE: Duration = Duration::from_millis(400);

impl SliderHandle {
    pub(crate) fn within_user_input_grace(&self, now: Instant) -> bool {
        user_input_grace_active(self.last_user_input_at.get(), now)
    }
}

pub(crate) fn user_input_grace_active(last_user_input_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_user_input_at) < USER_INPUT_GRACE
}

fn default_slider_flags() -> (Rc<Cell<bool>>, Rc<Cell<bool>>) {
    (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)))
}

fn long_ago() -> Instant {
    Instant::now() - Duration::from_secs(60)
}

/// Build the widgets for one channel strip.
///
/// User interaction writes into the shared `model` (the same one the refresh
/// tick reads from) and sends the matching `CoreCommand`. The widget never
/// keeps its own private copy of the volume/mute state, so the label, slider
/// and mute button can't disagree with the model.
pub fn build_strip_widget_with_meter(
    strip: ChannelStrip,
    model: Arc<Mutex<MixerTab>>,
    command_tx: Sender<CoreCommand>,
) -> (gtk::Box, gtk::ProgressBar, SliderHandle) {
    let channel = strip.channel;

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let header = gtk::Label::new(Some(&format!("{} {}", strip.icon, strip.label)));
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
    meter.add_css_class(&meter_css_class_for(channel));

    let slider = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, 1.0, 0.01);
    slider.set_value(linear_to_slider_fraction(strip.volume_linear) as f64);
    slider.set_inverted(true);
    slider.set_vexpand(true);
    slider.set_margin_top(TRACK_TOP_INSET_PX);
    slider.set_margin_bottom(SLIDER_BOTTOM_OFFSET_ADJUST_PX);

    slider.add_css_class(&format!("slider-{}", channel.css_class()));

    let db_label = gtk::Label::new(Some(&strip.volume_text()));
    let (suppress_signal, is_dragging) = default_slider_flags();
    let last_user_input_at = Rc::new(Cell::new(long_ago()));

    let mute = gtk::ToggleButton::with_label("Mute");
    mute.set_active(strip.muted);

    {
        let model = model.clone();
        let tx = command_tx.clone();
        let db_label = db_label.clone();
        let suppress_signal = suppress_signal.clone();
        let last_user_input_at = last_user_input_at.clone();
        slider.connect_value_changed(move |scale| {
            if suppress_signal.get() {
                return;
            }
            last_user_input_at.set(Instant::now());
            let volume = slider_fraction_to_linear(scale.value());
            if let Some((cmd, text)) = with_strip(&model, channel, |strip| {
                let cmd = strip.set_volume_command(volume);
                (cmd, strip.volume_text())
            }) {
                db_label.set_text(&text);
                let _ = tx.send(cmd);
            }
        });
    }

    {
        let is_dragging_press = is_dragging.clone();
        let is_dragging_release = is_dragging.clone();
        let last_user_input_at = last_user_input_at.clone();
        let drag = gtk::GestureClick::new();
        drag.connect_pressed(move |_, _, _, _| {
            is_dragging_press.set(true);
        });
        drag.connect_released(move |_, _, _, _| {
            is_dragging_release.set(false);
            last_user_input_at.set(Instant::now());
        });
        slider.add_controller(drag);
    }

    {
        let model = model.clone();
        let tx = command_tx.clone();
        let db_label = db_label.clone();
        let suppress_signal = suppress_signal.clone();
        let last_user_input_at = last_user_input_at.clone();
        mute.connect_toggled(move |btn| {
            if suppress_signal.get() {
                return;
            }
            last_user_input_at.set(Instant::now());
            let muted = btn.is_active();
            if let Some((cmd, text)) = with_strip(&model, channel, |strip| {
                let cmd = strip.set_mute_command(muted);
                (cmd, strip.volume_text())
            }) {
                db_label.set_text(&text);
                let _ = tx.send(cmd);
            }
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
        mute_button: mute.clone(),
        suppress_signal,
        is_dragging,
        last_user_input_at,
    };

    (root, meter, handle)
}

/// Run `f` against the shared model's strip for `channel`, if present.
fn with_strip<T>(
    model: &Arc<Mutex<MixerTab>>,
    channel: Channel,
    f: impl FnOnce(&mut ChannelStrip) -> T,
) -> Option<T> {
    let mut mixer = model.lock().ok()?;
    mixer.strips.get_mut(&channel).map(f)
}

fn meter_css_class_for(channel: Channel) -> String {
    format!("meter-{}", channel.css_class())
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
    fn channel_css_class_returns_correct_suffix() {
        assert_eq!(Channel::Main.css_class(), "main");
        assert_eq!(Channel::Game.css_class(), "game");
        assert_eq!(Channel::Media.css_class(), "media");
        assert_eq!(Channel::Chat.css_class(), "chat");
        assert_eq!(Channel::Aux.css_class(), "aux");
        assert_eq!(Channel::Mic.css_class(), "mic");
    }

    #[test]
    fn slider_handle_suppression_flags_default_to_false() {
        let (suppress_signal, is_dragging) = super::default_slider_flags();
        assert!(!suppress_signal.get());
        assert!(!is_dragging.get());
    }

    #[test]
    fn user_input_grace_blocks_model_sync_only_briefly_after_interaction() {
        let touched = Instant::now();

        assert!(super::user_input_grace_active(touched, touched));
        assert!(super::user_input_grace_active(
            touched,
            touched + super::USER_INPUT_GRACE - Duration::from_millis(1)
        ));
        assert!(!super::user_input_grace_active(
            touched,
            touched + super::USER_INPUT_GRACE
        ));
        // A fresh handle starts "long ago" so the first tick syncs immediately.
        assert!(!super::user_input_grace_active(
            super::long_ago(),
            Instant::now()
        ));
    }

    #[test]
    fn mute_command_updates_strip_and_targets_channel() {
        let mut strip = super::ChannelStrip::new(Channel::Mic, "🎤", "Mic");

        let cmd = strip.set_mute_command(true);

        assert!(matches!(cmd, CoreCommand::SetMute(Channel::Mic, true)));
        assert!(strip.muted);
        assert_eq!(strip.volume_text(), "0%");
    }

    #[test]
    fn volume_text_matches_linear_percentage_scale() {
        let mut strip = super::ChannelStrip::new(Channel::Media, "🎮", "Media");
        strip.volume_linear = 0.421_875;

        assert_eq!(strip.volume_text(), "42%");
    }
}
