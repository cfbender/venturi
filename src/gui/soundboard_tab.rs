use std::path::Path;
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;

use crate::config::persistence::{Paths, load_config, save_config};
use crate::config::schema::SoundPad;
use crate::core::messages::CoreCommand;

const SOUND_PAD_COUNT: usize = 20;
const EMPTY_PAD_NAME: &str = "Add";
const EMPTY_PAD_ICON: &str = "➕";
const DEFAULT_PAD_ICON: &str = "🎵";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundboardPad {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub image: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PadAction {
    Rename(String),
    SetIcon(String),
    SetImage(Option<String>),
    ChangeFile(Option<String>),
    Remove,
}

#[derive(Debug, Clone, Default)]
pub struct SoundboardTab {
    pub pads: Vec<SoundboardPad>,
    transport_playing: bool,
}

fn normalize_optional_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn default_pad(id: u32) -> SoundboardPad {
    SoundboardPad {
        id,
        name: EMPTY_PAD_NAME.to_string(),
        icon: EMPTY_PAD_ICON.to_string(),
        image: None,
        file: None,
    }
}

impl SoundboardTab {
    pub fn from_config(pads: &[SoundPad]) -> Self {
        let mut tab = Self {
            pads: pads
                .iter()
                .enumerate()
                .map(|(index, pad)| SoundboardPad {
                    id: index as u32,
                    name: pad.name.clone(),
                    icon: pad.icon.clone(),
                    image: pad.image.clone(),
                    file: Some(pad.file.clone()),
                })
                .collect(),
            transport_playing: false,
        };
        tab.ensure_empty_slots(SOUND_PAD_COUNT);
        tab
    }

    pub fn ensure_empty_slots(&mut self, target: usize) {
        while self.pads.len() < target {
            self.pads.push(default_pad(self.pads.len() as u32));
        }
    }

    pub fn apply_action(&mut self, pad_id: u32, action: PadAction) {
        if let Some(pad) = self.pads.iter_mut().find(|p| p.id == pad_id) {
            match action {
                PadAction::Rename(name) => {
                    pad.name = if name.trim().is_empty() {
                        EMPTY_PAD_NAME.to_string()
                    } else {
                        name
                    };
                }
                PadAction::SetIcon(icon) => {
                    pad.icon = if icon.trim().is_empty() {
                        DEFAULT_PAD_ICON.to_string()
                    } else {
                        icon
                    };
                }
                PadAction::SetImage(path) => pad.image = path,
                PadAction::ChangeFile(path) => pad.file = path,
                PadAction::Remove => {
                    *pad = default_pad(pad_id);
                }
            }
        }
    }

    pub fn configured_pads(&self) -> Vec<SoundPad> {
        self.pads
            .iter()
            .filter_map(|pad| {
                let file = normalize_optional_text(pad.file.as_deref().unwrap_or_default())?;
                Some(SoundPad {
                    name: if pad.name.trim().is_empty() {
                        format!("Pad {}", pad.id + 1)
                    } else {
                        pad.name.clone()
                    },
                    file,
                    icon: if pad.icon.trim().is_empty() {
                        DEFAULT_PAD_ICON.to_string()
                    } else {
                        pad.icon.clone()
                    },
                    image: normalize_optional_text(pad.image.as_deref().unwrap_or_default()),
                })
            })
            .collect()
    }

    pub fn on_soundboard_state(&mut self, playing: bool) {
        self.set_transport_state(playing);
    }

    fn set_transport_state(&mut self, playing: bool) {
        self.transport_playing = playing;
    }
}

pub fn persist_soundboard_to_config(
    config_file: &Path,
    pads: &[SoundboardPad],
) -> Result<(), String> {
    let config_dir = config_file
        .parent()
        .ok_or_else(|| "config path has no parent directory".to_string())?
        .to_path_buf();

    std::fs::create_dir_all(&config_dir).map_err(|err| err.to_string())?;

    let paths = Paths {
        config_dir,
        state_dir: Paths::resolve().state_dir,
    };

    let mut config = load_config(&paths);
    let tab = SoundboardTab {
        pads: pads.to_vec(),
        transport_playing: false,
    };
    config.soundboard.pads = tab.configured_pads();
    save_config(&paths, &config)
}

fn read_pad(model: &Arc<Mutex<SoundboardTab>>, pad_id: u32) -> Option<SoundboardPad> {
    model
        .lock()
        .ok()
        .and_then(|state| state.pads.iter().find(|pad| pad.id == pad_id).cloned())
}

fn persist_soundboard_model(model: &Arc<Mutex<SoundboardTab>>, config_path: &str) {
    let pads = match model.lock() {
        Ok(state) => state.pads.clone(),
        Err(_) => return,
    };
    let _ = persist_soundboard_to_config(Path::new(config_path), &pads);
}

fn refresh_pad_button_visual(button: &gtk::Button, pad: &SoundboardPad) {
    use gtk::prelude::*;

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);

    if let Some(image_path) = pad
        .image
        .as_deref()
        .and_then(normalize_optional_text)
        .as_deref()
    {
        let image = gtk::Image::from_file(image_path);
        image.set_pixel_size(20);
        row.append(&image);
    } else {
        row.append(&gtk::Label::new(Some(if pad.icon.trim().is_empty() {
            DEFAULT_PAD_ICON
        } else {
            &pad.icon
        })));
    }

    let label = gtk::Label::new(Some(&pad.name));
    label.set_xalign(0.0);
    row.append(&label);

    button.set_child(Some(&row));
    button.set_hexpand(true);
    button.set_vexpand(true);
    button.add_css_class("soundboard-pad-button");

    if normalize_optional_text(pad.file.as_deref().unwrap_or_default()).is_some() {
        button.remove_css_class("flat");
        button.set_tooltip_text(Some("Click to play, right-click to edit"));
    } else {
        button.add_css_class("flat");
        button.set_tooltip_text(Some("Click to configure this soundboard pad"));
    }
}

fn refresh_pad_preview_button(preview_button: &gtk::Button, pad: &SoundboardPad) {
    use gtk::prelude::*;

    let has_file = normalize_optional_text(pad.file.as_deref().unwrap_or_default()).is_some();
    preview_button.set_sensitive(has_file);
    preview_button.add_css_class("soundboard-preview-button");
    if has_file {
        preview_button.set_tooltip_text(Some("Preview on output only"));
    } else {
        preview_button.set_tooltip_text(Some("Configure this pad to enable preview"));
    }
}

fn choose_file_into_entry(anchor: &gtk::Button, title: &str, mime_type: &str, entry: &gtk::Entry) {
    use gtk::prelude::*;

    let mut chooser_builder = gtk::FileChooserNative::builder()
        .title(title)
        .action(gtk::FileChooserAction::Open)
        .accept_label("Select")
        .cancel_label("Cancel");

    if let Some(window) = anchor
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
    {
        chooser_builder = chooser_builder.transient_for(&window);
    }

    let chooser = chooser_builder.build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(title));
    filter.add_mime_type(mime_type);
    chooser.add_filter(&filter);

    let entry_for_response = entry.clone();
    chooser.connect_response(move |chooser, response| {
        if response == gtk::ResponseType::Accept
            && let Some(path) = chooser.file().and_then(|file| file.path())
        {
            entry_for_response.set_text(path.to_string_lossy().as_ref());
        }
        chooser.destroy();
    });
    chooser.show();
}

fn open_pad_editor_dialog(
    anchor: &gtk::Button,
    pad_id: u32,
    model: Arc<Mutex<SoundboardTab>>,
    config_path: String,
    preview_button: gtk::Button,
) {
    use gtk::prelude::*;

    let Some(pad) = read_pad(&model, pad_id) else {
        return;
    };

    let dialog = gtk::Dialog::builder()
        .title("Configure Soundboard Pad")
        .modal(true)
        .build();

    if let Some(window) = anchor
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok())
    {
        dialog.set_transient_for(Some(&window));
    }

    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog.add_button("Clear", gtk::ResponseType::Other(1));
    dialog.add_button("Save", gtk::ResponseType::Accept);

    let content = dialog.content_area();
    let form = gtk::Grid::new();
    form.set_column_spacing(8);
    form.set_row_spacing(8);
    form.set_margin_top(8);
    form.set_margin_bottom(8);
    form.set_margin_start(8);
    form.set_margin_end(8);

    let name_entry = gtk::Entry::new();
    name_entry.set_text(&pad.name);
    let icon_entry = gtk::Entry::new();
    icon_entry.set_text(&pad.icon);
    let file_entry = gtk::Entry::new();
    file_entry.set_hexpand(true);
    file_entry.set_text(pad.file.as_deref().unwrap_or_default());
    let image_entry = gtk::Entry::new();
    image_entry.set_hexpand(true);
    image_entry.set_text(pad.image.as_deref().unwrap_or_default());

    let pick_audio = gtk::Button::with_label("Choose audio…");
    let pick_image = gtk::Button::with_label("Choose image…");

    {
        let file_entry = file_entry.clone();
        let anchor = anchor.clone();
        pick_audio.connect_clicked(move |_| {
            choose_file_into_entry(&anchor, "Choose sound file", "audio/*", &file_entry);
        });
    }

    {
        let image_entry = image_entry.clone();
        let anchor = anchor.clone();
        pick_image.connect_clicked(move |_| {
            choose_file_into_entry(&anchor, "Choose image", "image/*", &image_entry);
        });
    }

    form.attach(&gtk::Label::new(Some("Name")), 0, 0, 1, 1);
    form.attach(&name_entry, 1, 0, 2, 1);
    form.attach(&gtk::Label::new(Some("Emoji")), 0, 1, 1, 1);
    form.attach(&icon_entry, 1, 1, 2, 1);
    form.attach(&gtk::Label::new(Some("Audio file")), 0, 2, 1, 1);
    form.attach(&file_entry, 1, 2, 1, 1);
    form.attach(&pick_audio, 2, 2, 1, 1);
    form.attach(&gtk::Label::new(Some("Image (optional)")), 0, 3, 1, 1);
    form.attach(&image_entry, 1, 3, 1, 1);
    form.attach(&pick_image, 2, 3, 1, 1);

    content.append(&form);

    let pad_button = anchor.clone();
    dialog.connect_response(move |dialog, response| {
        match response {
            gtk::ResponseType::Accept => {
                let file = normalize_optional_text(file_entry.text().as_ref());
                if let Ok(mut state) = model.lock() {
                    if file.is_none() {
                        state.apply_action(pad_id, PadAction::Remove);
                    } else {
                        state
                            .apply_action(pad_id, PadAction::Rename(name_entry.text().to_string()));
                        state.apply_action(
                            pad_id,
                            PadAction::SetIcon(icon_entry.text().to_string()),
                        );
                        state.apply_action(
                            pad_id,
                            PadAction::SetImage(normalize_optional_text(
                                image_entry.text().as_ref(),
                            )),
                        );
                        state.apply_action(pad_id, PadAction::ChangeFile(file));
                    }
                }
                if let Some(updated_pad) = read_pad(&model, pad_id) {
                    refresh_pad_button_visual(&pad_button, &updated_pad);
                    refresh_pad_preview_button(&preview_button, &updated_pad);
                }
                persist_soundboard_model(&model, &config_path);
            }
            gtk::ResponseType::Other(1) => {
                if let Ok(mut state) = model.lock() {
                    state.apply_action(pad_id, PadAction::Remove);
                }
                if let Some(updated_pad) = read_pad(&model, pad_id) {
                    refresh_pad_button_visual(&pad_button, &updated_pad);
                    refresh_pad_preview_button(&preview_button, &updated_pad);
                }
                persist_soundboard_model(&model, &config_path);
            }
            _ => {}
        }

        dialog.close();
    });

    dialog.present();
}

pub fn build_soundboard_widget(
    model: Arc<Mutex<SoundboardTab>>,
    command_tx: Sender<CoreCommand>,
    config_path: String,
) -> gtk::Box {
    use gtk::prelude::*;

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_margin_top(14);
    root.set_margin_start(14);
    root.set_margin_end(14);
    root.set_margin_bottom(12);

    let hint = gtk::Label::new(Some(
        "Play sends to mic + output, Preview sends to output only. Right-click to edit.",
    ));
    hint.set_xalign(0.0);
    hint.add_css_class("dim-label");
    hint.add_css_class("soundboard-hint");
    root.append(&hint);

    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_hexpand(true);
    grid.set_vexpand(true);
    grid.set_column_homogeneous(true);
    grid.set_row_homogeneous(true);
    grid.add_css_class("soundboard-grid");

    {
        let mut state = model.lock().expect("soundboard lock");
        state.ensure_empty_slots(SOUND_PAD_COUNT);
    }

    let pads = model.lock().expect("soundboard lock").pads.clone();
    for (index, pad) in pads.into_iter().enumerate() {
        let button = gtk::Button::new();
        let preview_button = gtk::Button::builder()
            .icon_name("audio-volume-high-symbolic")
            .valign(gtk::Align::Start)
            .halign(gtk::Align::End)
            .margin_top(6)
            .margin_end(6)
            .build();

        refresh_pad_button_visual(&button, &pad);
        refresh_pad_preview_button(&preview_button, &pad);

        {
            let model_for_click = model.clone();
            let tx_for_click = command_tx.clone();
            let config_path_for_click = config_path.clone();
            let button_for_click = button.clone();
            let preview_for_click = preview_button.clone();
            button.connect_clicked(move |_| {
                let Some(current_pad) = read_pad(&model_for_click, pad.id) else {
                    return;
                };

                if let Some(file) =
                    normalize_optional_text(current_pad.file.as_deref().unwrap_or_default())
                {
                    let _ = tx_for_click.send(CoreCommand::typed_play_sound(current_pad.id, file));
                } else {
                    open_pad_editor_dialog(
                        &button_for_click,
                        current_pad.id,
                        model_for_click.clone(),
                        config_path_for_click.clone(),
                        preview_for_click.clone(),
                    );
                }
            });
        }

        {
            let model_for_preview = model.clone();
            let tx_for_preview = command_tx.clone();
            let config_path_for_preview = config_path.clone();
            let button_for_preview = button.clone();
            let preview_for_preview = preview_button.clone();
            preview_button.connect_clicked(move |_| {
                let Some(current_pad) = read_pad(&model_for_preview, pad.id) else {
                    return;
                };

                if let Some(file) =
                    normalize_optional_text(current_pad.file.as_deref().unwrap_or_default())
                {
                    let _ =
                        tx_for_preview.send(CoreCommand::typed_preview_sound(current_pad.id, file));
                } else {
                    open_pad_editor_dialog(
                        &button_for_preview,
                        current_pad.id,
                        model_for_preview.clone(),
                        config_path_for_preview.clone(),
                        preview_for_preview.clone(),
                    );
                }
            });
        }

        {
            let model_for_secondary = model.clone();
            let config_path_for_secondary = config_path.clone();
            let button_for_secondary = button.clone();
            let preview_for_secondary = preview_button.clone();
            let right_click = gtk::GestureClick::new();
            right_click.set_button(3);
            right_click.connect_pressed(move |_, _, _, _| {
                open_pad_editor_dialog(
                    &button_for_secondary,
                    pad.id,
                    model_for_secondary.clone(),
                    config_path_for_secondary.clone(),
                    preview_for_secondary.clone(),
                );
            });
            button.add_controller(right_click);
        }

        let pad_controls = gtk::Overlay::new();
        pad_controls.set_hexpand(true);
        pad_controls.set_vexpand(true);
        pad_controls.set_size_request(0, 92);
        pad_controls.set_child(Some(&button));
        pad_controls.add_overlay(&preview_button);

        grid.attach(&pad_controls, (index % 5) as i32, (index / 5) as i32, 1, 1);
    }

    root.append(&grid);
    root
}

#[cfg(test)]
mod tests {
    use super::{EMPTY_PAD_NAME, SOUND_PAD_COUNT, SoundboardPad, SoundboardTab, default_pad};
    use crate::config::schema::SoundPad;

    #[test]
    fn loads_soundboard_tab_from_config_and_keeps_empty_slots() {
        let config_pads = vec![
            SoundPad {
                name: "Airhorn".to_string(),
                file: "/tmp/airhorn.wav".to_string(),
                icon: "📣".to_string(),
                image: None,
            },
            SoundPad {
                name: "Ding".to_string(),
                file: "/tmp/ding.wav".to_string(),
                icon: "🔔".to_string(),
                image: Some("/tmp/ding.png".to_string()),
            },
        ];

        let tab = SoundboardTab::from_config(&config_pads);
        assert_eq!(tab.pads.len(), SOUND_PAD_COUNT);
        assert_eq!(tab.pads[0].name, "Airhorn");
        assert_eq!(tab.pads[1].image.as_deref(), Some("/tmp/ding.png"));
        assert_eq!(tab.pads[19], default_pad(19));
    }

    #[test]
    fn configured_pads_omit_empty_slots() {
        let mut tab = SoundboardTab {
            pads: vec![
                SoundboardPad {
                    id: 0,
                    name: "Airhorn".to_string(),
                    icon: "📣".to_string(),
                    image: None,
                    file: Some("/tmp/airhorn.wav".to_string()),
                },
                SoundboardPad {
                    id: 1,
                    name: EMPTY_PAD_NAME.to_string(),
                    icon: "➕".to_string(),
                    image: None,
                    file: None,
                },
            ],
            transport_playing: false,
        };
        tab.ensure_empty_slots(SOUND_PAD_COUNT);

        let configured = tab.configured_pads();
        assert_eq!(configured.len(), 1);
        assert_eq!(configured[0].name, "Airhorn");
        assert_eq!(configured[0].file, "/tmp/airhorn.wav");
    }
}
