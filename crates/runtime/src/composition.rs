use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use venturi_application::{Channel, MeterSnapshot, StableDeviceId};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TestSnapshot {
    channel_volume: BTreeMap<u8, f32>,
    stream_channel: BTreeMap<u32, Channel>,
    selected_output: Option<StableDeviceId>,
    selected_input: Option<StableDeviceId>,
    meter_levels: BTreeMap<u8, (f32, f32)>,
    playing_pads: BTreeMap<u32, String>,
}

impl TestSnapshot {
    pub fn view(&self) -> SnapshotView {
        SnapshotView {
            inner: self.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotView {
    inner: TestSnapshot,
}

impl SnapshotView {
    pub fn volume_for(&self, channel: Channel) -> Option<f32> {
        self.inner
            .channel_volume
            .get(&channel_key(channel))
            .copied()
    }

    pub fn stream_channel(&self, stream_id: u32) -> Option<Channel> {
        self.inner.stream_channel.get(&stream_id).copied()
    }

    pub fn selected_output(&self) -> Option<&StableDeviceId> {
        self.inner.selected_output.as_ref()
    }

    pub fn selected_input(&self) -> Option<&StableDeviceId> {
        self.inner.selected_input.as_ref()
    }

    pub fn meter_for(&self, channel: Channel) -> Option<(f32, f32)> {
        self.inner.meter_levels.get(&channel_key(channel)).copied()
    }

    pub fn playing_file(&self, pad_id: u32) -> Option<&str> {
        self.inner.playing_pads.get(&pad_id).map(String::as_str)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RuntimeComposition {
    snapshot: Arc<Mutex<TestSnapshot>>,
}

pub fn test_harness() -> RuntimeComposition {
    RuntimeComposition::default()
}

impl RuntimeComposition {
    pub fn snapshot(&self) -> TestSnapshot {
        self.snapshot
            .lock()
            .expect("composition snapshot lock")
            .clone()
    }

    pub fn set(&self, channel: Channel, value: f32) {
        let mut snapshot = self.snapshot.lock().expect("composition set lock");
        snapshot
            .channel_volume
            .insert(channel_key(channel), value.clamp(0.0, 1.0));
    }

    pub fn route(&self, stream_id: u32, channel: Channel) {
        let mut snapshot = self.snapshot.lock().expect("composition route lock");
        snapshot.stream_channel.insert(stream_id, channel);
    }

    pub fn select_output(&self, output: Option<StableDeviceId>) {
        let mut snapshot = self
            .snapshot
            .lock()
            .expect("composition select output lock");
        snapshot.selected_output = output;
    }

    pub fn select_input(&self, input: Option<StableDeviceId>) {
        let mut snapshot = self.snapshot.lock().expect("composition select input lock");
        snapshot.selected_input = input;
    }

    pub fn meter(&self, meter: MeterSnapshot) {
        let mut snapshot = self.snapshot.lock().expect("composition meter lock");
        snapshot
            .meter_levels
            .insert(channel_key(meter.channel), (meter.level, meter.peak));
    }

    pub fn play(&self, pad_id: u32, file: String) {
        let mut snapshot = self.snapshot.lock().expect("composition play lock");
        snapshot.playing_pads.insert(pad_id, file);
    }

    pub fn stop(&self, pad_id: u32) {
        let mut snapshot = self.snapshot.lock().expect("composition stop lock");
        snapshot.playing_pads.remove(&pad_id);
    }
}

fn channel_key(channel: Channel) -> u8 {
    match channel {
        Channel::Main => 0,
        Channel::Game => 1,
        Channel::Media => 2,
        Channel::Chat => 3,
        Channel::Aux => 4,
        Channel::Mic => 5,
    }
}
