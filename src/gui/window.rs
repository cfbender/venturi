use std::time::Duration;
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

use adw::prelude::*;
use crossbeam_channel::{Receiver, Sender};

use crate::app::GuiLauncher;
use crate::config::persistence::{Paths, load_config};
use crate::config::schema::Palette;
use crate::core::messages::{CoreCommand, CoreEvent};
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
        install_mixer_css(config.palette.as_ref());

        let config_path = paths.config_file().display().to_string();
        let vm = Rc::new(RefCell::new(MainWindow::new(
            config_path,
            format!("Venturi {}", env!("CARGO_PKG_VERSION")),
        )));

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

        let vm = vm.clone();
        let mixer_model_for_events = mixer_model.clone();
        let event_rx = event_rx_outer.clone();
        let window_for_events = window.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            while let Ok(event) = event_rx.try_recv() {
                if matches!(event, CoreEvent::ToggleWindowRequested) {
                    if window_for_events.is_visible() {
                        window_for_events.hide();
                    } else {
                        window_for_events.present();
                    }
                    continue;
                }

                vm.borrow_mut().apply_core_event(&event);
                if let Ok(mut state) = mixer_model_for_events.lock() {
                    state.apply_event(&event);
                }
            }
            gtk::glib::ControlFlow::Continue
        });

        window.present();
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

    .chip-drop-zone {{
        border-radius: 12px;
        min-height: 136px;
        padding: 8px;
        background-color: alpha(@window_fg_color, 0.06);
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
    use super::parse_hex_color;

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
}
