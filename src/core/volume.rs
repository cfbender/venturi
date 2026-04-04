pub fn apply_mute(volume: f32, muted: bool) -> f32 {
    if muted { 0.0 } else { volume }
}
