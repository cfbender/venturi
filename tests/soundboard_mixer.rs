use std::collections::HashSet;

use venturi::audio::soundboard::{collision_safe_name, managed_soundboard_path, mix_stereo};

#[test]
fn mixes_multiple_tracks_into_single_stereo_buffer() {
    let a = vec![[0.2, 0.2], [0.2, 0.2]];
    let b = vec![[0.3, 0.1], [0.3, 0.1]];
    let mixed = mix_stereo(&[a, b], 2);
    assert_eq!(mixed[0], [0.5, 0.3]);
    assert_eq!(mixed[1], [0.5, 0.3]);
}

#[test]
fn generates_collision_safe_names() {
    let existing = HashSet::from(["airhorn.wav".to_string(), "airhorn-1.wav".to_string()]);
    assert_eq!(
        collision_safe_name(&existing, "applause.wav"),
        "applause.wav"
    );
    assert_eq!(
        collision_safe_name(&existing, "airhorn.wav"),
        "airhorn-2.wav"
    );
}

#[test]
fn builds_managed_soundboard_directory_path() {
    let root = std::path::Path::new("/tmp/venturi");
    let path = managed_soundboard_path(root, "airhorn.wav");
    assert_eq!(
        path,
        std::path::Path::new("/tmp/venturi/soundboard/airhorn.wav")
    );
}
