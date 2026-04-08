use venturi::core::meter::{apply_mute, decay_peak};

#[test]
fn mute_sets_output_to_zero() {
    assert_eq!(apply_mute(0.75, true), 0.0);
    assert_eq!(apply_mute(0.75, false), 0.75);
}

#[test]
fn meter_decay_tracks_300ms_profile() {
    let previous = 1.0;
    let current = 0.0;
    let after_150 = decay_peak(previous, current, 150);
    let after_300 = decay_peak(previous, current, 300);
    assert!(after_150 > 0.0 && after_150 < 1.0);
    assert!(after_300 <= 0.01);
}
