use std::time::Duration;
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
};

use adw::prelude::*;
use crossbeam_channel::{Receiver, Sender};

use crate::app::GuiLauncher;
use crate::config::persistence::{Paths, load_config, load_state};
use crate::config::schema::Palette;
use crate::core::messages::{Channel, CoreCommand, CoreEvent};
use crate::gui::mixer_tab::{MixerTab, build_mixer_widget};
use crate::gui::settings_tab::{SettingsTab, build_settings_widget};
use crate::gui::soundboard_tab::{SoundboardTab, build_soundboard_widget};

pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    Mixer,
    Soundboard,
    Settings,
}

#[derive(Debug, Clone)]
pub struct MainWindow {
    pub width: u32,
    pub height: u32,
    pub active_tab: Tab,
    pub mixer: MixerTab,
    pub soundboard: SoundboardTab,
    pub settings: SettingsTab,
    pub reconnect_every: Duration,
}

impl MainWindow {
    pub fn new(config_path: String, runtime_versions: String) -> Self {
        Self {
            width: 900,
            height: 600,
            active_tab: Tab::Mixer,
            mixer: MixerTab::new(),
            soundboard: SoundboardTab::default(),
            settings: SettingsTab::new(config_path, runtime_versions),
            reconnect_every: RECONNECT_INTERVAL,
        }
    }

    pub fn apply_core_event(&mut self, event: &CoreEvent) {
        self.mixer.apply_event(event);
    }

    pub fn on_pipewire_disconnect(&mut self) {
        self.mixer.banner = Some("Connection to PipeWire lost. Reconnecting...".to_string());
    }

    pub fn on_pipewire_not_running(&mut self) {
        self.mixer.banner =
            Some("PipeWire not detected. Start PipeWire and restart Venturi.".to_string());
    }

    pub fn on_config_corrupt(&mut self) {
        self.mixer.toast = Some("Config was reset due to errors.".to_string());
    }
}

fn ui_selected_device_from_config(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    if raw.eq_ignore_ascii_case("default") {
        return Some("Default".to_string());
    }
    Some(raw.to_string())
}

fn should_metering_be_enabled(window_visible: bool, _window_active: bool) -> bool {
    window_visible
}

fn apply_persisted_strip_values(mixer: &mut MixerTab, persisted: &crate::config::schema::State) {
    set_strip_value(
        mixer,
        Channel::Main,
        persisted.volumes.main,
        persisted.muted.main,
    );
    set_strip_value(
        mixer,
        Channel::Game,
        persisted.volumes.game,
        persisted.muted.game,
    );
    set_strip_value(
        mixer,
        Channel::Media,
        persisted.volumes.media,
        persisted.muted.media,
    );
    set_strip_value(
        mixer,
        Channel::Chat,
        persisted.volumes.chat,
        persisted.muted.chat,
    );
    set_strip_value(
        mixer,
        Channel::Aux,
        persisted.volumes.aux,
        persisted.muted.aux,
    );
    set_strip_value(
        mixer,
        Channel::Mic,
        persisted.volumes.mic,
        persisted.muted.mic,
    );
}

fn set_strip_value(mixer: &mut MixerTab, channel: Channel, volume: f32, muted: bool) {
    if let Some(strip) = mixer.strips.get_mut(&channel) {
        strip.volume_linear = volume.clamp(0.0, 1.0);
        strip.muted = muted;
    }
}

#[derive(Debug, Clone, Default)]
pub struct GtkGuiLauncher;

impl GuiLauncher for GtkGuiLauncher {
    fn launch(
        &self,
        command_tx: Sender<CoreCommand>,
        event_rx: Receiver<CoreEvent>,
    ) -> Result<(), String> {
        run_gtk_app(command_tx, event_rx)
    }
}

pub fn run_gtk_app(
    command_tx: Sender<CoreCommand>,
    event_rx: Receiver<CoreEvent>,
) -> Result<(), String> {
    let app = adw::Application::builder()
        .application_id("org.venturi.Venturi")
        .build();

    let command_tx_outer = command_tx.clone();
    let event_rx_outer = event_rx.clone();

    app.connect_activate(move |app| {
        adw::StyleManager::default().set_color_scheme(adw::ColorScheme::Default);
        let paths = Paths::resolve();
        let config = load_config(&paths);
        let persisted_state = load_state(&paths);
        install_mixer_css(config.palette.as_ref());

        let config_path = paths.config_file().display().to_string();
        let vm = Rc::new(RefCell::new(MainWindow::new(
            config_path,
            format!("Venturi {}", env!("CARGO_PKG_VERSION")),
        )));

        {
            let mut state = vm.borrow_mut();
            state.settings.mute_main_hotkey = config.hotkeys.mute_main.clone();
            state.settings.mute_mic_hotkey = config.hotkeys.mute_mic.clone();
            state.settings.push_to_talk_hotkey = config.hotkeys.push_to_talk.clone();
            state.settings.toggle_window_hotkey = config.hotkeys.toggle_window.clone();
            state.settings.noise_gate_enabled = config.mic_processing.noise_gate_enabled;
            state.settings.noise_gate_threshold_db = config.mic_processing.noise_gate_threshold;
            state.mixer.devices.selected_output =
                ui_selected_device_from_config(&config.audio.output_device);
            state.mixer.devices.selected_input =
                ui_selected_device_from_config(&config.audio.input_device);
            apply_persisted_strip_values(&mut state.mixer, &persisted_state);
        }

        let mixer_model = Arc::new(Mutex::new(vm.borrow().mixer.clone()));
        let settings_model = Arc::new(Mutex::new(vm.borrow().settings.clone()));
        let soundboard_model = Arc::new(Mutex::new(vm.borrow().soundboard.clone()));

        let stack = adw::ViewStack::new();
        let switcher = adw::ViewSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_policy(adw::ViewSwitcherPolicy::Wide);

        let mixer = build_mixer_widget(mixer_model.clone(), command_tx_outer.clone());
        let soundboard =
            build_soundboard_widget(soundboard_model.clone(), command_tx_outer.clone());
        let settings = build_settings_widget(settings_model);

        let mixer_page = stack.add_titled(&mixer, Some("mixer"), "Mixer");
        mixer_page.set_icon_name(Some("audio-speakers-symbolic"));
        let soundboard_page = stack.add_titled(&soundboard, Some("soundboard"), "Soundboard");
        soundboard_page.set_icon_name(Some("media-playback-start-symbolic"));
        let settings_page = stack.add_titled(&settings, Some("settings"), "Settings");
        settings_page.set_icon_name(Some("emblem-system-symbolic"));

        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&switcher));

        let brand = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        brand.add_css_class("brand-badge");
        let brand_mark = build_brand_logo();
        brand_mark.add_css_class("brand-mark");
        let brand_name = gtk::Label::new(Some("Venturi"));
        brand_name.add_css_class("brand-name");
        brand.append(&brand_mark);
        brand.append(&brand_name);
        header.pack_start(&brand);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&header);
        content.append(&stack);

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Venturi")
            .default_width(vm.borrow().width as i32)
            .default_height(vm.borrow().height as i32)
            .content(&content)
            .build();

        {
            let command_tx_for_hide = command_tx_outer.clone();
            window.connect_hide(move |_| {
                let _ = command_tx_for_hide.send(CoreCommand::SetMeteringEnabled(false));
            });
        }

        {
            let command_tx_for_show = command_tx_outer.clone();
            window.connect_show(move |_| {
                let _ = command_tx_for_show.send(CoreCommand::SetMeteringEnabled(true));
            });
        }

        {
            let command_tx_for_unmap = command_tx_outer.clone();
            window.connect_unmap(move |_| {
                let _ = command_tx_for_unmap.send(CoreCommand::SetMeteringEnabled(false));
            });
        }

        {
            let command_tx_for_map = command_tx_outer.clone();
            window.connect_map(move |_| {
                let _ = command_tx_for_map.send(CoreCommand::SetMeteringEnabled(true));
            });
        }

        {
            let command_tx_for_close = command_tx_outer.clone();
            window.connect_close_request(move |window| {
                let _ = command_tx_for_close.send(CoreCommand::SetMeteringEnabled(false));
                window.hide();
                gtk::glib::Propagation::Stop
            });
        }

        let vm = vm.clone();
        let mixer_model_for_events = mixer_model.clone();
        let event_rx = event_rx_outer.clone();
        let window_for_events = window.clone();
        let command_tx_for_events = command_tx_outer.clone();
        let last_metering_enabled = Rc::new(Cell::new(None::<bool>));
        let last_metering_enabled_for_events = last_metering_enabled.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(event) = event_rx.try_recv() {
                if matches!(event, CoreEvent::ToggleWindowRequested) {
                    if window_for_events.is_visible() {
                        let _ = command_tx_for_events.send(CoreCommand::SetMeteringEnabled(false));
                        last_metering_enabled_for_events.set(Some(false));
                        window_for_events.hide();
                    } else {
                        window_for_events.present();
                        let _ = command_tx_for_events.send(CoreCommand::SetMeteringEnabled(true));
                        last_metering_enabled_for_events.set(Some(true));
                    }
                    continue;
                }

                vm.borrow_mut().apply_core_event(&event);
                if let Ok(mut state) = mixer_model_for_events.lock() {
                    state.apply_event(&event);
                }
            }

            let desired_metering = should_metering_be_enabled(
                window_for_events.is_visible(),
                window_for_events.is_active(),
            );
            if last_metering_enabled_for_events.get() != Some(desired_metering) {
                let _ =
                    command_tx_for_events.send(CoreCommand::SetMeteringEnabled(desired_metering));
                last_metering_enabled_for_events.set(Some(desired_metering));
            }

            gtk::glib::ControlFlow::Continue
        });

        window.present();
        let _ = command_tx_outer.send(CoreCommand::SetMeteringEnabled(true));
    });

    app.run();
    Ok(())
}

fn install_mixer_css(palette: Option<&Palette>) {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };

    let main = palette.and_then(|p| parse_hex_color(&p.main));
    let mic = palette.and_then(|p| parse_hex_color(&p.mic));
    let game = palette.and_then(|p| parse_hex_color(&p.game));
    let media = palette.and_then(|p| parse_hex_color(&p.media));
    let chat = palette.and_then(|p| parse_hex_color(&p.chat));
    let aux = palette.and_then(|p| parse_hex_color(&p.aux));

    let slider_main = color_or_theme(main, "alpha(@window_fg_color, 1.00)", 1.00);
    let slider_mic = color_or_theme(mic, "alpha(@accent_color, 1.00)", 1.00);
    let slider_game = color_or_theme(game, "alpha(@success_color, 1.00)", 1.00);
    let slider_media = color_or_theme(media, "alpha(@error_color, 1.00)", 1.00);
    let slider_chat = color_or_theme(chat, "alpha(@accent_color, 1.00)", 1.00);
    let slider_aux = color_or_theme(aux, "alpha(@warning_color, 1.00)", 1.00);

    let chip_main_bg = color_or_theme(main, "alpha(@window_fg_color, 0.24)", 0.28);
    let chip_mic_bg = color_or_theme(mic, "alpha(@accent_color, 0.24)", 0.28);
    let chip_game_bg = color_or_theme(game, "alpha(@success_color, 0.24)", 0.28);
    let chip_media_bg = color_or_theme(media, "alpha(@error_color, 0.24)", 0.28);
    let chip_chat_bg = color_or_theme(chat, "alpha(@accent_color, 0.24)", 0.28);
    let chip_aux_bg = color_or_theme(aux, "alpha(@warning_color, 0.24)", 0.28);

    let chip_main_border = color_or_theme(main, "alpha(@window_fg_color, 0.76)", 0.80);
    let chip_mic_border = color_or_theme(mic, "alpha(@accent_color, 0.76)", 0.80);
    let chip_game_border = color_or_theme(game, "alpha(@success_color, 0.76)", 0.80);
    let chip_media_border = color_or_theme(media, "alpha(@error_color, 0.76)", 0.80);
    let chip_chat_border = color_or_theme(chat, "alpha(@accent_color, 0.76)", 0.80);
    let chip_aux_border = color_or_theme(aux, "alpha(@warning_color, 0.76)", 0.80);

    let css = format!(
        r#"
    .channel-surface {{
        border-radius: 14px;
        padding: 8px;
        background-color: alpha(@window_fg_color, 0.03);
    }}

    .device-label {{
        color: alpha(@window_fg_color, 0.95);
        font-weight: 600;
    }}

    .device-dropdown {{
        color: alpha(@window_fg_color, 0.98);
    }}

    .brand-badge {{
        margin-left: 4px;
    }}

    .brand-mark {{
        font-size: 1.18em;
    }}

    .brand-name {{
        font-weight: 700;
        color: alpha(@window_fg_color, 0.94);
    }}

    .chip-drop-zone {{
        border-radius: 12px;
        min-height: 92px;
        padding: 4px;
        background-color: alpha(@window_fg_color, 0.06);
        border: none;
        box-shadow: none;
    }}

    .chip-drop-zone-hover {{
        background-color: alpha(@accent_color, 0.16);
        border: 1px solid alpha(@accent_color, 0.36);
    }}

    .chip-grid flowboxchild {{
        padding: 0;
        margin: 0;
        background: transparent;
        border: none;
    }}

    .chip-grid flowboxchild:hover,
    .chip-grid flowboxchild:selected,
    .chip-grid flowboxchild:focus {{
        background: transparent;
        border: none;
        box-shadow: none;
    }}

    .chip-drop-zone-spacer {{
        min-height: 136px;
        background-color: alpha(@window_fg_color, 0.03);
        border-radius: 12px;
    }}

    .slider-main highlight {{ background: {slider_main}; }}
    .slider-mic highlight {{ background: {slider_mic}; }}
    .slider-game highlight {{ background: {slider_game}; }}
    .slider-media highlight {{ background: {slider_media}; }}
    .slider-chat highlight {{ background: {slider_chat}; }}
    .slider-aux highlight {{ background: {slider_aux}; }}

    .slider-meter {{
        min-width: 6px;
        opacity: 0.95;
        background: transparent;
        margin: 0;
        padding: 0;
    }}

    .slider-meter trough {{
        background: transparent;
        border: none;
        padding: 0;
        margin: 0;
        min-height: 0;
        min-width: 6px;
    }}

    .meter-main trough progress {{ background: {slider_main}; border-radius: 0; min-width: 6px; min-height: 0; margin: 0; padding: 0; }}
    .meter-mic trough progress {{ background: {slider_mic}; border-radius: 0; min-width: 6px; min-height: 0; margin: 0; padding: 0; }}
    .meter-game trough progress {{ background: {slider_game}; border-radius: 0; min-width: 6px; min-height: 0; margin: 0; padding: 0; }}
    .meter-media trough progress {{ background: {slider_media}; border-radius: 0; min-width: 6px; min-height: 0; margin: 0; padding: 0; }}
    .meter-chat trough progress {{ background: {slider_chat}; border-radius: 0; min-width: 6px; min-height: 0; margin: 0; padding: 0; }}
    .meter-aux trough progress {{ background: {slider_aux}; border-radius: 0; min-width: 6px; min-height: 0; margin: 0; padding: 0; }}

    .chip-main {{
        background-color: {chip_main_bg};
        border: 1px solid {chip_main_border};
        border-radius: 8px;
    }}
    .chip-mic {{
        background-color: {chip_mic_bg};
        border: 1px solid {chip_mic_border};
        border-radius: 8px;
    }}
    .chip-game {{
        background-color: {chip_game_bg};
        border: 1px solid {chip_game_border};
        border-radius: 8px;
    }}
    .chip-media {{
        background-color: {chip_media_bg};
        border: 1px solid {chip_media_border};
        border-radius: 8px;
    }}
    .chip-chat {{
        background-color: {chip_chat_bg};
        border: 1px solid {chip_chat_border};
        border-radius: 8px;
    }}
    .chip-aux {{
        background-color: {chip_aux_bg};
        border: 1px solid {chip_aux_border};
        border-radius: 8px;
    }}

    .chip-button {{
        padding: 1px 4px;
        min-height: 0;
        font-size: 0.86em;
        border-radius: 8px;
        background-image: none;
        box-shadow: none;
    }}

    .chip-button:hover {{
        background-image: none;
        box-shadow: none;
        border-color: alpha(@window_fg_color, 0.42);
    }}

    .chip-text,
    .chip-status,
    .chip-main label,
    .chip-mic label,
    .chip-game label,
    .chip-media label,
    .chip-chat label,
    .chip-aux label {{
        color: alpha(@window_fg_color, 0.98);
        font-weight: 600;
    }}
    "#
    );

    let provider = gtk::CssProvider::new();
    provider.load_from_data(&css);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn build_brand_logo() -> gtk::Image {
    let candidates = [
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/venturi-logo.svg"),
        PathBuf::from("/app/share/icons/hicolor/scalable/apps/org.venturi.Venturi.svg"),
    ];

    for candidate in candidates {
        if candidate.exists() {
            let image = gtk::Image::from_file(candidate);
            image.set_pixel_size(28);
            return image;
        }
    }

    let image = gtk::Image::from_icon_name("org.venturi.Venturi");
    image.set_pixel_size(28);
    image
}

fn color_or_theme(rgb: Option<(u8, u8, u8)>, fallback: &str, alpha: f32) -> String {
    match rgb {
        Some((r, g, b)) => format!("rgba({r}, {g}, {b}, {alpha:.2})"),
        None => fallback.to_string(),
    }
}

fn parse_hex_color(raw: &str) -> Option<(u8, u8, u8)> {
    let hex = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    if !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }

    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            Some((r, g, b))
        }
        3 => {
            let mut chars = hex.chars();
            let r = chars.next()?;
            let g = chars.next()?;
            let b = chars.next()?;

            let rr = u8::from_str_radix(&format!("{r}{r}"), 16).ok()?;
            let gg = u8::from_str_radix(&format!("{g}{g}"), 16).ok()?;
            let bb = u8::from_str_radix(&format!("{b}{b}"), 16).ok()?;
            Some((rr, gg, bb))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::config::schema::State;
    use crate::core::messages::Channel;
    use crate::gui::channel_strip::ChannelStrip;
    use crate::gui::mixer_tab::MixerTab;

    use super::{parse_hex_color, should_metering_be_enabled, ui_selected_device_from_config};

    #[test]
    fn parses_six_digit_hex_colors() {
        assert_eq!(parse_hex_color("#8BA4B0"), Some((139, 164, 176)));
    }

    #[test]
    fn parses_three_digit_hex_colors() {
        assert_eq!(parse_hex_color("#abc"), Some((170, 187, 204)));
    }

    #[test]
    fn rejects_invalid_hex_colors() {
        assert_eq!(parse_hex_color("not-a-color"), None);
        assert_eq!(parse_hex_color("#abcd"), None);
    }

    #[test]
    fn maps_default_device_to_ui_selection() {
        assert_eq!(
            ui_selected_device_from_config("default"),
            Some("Default".to_string())
        );
        assert_eq!(
            ui_selected_device_from_config("DEFAULT"),
            Some("Default".to_string())
        );
        assert_eq!(
            ui_selected_device_from_config("alsa_output.foo"),
            Some("alsa_output.foo".to_string())
        );
        assert_eq!(ui_selected_device_from_config(""), None);
    }

    #[test]
    fn enables_metering_when_window_is_visible() {
        assert!(should_metering_be_enabled(true, true));
        assert!(should_metering_be_enabled(true, false));
        assert!(!should_metering_be_enabled(false, true));
        assert!(!should_metering_be_enabled(false, false));
    }

    #[test]
    fn applies_persisted_strip_values_to_mixer() {
        let mut mixer = MixerTab::new();
        let persisted = State::default();

        super::apply_persisted_strip_values(&mut mixer, &persisted);

        let main = mixer
            .strips
            .get(&Channel::Main)
            .cloned()
            .unwrap_or_else(|| ChannelStrip::new(Channel::Main, "🔊", "Main"));
        assert!((main.volume_linear - persisted.volumes.main).abs() < 0.0001);
        assert_eq!(main.muted, persisted.muted.main);
    }
}
