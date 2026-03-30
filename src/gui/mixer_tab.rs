use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::messages::{Channel, CoreCommand, CoreEvent};
use crate::gui::app_chip::{AppChip, ChipStatus, DndPayload, build_chip_widget};
use crate::gui::channel_strip::{ChannelStrip, build_strip_widget};
use gtk::prelude::*;

pub const NO_DEVICES_FOUND: &str = "No devices found";

#[derive(Debug, Clone, Default)]
pub struct DeviceListModel {
    pub output_devices: Vec<String>,
    pub input_devices: Vec<String>,
    pub selected_output: Option<String>,
    pub selected_input: Option<String>,
}

impl DeviceListModel {
    pub fn output_label(&self) -> &str {
        self.selected_output.as_deref().unwrap_or(NO_DEVICES_FOUND)
    }

    pub fn input_label(&self) -> &str {
        self.selected_input.as_deref().unwrap_or(NO_DEVICES_FOUND)
    }

    pub fn set_selected_output(&mut self, selected: String) {
        self.selected_output = Some(selected);
    }

    pub fn set_selected_input(&mut self, selected: String) {
        self.selected_input = Some(selected);
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

        self.selected_output = self
            .selected_output
            .clone()
            .filter(|sel| outputs.iter().any(|d| d == sel))
            .or_else(|| outputs.first().cloned());
        self.selected_input = self
            .selected_input
            .clone()
            .filter(|sel| inputs.iter().any(|d| d == sel))
            .or_else(|| inputs.first().cloned());

        self.output_devices = outputs;
        self.input_devices = inputs;
    }

    pub fn reset_to_default_on_disconnect(&mut self) {
        self.output_devices = vec!["Default".to_string()];
        self.input_devices = vec!["Default".to_string()];
        self.selected_output = Some("Default".to_string());
        self.selected_input = Some("Default".to_string());
    }
}

#[derive(Debug, Clone)]
pub struct MixerTab {
    pub strips: BTreeMap<Channel, ChannelStrip>,
    pub chips: BTreeMap<Channel, Vec<AppChip>>,
    pub devices: DeviceListModel,
    pub banner: Option<String>,
    pub toast: Option<String>,
    ui_dirty: Arc<AtomicBool>,
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
            devices: DeviceListModel {
                output_devices: vec!["Default".to_string()],
                input_devices: vec!["Default".to_string()],
                selected_output: Some("Default".to_string()),
                selected_input: Some("Default".to_string()),
            },
            banner: None,
            toast: None,
            ui_dirty: Arc::new(AtomicBool::new(true)),
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
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::StreamRemoved(id) => {
                for chips in self.chips.values_mut() {
                    chips.retain(|chip| chip.stream_id != *id);
                }
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::DevicesChanged(devices) => {
                self.devices.set_from_devices_changed(devices);
                if self.devices.output_devices.is_empty() && self.devices.input_devices.is_empty() {
                    self.toast = Some("No audio devices found".to_string());
                }
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::Error(msg) => {
                self.banner = Some(msg.clone());
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn on_device_disconnect(&mut self) {
        self.devices.reset_to_default_on_disconnect();
        self.toast = Some("Device disconnected. Reset to Default.".to_string());
        self.ui_dirty.store(true, Ordering::Relaxed);
    }

    pub fn mark_ui_dirty(&self) {
        self.ui_dirty.store(true, Ordering::Relaxed);
    }

    pub fn take_ui_dirty(&self) -> bool {
        self.ui_dirty.swap(false, Ordering::Relaxed)
    }
}

impl Default for MixerTab {
    fn default() -> Self {
        Self::new()
    }
}

fn chip_drop_zone_class(channel: Channel) -> Option<&'static str> {
    match channel {
        Channel::Game => Some("chip-drop-zone-game"),
        Channel::Media => Some("chip-drop-zone-media"),
        Channel::Chat => Some("chip-drop-zone-chat"),
        Channel::Aux => Some("chip-drop-zone-aux"),
        Channel::Main | Channel::Mic => None,
    }
}

pub fn build_mixer_widget(
    model: Arc<Mutex<MixerTab>>,
    command_tx: crossbeam_channel::Sender<CoreCommand>,
) -> gtk::Box {
    const EMPTY_STRS: [&str; 0] = [];

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let output_label = gtk::Label::new(Some("Output"));
    let input_label = gtk::Label::new(Some("Input"));
    output_label.add_css_class("device-label");
    input_label.add_css_class("device-label");

    let out_model = gtk::StringList::new(EMPTY_STRS.as_slice());
    let in_model = gtk::StringList::new(EMPTY_STRS.as_slice());

    {
        let state = model.lock().expect("mixer lock");
        for d in &state.devices.output_devices {
            out_model.append(&friendly_device_label(d));
        }
        for d in &state.devices.input_devices {
            in_model.append(&friendly_device_label(d));
        }
        if state.devices.output_devices.is_empty() {
            out_model.append(NO_DEVICES_FOUND);
        }
        if state.devices.input_devices.is_empty() {
            in_model.append(NO_DEVICES_FOUND);
        }
    }

    let output_dropdown = gtk::DropDown::builder().model(&out_model).build();
    let input_dropdown = gtk::DropDown::builder().model(&in_model).build();
    output_dropdown.add_css_class("device-dropdown");
    input_dropdown.add_css_class("device-dropdown");

    {
        let tx = command_tx.clone();
        let model = model.clone();
        output_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            if let Ok(mut state) = model.try_lock()
                && let Some(chosen) = state.devices.output_devices.get(idx).cloned()
                && state.devices.selected_output.as_deref() != Some(chosen.as_str())
            {
                state.devices.set_selected_output(chosen.clone());
                state.mark_ui_dirty();
                let _ = tx.send(CoreCommand::SetOutputDevice(chosen));
            }
        });
    }

    {
        let tx = command_tx.clone();
        let model = model.clone();
        input_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            if let Ok(mut state) = model.try_lock()
                && let Some(chosen) = state.devices.input_devices.get(idx).cloned()
                && state.devices.selected_input.as_deref() != Some(chosen.as_str())
            {
                state.devices.set_selected_input(chosen.clone());
                state.mark_ui_dirty();
                let _ = tx.send(CoreCommand::SetInputDevice(chosen));
            }
        });
    }

    top.append(&output_label);
    top.append(&output_dropdown);
    top.append(&input_label);
    top.append(&input_dropdown);

    let banner = gtk::Label::new(None);
    banner.add_css_class("error");
    {
        let state = model.lock().expect("mixer lock");
        banner.set_text(state.banner.as_deref().unwrap_or(""));
    }

    let channels_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    channels_row.set_hexpand(true);
    channels_row.set_vexpand(true);
    channels_row.set_valign(gtk::Align::Fill);
    channels_row.set_homogeneous(true);
    let mut chip_lists: BTreeMap<Channel, gtk::Box> = BTreeMap::new();

    let channels = [
        Channel::Main,
        Channel::Mic,
        Channel::Game,
        Channel::Media,
        Channel::Chat,
        Channel::Aux,
    ];

    for channel in channels {
        let strip = {
            let state = model.lock().expect("mixer lock");
            state
                .strips
                .get(&channel)
                .cloned()
                .unwrap_or_else(|| ChannelStrip::new(channel, "🔊", "Channel"))
        };
        let channel_col = gtk::Box::new(gtk::Orientation::Vertical, 8);
        channel_col.set_hexpand(true);
        channel_col.set_vexpand(true);
        channel_col.set_valign(gtk::Align::Fill);
        channel_col.add_css_class("channel-surface");
        channel_col.append(&build_strip_widget(strip, command_tx.clone()));

        if let Some(section_class) = chip_drop_zone_class(channel) {
            let zone_shell = gtk::Box::new(gtk::Orientation::Vertical, 6);
            zone_shell.set_hexpand(true);
            zone_shell.set_vexpand(true);

            let chip_list = gtk::Box::new(gtk::Orientation::Vertical, 4);
            chip_list.add_css_class("chip-drop-zone");
            chip_list.add_css_class(section_class);
            chip_list.set_hexpand(true);
            chip_list.set_vexpand(true);
            zone_shell.append(&chip_list);
            chip_lists.insert(channel, chip_list.clone());

            {
                let state = model.lock().expect("mixer lock");
                let chips = state.chips.get(&channel).cloned().unwrap_or_default();
                for chip in chips {
                    chip_list.append(&build_chip_widget(&chip));
                }
            }

            let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
            {
                let tx = command_tx.clone();
                let model = model.clone();
                drop.connect_drop(move |_, value, _, _| {
                    if let Ok(raw) = value.get::<String>()
                        && let Some(payload) = DndPayload::decode(&raw)
                    {
                        let _ = tx.send(CoreCommand::MoveStream {
                            stream_id: payload.stream_id,
                            channel,
                        });
                        if let Ok(mut state) = model.try_lock() {
                            for chips in state.chips.values_mut() {
                                if let Some(idx) =
                                    chips.iter().position(|c| c.stream_id == payload.stream_id)
                                {
                                    let mut chip = chips.remove(idx);
                                    chip.channel = channel;
                                    chip.status = ChipStatus::Idle;
                                    state.chips.entry(channel).or_default().push(chip);
                                    state.mark_ui_dirty();
                                    break;
                                }
                            }
                        }
                        return true;
                    }
                    false
                });
            }
            chip_list.add_controller(drop);

            channel_col.append(&zone_shell);
        }
        channels_row.append(&channel_col);
    }

    root.append(&top);
    root.append(&banner);
    root.append(&channels_row);

    let model_for_refresh = model.clone();
    let out_model = out_model.clone();
    let in_model = in_model.clone();
    let mut last_banner = String::new();
    let mut last_out_devices: Vec<String> = Vec::new();
    let mut last_in_devices: Vec<String> = Vec::new();
    let mut last_out_selected: Option<u32> = None;
    let mut last_in_selected: Option<u32> = None;
    let mut last_chips_snapshot: BTreeMap<Channel, Vec<AppChip>> = BTreeMap::new();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(350), move || {
        if let Ok(state) = model_for_refresh.try_lock() {
            if !state.take_ui_dirty() {
                return gtk::glib::ControlFlow::Continue;
            }

            let banner_text = state.banner.clone().unwrap_or_default();
            let out_devices = if state.devices.output_devices.is_empty() {
                vec![NO_DEVICES_FOUND.to_string()]
            } else {
                state.devices.output_devices.clone()
            };
            let in_devices = if state.devices.input_devices.is_empty() {
                vec![NO_DEVICES_FOUND.to_string()]
            } else {
                state.devices.input_devices.clone()
            };
            let selected_out = state
                .devices
                .selected_output
                .as_ref()
                .and_then(|sel| state.devices.output_devices.iter().position(|d| d == sel));
            let selected_in = state
                .devices
                .selected_input
                .as_ref()
                .and_then(|sel| state.devices.input_devices.iter().position(|d| d == sel));
            let chips_snapshot = state.chips.clone();
            drop(state);

            if banner_text != last_banner {
                banner.set_text(&banner_text);
                last_banner = banner_text;
            }

            if out_devices != last_out_devices {
                out_model.splice(0, out_model.n_items(), &EMPTY_STRS);
                for dev in &out_devices {
                    out_model.append(&friendly_device_label(dev));
                }
                last_out_devices = out_devices;
            }

            if in_devices != last_in_devices {
                in_model.splice(0, in_model.n_items(), &EMPTY_STRS);
                for dev in &in_devices {
                    in_model.append(&friendly_device_label(dev));
                }
                last_in_devices = in_devices;
            }

            let next_out_selected = selected_out.map(|idx| idx as u32);
            if next_out_selected != last_out_selected {
                if let Some(idx) = next_out_selected
                    && output_dropdown.selected() != idx
                {
                    output_dropdown.set_selected(idx);
                }
                last_out_selected = next_out_selected;
            }

            let next_in_selected = selected_in.map(|idx| idx as u32);
            if next_in_selected != last_in_selected {
                if let Some(idx) = next_in_selected
                    && input_dropdown.selected() != idx
                {
                    input_dropdown.set_selected(idx);
                }
                last_in_selected = next_in_selected;
            }

            if chips_snapshot != last_chips_snapshot {
                for (channel, list_box) in &chip_lists {
                    while let Some(child) = list_box.first_child() {
                        list_box.remove(&child);
                    }
                    let chips = chips_snapshot.get(channel).cloned().unwrap_or_default();
                    for chip in chips {
                        list_box.append(&build_chip_widget(&chip));
                    }
                }
                last_chips_snapshot = chips_snapshot;
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    root
}

fn friendly_device_label(raw: &str) -> String {
    if raw == NO_DEVICES_FOUND {
        return raw.to_string();
    }

    raw.replace("alsa_output.", "")
        .replace("alsa_input.", "")
        .replace(".analog-stereo", "")
        .replace(".mono-fallback", "")
        .replace("_", " ")
}
