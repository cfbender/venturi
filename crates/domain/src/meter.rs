pub fn decay_peak(previous: f32, current: f32, elapsed_ms: u32) -> f32 {
    if current >= previous {
        return current;
    }

    let decay_window_ms = 300.0;
    let step = (elapsed_ms as f32 / decay_window_ms).clamp(0.0, 1.0);
    previous + (current - previous) * step
}
