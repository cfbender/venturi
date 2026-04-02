use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::core::messages::{Channel, CoreCommand, CoreEvent};
use crate::core::messages::{DeviceEntry, DeviceKind};
use crate::core::meter::decay_peak;
use crate::gui::app_chip::{AppChip, ChipStatus, DndPayload, build_chip_widget};
use crate::gui::channel_strip::{ChannelStrip, SliderHandle, build_strip_widget_with_meter};
use gtk::prelude::*;

pub const NO_DEVICES_FOUND: &str = "No devices found";

#[derive(Debug, Clone, Default)]
pub struct DeviceListModel {
    pub output_devices: Vec<String>,
    pub input_devices: Vec<String>,
    pub output_labels_by_id: BTreeMap<String, String>,
    pub input_labels_by_id: BTreeMap<String, String>,
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

    pub fn set_from_devices_changed(&mut self, devices: &[DeviceEntry]) {
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();
        let mut output_labels_by_id = BTreeMap::new();
        let mut input_labels_by_id = BTreeMap::new();

        for device in devices {
            match device.kind {
                DeviceKind::Output => {
                    outputs.push(device.id.clone());
                    output_labels_by_id.insert(device.id.clone(), device.label.clone());
                }
                DeviceKind::Input => {
                    inputs.push(device.id.clone());
                    input_labels_by_id.insert(device.id.clone(), device.label.clone());
                }
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
        self.output_labels_by_id = output_labels_by_id;
        self.input_labels_by_id = input_labels_by_id;
    }

    pub fn reset_to_default_on_disconnect(&mut self) {
        self.output_devices = vec!["Default".to_string()];
        self.input_devices = vec!["Default".to_string()];
        self.output_labels_by_id = BTreeMap::new();
        self.input_labels_by_id = BTreeMap::new();
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
    pub levels: BTreeMap<Channel, (f32, f32)>,
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
                output_labels_by_id: BTreeMap::new(),
                input_labels_by_id: BTreeMap::new(),
                selected_output: Some("Default".to_string()),
                selected_input: Some("Default".to_string()),
            },
            banner: None,
            toast: None,
            levels: BTreeMap::new(),
            ui_dirty: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn apply_event(&mut self, event: &CoreEvent) {
        match event {
            CoreEvent::StreamAppeared {
                id,
                app_key,
                name,
                category,
            } => {
                self.upsert_chip_stream(*id, app_key, name, *category);
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::StreamRemoved(id) => {
                self.remove_chip_stream(*id);
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::DevicesChanged(devices) => {
                self.devices.set_from_devices_changed(devices);
                if self.devices.output_devices.is_empty() && self.devices.input_devices.is_empty() {
                    self.toast = Some("No audio devices found".to_string());
                }
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::DeviceSelectionChanged {
                selected_output,
                selected_input,
            } => {
                self.devices.selected_output = selected_output.clone();
                self.devices.selected_input = selected_input.clone();
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::Error(msg) => {
                self.banner = Some(msg.clone());
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::LevelsUpdate(levels) => {
                for (channel, (left, right)) in levels {
                    self.levels.insert(*channel, (*left, *right));
                }
                self.ui_dirty.store(true, Ordering::Relaxed);
            }
            CoreEvent::VolumeChanged(channel, volume) => {
                if let Some(strip) = self.strips.get_mut(channel) {
                    strip.volume_linear = *volume;
                }
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

    fn upsert_chip_stream(&mut self, stream_id: u32, app_key: &str, name: &str, category: Channel) {
        self.remove_chip_stream(stream_id);

        let mut found = None;
        for (channel, chips) in &self.chips {
            if let Some(idx) = chips.iter().position(|chip| chip.app_key == app_key) {
                found = Some((*channel, idx));
                break;
            }
        }

        if let Some((existing_channel, chip_index)) = found {
            let mut chip = self
                .chips
                .get_mut(&existing_channel)
                .expect("existing chip bucket")
                .remove(chip_index);

            if !chip.stream_ids.contains(&stream_id) {
                chip.stream_ids.push(stream_id);
                chip.stream_ids.sort_unstable();
            }
            chip.stream_id = chip.stream_ids.first().copied().unwrap_or(stream_id);
            chip.display_name = name.to_string();
            chip.channel = category;
            self.chips.entry(category).or_default().push(chip);
            self.remove_empty_chip_bucket(existing_channel);
            return;
        }

        let chip = AppChip {
            stream_id,
            stream_ids: vec![stream_id],
            app_key: app_key.to_string(),
            display_name: name.to_string(),
            channel: category,
            status: crate::gui::app_chip::ChipStatus::Idle,
        };
        self.chips.entry(category).or_default().push(chip);
    }

    fn remove_chip_stream(&mut self, stream_id: u32) {
        let mut found = None;
        for (channel, chips) in &self.chips {
            if let Some(idx) = chips
                .iter()
                .position(|chip| chip.stream_ids.contains(&stream_id))
            {
                found = Some((*channel, idx));
                break;
            }
        }

        let Some((channel, chip_index)) = found else {
            return;
        };

        let mut remove_channel = false;
        if let Some(chips) = self.chips.get_mut(&channel)
            && let Some(chip) = chips.get_mut(chip_index)
        {
            chip.stream_ids.retain(|id| *id != stream_id);
            if chip.stream_ids.is_empty() {
                chips.remove(chip_index);
            } else {
                chip.stream_id = chip.stream_ids[0];
            }
            remove_channel = chips.is_empty();
        }

        if remove_channel {
            self.chips.remove(&channel);
        }
    }

    fn move_chip_to_channel(&mut self, stream_id: u32, channel: Channel) -> Vec<u32> {
        let mut found = None;
        for (current_channel, chips) in &self.chips {
            if let Some(idx) = chips.iter().position(|chip| {
                chip.stream_id == stream_id || chip.stream_ids.contains(&stream_id)
            }) {
                found = Some((*current_channel, idx));
                break;
            }
        }

        let Some((current_channel, chip_index)) = found else {
            return Vec::new();
        };

        if current_channel == channel {
            return Vec::new();
        }

        let mut chip = self
            .chips
            .get_mut(&current_channel)
            .expect("existing chip bucket")
            .remove(chip_index);
        let stream_ids = chip.stream_ids.clone();
        chip.channel = channel;
        chip.status = ChipStatus::Idle;
        self.chips.entry(channel).or_default().push(chip);
        self.remove_empty_chip_bucket(current_channel);
        self.mark_ui_dirty();
        stream_ids
    }

    fn remove_empty_chip_bucket(&mut self, channel: Channel) {
        if self.chips.get(&channel).is_some_and(Vec::is_empty) {
            self.chips.remove(&channel);
        }
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
    const METER_UI_TICK_MS: u64 = 33;

    let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_margin_top(14);
    root.set_margin_start(14);
    root.set_margin_end(14);
    root.set_margin_bottom(12);

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
            out_model.append(&display_device_label(&state.devices.output_labels_by_id, d));
        }
        for d in &state.devices.input_devices {
            in_model.append(&display_device_label(&state.devices.input_labels_by_id, d));
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
    let suppress_device_notify = Rc::new(Cell::new(false));

    {
        let tx = command_tx.clone();
        let model = model.clone();
        let suppress_device_notify = suppress_device_notify.clone();
        output_dropdown.connect_selected_notify(move |dd| {
            if suppress_device_notify.get() {
                return;
            }
            let idx = dd.selected() as usize;
            if let Ok(mut state) = model.try_lock()
                && let Some(chosen) = state.devices.output_devices.get(idx).cloned()
                && should_apply_device_selection_change(
                    false,
                    state.devices.selected_output.as_deref(),
                    chosen.as_str(),
                )
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
        let suppress_device_notify = suppress_device_notify.clone();
        input_dropdown.connect_selected_notify(move |dd| {
            if suppress_device_notify.get() {
                return;
            }
            let idx = dd.selected() as usize;
            if let Ok(mut state) = model.try_lock()
                && let Some(chosen) = state.devices.input_devices.get(idx).cloned()
                && should_apply_device_selection_change(
                    false,
                    state.devices.selected_input.as_deref(),
                    chosen.as_str(),
                )
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
    top.set_halign(gtk::Align::Center);

    let top_wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    top_wrap.set_hexpand(true);
    top_wrap.set_halign(gtk::Align::Center);
    top_wrap.set_margin_top(8);
    top_wrap.append(&top);

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
    channels_row.set_margin_start(6);
    channels_row.set_margin_end(6);
    channels_row.set_margin_bottom(6);
    let mut chip_lists: BTreeMap<Channel, gtk::FlowBox> = BTreeMap::new();
    let mut meter_widgets: BTreeMap<Channel, gtk::ProgressBar> = BTreeMap::new();
    let mut slider_widgets: BTreeMap<Channel, SliderHandle> = BTreeMap::new();

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
        let (strip_widget, meter, slider_handle) =
            build_strip_widget_with_meter(strip, command_tx.clone());
        channel_col.append(&strip_widget);
        meter_widgets.insert(channel, meter);
        slider_widgets.insert(channel, slider_handle);

        if let Some(section_class) = chip_drop_zone_class(channel) {
            let zone_shell = gtk::Box::new(gtk::Orientation::Vertical, 6);
            zone_shell.add_css_class("chip-drop-zone");
            zone_shell.add_css_class(section_class);
            zone_shell.set_hexpand(true);
            zone_shell.set_vexpand(false);
            zone_shell.set_margin_top(8);
            zone_shell.set_valign(gtk::Align::End);

            let chip_list = gtk::FlowBox::new();
            chip_list.add_css_class("chip-grid");
            chip_list.set_hexpand(false);
            chip_list.set_vexpand(true);
            chip_list.set_halign(gtk::Align::Center);
            chip_list.set_valign(gtk::Align::Center);
            chip_list.set_selection_mode(gtk::SelectionMode::None);
            chip_list.set_max_children_per_line(2);
            chip_list.set_min_children_per_line(1);
            chip_list.set_column_spacing(6);
            chip_list.set_row_spacing(6);
            zone_shell.append(&chip_list);
            chip_lists.insert(channel, chip_list.clone());

            {
                let state = model.lock().expect("mixer lock");
                let chips = state.chips.get(&channel).cloned().unwrap_or_default();
                for chip in chips {
                    chip_list.insert(&build_chip_widget(&chip), -1);
                }
            }

            let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
            {
                let tx = command_tx.clone();
                let model = model.clone();
                let zone_shell_for_enter = zone_shell.clone();
                let zone_shell_for_leave = zone_shell.clone();
                let zone_shell_for_drop = zone_shell.clone();

                drop.connect_enter(move |_, _, _| {
                    zone_shell_for_enter.add_css_class("chip-drop-zone-hover");
                    gtk::gdk::DragAction::MOVE
                });

                drop.connect_leave(move |_| {
                    zone_shell_for_leave.remove_css_class("chip-drop-zone-hover");
                });

                drop.connect_drop(move |_, value, _, _| {
                    zone_shell_for_drop.remove_css_class("chip-drop-zone-hover");
                    if let Ok(raw) = value.get::<String>()
                        && let Some(payload) = DndPayload::decode(&raw)
                    {
                        let mut moved_stream_ids = Vec::new();
                        if let Ok(mut state) = model.try_lock() {
                            moved_stream_ids =
                                state.move_chip_to_channel(payload.stream_id, channel);
                        }

                        for stream_id in moved_stream_ids {
                            let _ = tx.send(CoreCommand::MoveStream { stream_id, channel });
                        }

                        if payload.origin == channel {
                            // Keep drop feedback feeling responsive when dropping in same zone.
                            if let Ok(state) = model.try_lock() {
                                state.mark_ui_dirty();
                            }
                        }
                        return true;
                    }
                    false
                });
            }
            zone_shell.add_controller(drop);

            channel_col.append(&zone_shell);
        }
        channels_row.append(&channel_col);
    }

    root.append(&top_wrap);
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
    let mut last_meter_levels: BTreeMap<Channel, f32> = BTreeMap::new();
    let mut last_meter_tick = Instant::now();
    gtk::glib::timeout_add_local(
        std::time::Duration::from_millis(METER_UI_TICK_MS),
        move || {
            if let Ok(state) = model_for_refresh.try_lock() {
                let levels_snapshot = state.levels.clone();
                let ui_dirty = state.take_ui_dirty();

                let now = Instant::now();
                let elapsed_ms = now.duration_since(last_meter_tick).as_millis() as u32;
                last_meter_tick = now;
                for channel in [
                    Channel::Main,
                    Channel::Mic,
                    Channel::Game,
                    Channel::Media,
                    Channel::Chat,
                    Channel::Aux,
                ] {
                    let (left, right) =
                        levels_snapshot.get(&channel).copied().unwrap_or((0.0, 0.0));
                    let current = meter_display_level(left.max(right));
                    let previous = *last_meter_levels.get(&channel).unwrap_or(&0.0);
                    let next = decay_peak(previous, current, elapsed_ms);
                    if let Some(widget) = meter_widgets.get(&channel) {
                        widget.set_fraction(next as f64);
                        widget.set_visible(meter_should_be_visible(next));
                    }
                    last_meter_levels.insert(channel, next);
                }

                // Volume sync — update sliders from model state
                for (channel, handle) in &slider_widgets {
                    if handle.is_dragging.get() {
                        continue;
                    }
                    if let Some(strip_data) = state.strips.get(channel) {
                        sync_slider_widget_from_model(handle, strip_data);
                    }
                }

                if !ui_dirty {
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
                let out_labels_by_id = state.devices.output_labels_by_id.clone();
                let in_labels_by_id = state.devices.input_labels_by_id.clone();
                let chips_snapshot = state.chips.clone();
                drop(state);

                if banner_text != last_banner {
                    banner.set_text(&banner_text);
                    last_banner = banner_text;
                }

                if out_devices != last_out_devices {
                    suppress_device_notify.set(true);
                    out_model.splice(0, out_model.n_items(), &EMPTY_STRS);
                    for dev in &out_devices {
                        out_model.append(&display_device_label(&out_labels_by_id, dev));
                    }
                    last_out_devices = out_devices;
                    suppress_device_notify.set(false);
                }

                if in_devices != last_in_devices {
                    suppress_device_notify.set(true);
                    in_model.splice(0, in_model.n_items(), &EMPTY_STRS);
                    for dev in &in_devices {
                        in_model.append(&display_device_label(&in_labels_by_id, dev));
                    }
                    last_in_devices = in_devices;
                    suppress_device_notify.set(false);
                }

                let next_out_selected = selected_out.map(|idx| idx as u32);
                if next_out_selected != last_out_selected {
                    if let Some(idx) = next_out_selected
                        && output_dropdown.selected() != idx
                    {
                        suppress_device_notify.set(true);
                        output_dropdown.set_selected(idx);
                        suppress_device_notify.set(false);
                    }
                    last_out_selected = next_out_selected;
                }

                let next_in_selected = selected_in.map(|idx| idx as u32);
                if next_in_selected != last_in_selected {
                    if let Some(idx) = next_in_selected
                        && input_dropdown.selected() != idx
                    {
                        suppress_device_notify.set(true);
                        input_dropdown.set_selected(idx);
                        suppress_device_notify.set(false);
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
                            list_box.insert(&build_chip_widget(&chip), -1);
                        }
                    }
                    last_chips_snapshot = chips_snapshot;
                }
            }
            gtk::glib::ControlFlow::Continue
        },
    );

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

fn display_device_label(labels_by_id: &BTreeMap<String, String>, raw_id: &str) -> String {
    labels_by_id
        .get(raw_id)
        .cloned()
        .unwrap_or_else(|| friendly_device_label(raw_id))
}

fn meter_display_level(linear_peak: f32) -> f32 {
    linear_peak.clamp(0.0, 1.0).sqrt()
}

fn meter_should_be_visible(level: f32) -> bool {
    level > 0.01
}

fn sync_slider_widget_from_model(handle: &SliderHandle, strip_data: &ChannelStrip) {
    let (next_slider_value, next_label) =
        compute_slider_sync_update(handle.scale.value(), strip_data);
    if let Some(model_volume) = next_slider_value {
        handle.suppress_signal.set(true);
        handle.scale.set_value(model_volume);
        handle.suppress_signal.set(false);
    }

    if handle.value_label.text().as_str() != next_label {
        handle.value_label.set_text(&next_label);
    }
}

fn compute_slider_sync_update(
    current_slider_value: f64,
    strip_data: &ChannelStrip,
) -> (Option<f64>, String) {
    let model_volume = strip_data.volume_linear as f64;
    let next_slider_value =
        ((current_slider_value - model_volume).abs() > 0.005).then_some(model_volume);
    (next_slider_value, strip_data.volume_text())
}

fn should_apply_device_selection_change(
    is_programmatic_update: bool,
    current_selected: Option<&str>,
    chosen: &str,
) -> bool {
    !is_programmatic_update && current_selected != Some(chosen)
}

#[cfg(test)]
mod tests {
    use super::{
        DeviceListModel, MixerTab, compute_slider_sync_update, meter_display_level,
        meter_should_be_visible, should_apply_device_selection_change,
    };
    use crate::core::messages::{Channel, CoreEvent, DeviceEntry, DeviceKind};
    use crate::gui::channel_strip::ChannelStrip;

    #[test]
    fn retains_persisted_selected_ids_when_devices_refresh() {
        let mut model = DeviceListModel {
            selected_output: Some("alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1".to_string()),
            selected_input: Some(
                "alsa_input.usb-Logitech_G735_Gaming_Headset-01.mono-fallback".to_string(),
            ),
            ..DeviceListModel::default()
        };
        let devices = vec![
            DeviceEntry {
                kind: DeviceKind::Output,
                id: "alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1".to_string(),
                label: "Navi 48 HDMI/DP Audio Controller Digital Stereo (HDMI 2)".to_string(),
            },
            DeviceEntry {
                kind: DeviceKind::Input,
                id: "alsa_input.usb-Logitech_G735_Gaming_Headset-01.mono-fallback".to_string(),
                label: "G735 Gaming Headset Mono".to_string(),
            },
        ];

        model.set_from_devices_changed(devices.as_slice());

        assert_eq!(
            model.selected_output.as_deref(),
            Some("alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1")
        );
        assert_eq!(
            model.selected_input.as_deref(),
            Some("alsa_input.usb-Logitech_G735_Gaming_Headset-01.mono-fallback")
        );
    }

    #[test]
    fn meter_display_level_uses_perceptual_curve() {
        assert_eq!(meter_display_level(0.0), 0.0);
        assert!((meter_display_level(0.01) - 0.1).abs() < 0.0001);
        assert!((meter_display_level(0.25) - 0.5).abs() < 0.0001);
        assert_eq!(meter_display_level(1.5), 1.0);
    }

    #[test]
    fn hides_meter_when_signal_is_near_silence() {
        assert!(!meter_should_be_visible(0.0));
        assert!(!meter_should_be_visible(0.009));
        assert!(meter_should_be_visible(0.02));
    }

    #[test]
    fn ignores_programmatic_or_duplicate_device_selection_events() {
        assert!(!should_apply_device_selection_change(
            true,
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo"),
            "alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo",
        ));
        assert!(!should_apply_device_selection_change(
            false,
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo"),
            "alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo",
        ));
        assert!(should_apply_device_selection_change(
            false,
            Some("alsa_output.usb-FIIO_FiiO_K11-01.analog-stereo"),
            "alsa_output.pci-0000_03_00.1.hdmi-stereo-extra1",
        ));
    }

    #[test]
    fn applies_device_selection_changed_event() {
        let mut mixer = MixerTab::default();

        mixer.apply_event(&CoreEvent::DeviceSelectionChanged {
            selected_output: Some("alsa_output.usb-Headset.analog-stereo".to_string()),
            selected_input: Some("alsa_input.usb-Headset.mono-fallback".to_string()),
        });

        assert_eq!(
            mixer.devices.selected_output.as_deref(),
            Some("alsa_output.usb-Headset.analog-stereo")
        );
        assert_eq!(
            mixer.devices.selected_input.as_deref(),
            Some("alsa_input.usb-Headset.mono-fallback")
        );
    }

    #[test]
    fn apply_volume_changed_updates_strip_model() {
        let mut mixer = MixerTab::default();
        mixer.strips.insert(
            Channel::Main,
            ChannelStrip {
                channel: Channel::Main,
                icon: "🔊",
                label: "Main",
                volume_linear: 0.5,
                muted: false,
            },
        );

        mixer.apply_event(&CoreEvent::VolumeChanged(Channel::Main, 0.75));

        let strip = mixer.strips.get(&Channel::Main).unwrap();
        assert!((strip.volume_linear - 0.75).abs() < 0.001);
    }

    #[test]
    fn compute_slider_sync_update_refreshes_label_even_without_value_change() {
        let strip = ChannelStrip {
            channel: Channel::Main,
            icon: "🔊",
            label: "Main",
            volume_linear: 0.26,
            muted: false,
        };

        let (next_slider, next_label) = compute_slider_sync_update(0.26, &strip);

        assert_eq!(next_slider, None);
        assert_eq!(next_label, "26%");
    }

    #[test]
    fn aggregates_streams_with_same_app_key_into_single_chip() {
        let mut mixer = MixerTab::default();

        mixer.apply_event(&CoreEvent::StreamAppeared {
            id: 100,
            app_key: "discord".to_string(),
            name: "Discord".to_string(),
            category: Channel::Chat,
        });
        mixer.apply_event(&CoreEvent::StreamAppeared {
            id: 101,
            app_key: "discord".to_string(),
            name: "Discord".to_string(),
            category: Channel::Chat,
        });

        let chips = mixer.chips.get(&Channel::Chat).expect("chat chips exist");
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].stream_id, 100);
        assert_eq!(chips[0].stream_ids, vec![100, 101]);
    }

    #[test]
    fn stream_removed_keeps_aggregated_chip_until_last_stream_is_gone() {
        let mut mixer = MixerTab::default();

        mixer.apply_event(&CoreEvent::StreamAppeared {
            id: 100,
            app_key: "discord".to_string(),
            name: "Discord".to_string(),
            category: Channel::Chat,
        });
        mixer.apply_event(&CoreEvent::StreamAppeared {
            id: 101,
            app_key: "discord".to_string(),
            name: "Discord".to_string(),
            category: Channel::Chat,
        });

        mixer.apply_event(&CoreEvent::StreamRemoved(100));

        let chips = mixer.chips.get(&Channel::Chat).expect("chat chips exist");
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].stream_id, 101);
        assert_eq!(chips[0].stream_ids, vec![101]);

        mixer.apply_event(&CoreEvent::StreamRemoved(101));

        assert!(!mixer.chips.contains_key(&Channel::Chat));
    }

    #[test]
    fn move_chip_to_channel_returns_all_stream_ids_for_app() {
        let mut mixer = MixerTab::default();

        mixer.apply_event(&CoreEvent::StreamAppeared {
            id: 200,
            app_key: "youtube-music-desktop-app".to_string(),
            name: "YouTube Music".to_string(),
            category: Channel::Media,
        });
        mixer.apply_event(&CoreEvent::StreamAppeared {
            id: 201,
            app_key: "youtube-music-desktop-app".to_string(),
            name: "YouTube Music".to_string(),
            category: Channel::Media,
        });

        let moved_streams = mixer.move_chip_to_channel(200, Channel::Game);

        assert_eq!(moved_streams, vec![200, 201]);
        assert!(!mixer.chips.contains_key(&Channel::Media));
        let game_chips = mixer.chips.get(&Channel::Game).expect("game chips exist");
        assert_eq!(game_chips.len(), 1);
        assert_eq!(game_chips[0].stream_ids, vec![200, 201]);
    }
}
