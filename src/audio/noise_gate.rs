pub fn apply_threshold(sample: f32, threshold_db: f32) -> f32 {
    let amp = sample.abs();
    let db = if amp <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * amp.log10()
    };
    if db >= threshold_db { sample } else { 0.0 }
}

#[derive(Debug, Clone, Copy)]
pub struct GateConfig {
    pub threshold_db: f32,
    pub attack_ms: u32,
    pub release_ms: u32,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            threshold_db: -40.0,
            attack_ms: 1,
            release_ms: 100,
        }
    }
}

pub fn process_buffer(samples: &[f32], config: GateConfig) -> Vec<f32> {
    let mut envelope = 0.0f32;
    let attack = (1.0f32 / (config.attack_ms.max(1) as f32)).min(1.0);
    let release = (1.0f32 / (config.release_ms.max(1) as f32)).min(1.0);

    samples
        .iter()
        .map(|sample| {
            let target = if apply_threshold(*sample, config.threshold_db) == 0.0 {
                0.0
            } else {
                1.0
            };
            let coeff = if target > envelope { attack } else { release };
            envelope += (target - envelope) * coeff;
            sample * envelope
        })
        .collect()
}
