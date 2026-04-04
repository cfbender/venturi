use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Clone)]
pub struct MeterValue(Arc<AtomicU32>);

impl MeterValue {
    pub fn new(value: f32) -> Self {
        Self(Arc::new(AtomicU32::new(value.to_bits())))
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

pub fn decay_peak(previous: f32, current: f32, elapsed_ms: u32) -> f32 {
    if current >= previous {
        return current;
    }
    let decay_window_ms = 300.0;
    let step = (elapsed_ms as f32 / decay_window_ms).clamp(0.0, 1.0);
    previous + (current - previous) * step
}
