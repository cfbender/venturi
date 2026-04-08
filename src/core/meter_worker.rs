use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;

use crate::core::messages::{Channel, CoreEvent};
use crate::core::pipewire_backend::PwTargetSampler;
use crate::core::pipewire_discovery::Snapshot;

pub(crate) const METER_WORKER_INTERVAL: Duration = Duration::from_millis(33);
pub(crate) const METER_WORKER_IDLE_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const METER_SAMPLE_INTERVAL: Duration = Duration::from_millis(66);

/// Channels whose bus-level meters the meter worker should maintain.
///
/// Each entry maps a mixer [`Channel`] to the PipeWire node name used to look
/// up the numeric `object.serial` in the [`Snapshot`].  The serial is then
/// passed to `pw-record --target <serial>` because `pw-record` cannot resolve
/// sink names — it only matches source names, so using a sink name causes it
/// to silently fall back to the default source (the physical mic).
pub(crate) const BUS_METER_CHANNELS: [(Channel, &str); 6] = [
    (Channel::Main, "Venturi-Output"),
    (Channel::Game, "Venturi-Game"),
    (Channel::Media, "Venturi-Media"),
    (Channel::Chat, "Venturi-Chat"),
    (Channel::Aux, "Venturi-Aux"),
    (Channel::Mic, "Venturi-VirtualMic"),
];

pub(crate) fn compute_level_sample_count(sample_rate_hz: u32, sample_interval: Duration) -> u32 {
    let interval_ms = sample_interval.as_millis() as u64;
    let sample_count = (sample_rate_hz as u64)
        .saturating_mul(interval_ms)
        .saturating_div(1000)
        .max(1);
    sample_count.min(u32::MAX as u64) as u32
}

pub(crate) fn spawn_meter_worker(
    event_tx: Sender<CoreEvent>,
    running: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
    shared_snapshot: Arc<Mutex<Snapshot>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let level_sample_count = compute_level_sample_count(48_000, METER_SAMPLE_INTERVAL);
        let mut last_meter_sample = Instant::now() - METER_SAMPLE_INTERVAL;
        let mut samplers: BTreeMap<Channel, PwTargetSampler> = BTreeMap::new();

        while running.load(Ordering::Relaxed) {
            if !enabled.load(Ordering::Relaxed) {
                samplers.clear();
                std::thread::sleep(METER_WORKER_IDLE_INTERVAL);
                continue;
            }
            if last_meter_sample.elapsed() < METER_SAMPLE_INTERVAL {
                std::thread::sleep(METER_WORKER_INTERVAL);
                continue;
            }
            last_meter_sample = Instant::now();

            // Look up numeric object serials from the current snapshot.  `pw-record --target`
            // only resolves sinks correctly by serial — string node names silently fall back
            // to the default source.
            let (output_targets, input_targets) = {
                if let Ok(snap) = shared_snapshot.try_lock() {
                    (
                        snap.output_meter_targets.clone(),
                        snap.input_meter_targets.clone(),
                    )
                } else {
                    (BTreeMap::new(), BTreeMap::new())
                }
            };

            for (channel, node_name) in &BUS_METER_CHANNELS {
                if !samplers.contains_key(channel) {
                    let serial = output_targets
                        .get(*node_name)
                        .or_else(|| input_targets.get(*node_name));
                    if let Some(&target_serial) = serial {
                        let target_str = target_serial.to_string();
                        if let Ok(sampler) = PwTargetSampler::spawn(&target_str) {
                            samplers.insert(*channel, sampler);
                        }
                    }
                }
            }

            let mut updates = Vec::new();
            let mut failed = Vec::new();

            for (channel, _) in &BUS_METER_CHANNELS {
                let levels = if let Some(sampler) = samplers.get_mut(channel) {
                    match sampler.sample_levels(level_sample_count) {
                        Ok(levels) => levels,
                        Err(_) => {
                            failed.push(*channel);
                            (0.0, 0.0)
                        }
                    }
                } else {
                    (0.0, 0.0)
                };
                updates.push((*channel, levels));
            }

            for channel in failed {
                samplers.remove(&channel);
            }

            let _ = event_tx.send(CoreEvent::LevelsUpdate(updates));
            std::thread::sleep(METER_WORKER_INTERVAL);
        }
    })
}
