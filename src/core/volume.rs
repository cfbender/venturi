pub fn slider_to_linear(slider: f32) -> f32 {
    slider.clamp(0.0, 1.0).powi(3) * 1.5
}

pub fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * linear.log10()
    }
}

pub fn apply_mute(volume: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { volume }
}
