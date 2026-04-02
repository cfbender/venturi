#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pad {
    pub id: u32,
    pub name: String,
    pub file: String,
}

pub fn mix_stereo(active_tracks: &[Vec<[f32; 2]>], frames: usize) -> Vec<[f32; 2]> {
    let mut out = vec![[0.0f32, 0.0f32]; frames];
    for track in active_tracks {
        for (idx, sample) in track.iter().enumerate().take(frames) {
            out[idx][0] = (out[idx][0] + sample[0]).clamp(-1.0, 1.0);
            out[idx][1] = (out[idx][1] + sample[1]).clamp(-1.0, 1.0);
        }
    }
    out
}

pub fn managed_soundboard_path(
    config_dir: &std::path::Path,
    original_name: &str,
) -> std::path::PathBuf {
    config_dir.join("soundboard").join(original_name)
}

pub fn collision_safe_name(
    existing: &std::collections::HashSet<String>,
    file_name: &str,
) -> String {
    venturi_domain::collision_safe_name(existing, file_name)
}
