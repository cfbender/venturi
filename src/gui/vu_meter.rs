use std::collections::BTreeMap;

use crate::core::messages::Channel;
use crate::core::meter::{MeterValue, decay_peak};

#[derive(Debug, Clone)]
pub struct VuMeter {
    channels: BTreeMap<Channel, (MeterValue, MeterValue)>,
}

impl VuMeter {
    pub fn new() -> Self {
        Self {
            channels: BTreeMap::new(),
        }
    }

    pub fn update_instant(&mut self, channel: Channel, left: f32, right: f32) {
        let entry = self
            .channels
            .entry(channel)
            .or_insert_with(|| (MeterValue::new(0.0), MeterValue::new(0.0)));
        entry.0.store(left);
        entry.1.store(right);
    }

    pub fn decay_tick(
        &mut self,
        channel: Channel,
        left_target: f32,
        right_target: f32,
        elapsed_ms: u32,
    ) {
        let entry = self
            .channels
            .entry(channel)
            .or_insert_with(|| (MeterValue::new(0.0), MeterValue::new(0.0)));
        let left = decay_peak(entry.0.load(), left_target, elapsed_ms);
        let right = decay_peak(entry.1.load(), right_target, elapsed_ms);
        entry.0.store(left);
        entry.1.store(right);
    }

    pub fn sample(&self, channel: Channel) -> (f32, f32) {
        self.channels
            .get(&channel)
            .map(|(l, r)| (l.load(), r.load()))
            .unwrap_or((0.0, 0.0))
    }
}

impl Default for VuMeter {
    fn default() -> Self {
        Self::new()
    }
}

pub fn build_meter_widget() -> gtk::Box {
    use gtk::prelude::*;

    let container = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    let left = gtk::ProgressBar::new();
    let right = gtk::ProgressBar::new();
    left.set_show_text(false);
    right.set_show_text(false);
    container.append(&left);
    container.append(&right);
    container
}
