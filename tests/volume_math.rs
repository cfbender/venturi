use venturi::core::meter::decay_peak;
use venturi::core::volume::{apply_mute, linear_to_db, slider_to_linear};

#[test]
fn cubic_slider_mapping_matches_expected_points() {
    assert_eq!(slider_to_linear(0.0), 0.0);
    assert!((slider_to_linear(1.0) - 1.5).abs() < 1e-6);
}

#[test]
fn db_conversion_handles_negative_infinity_and_unity() {
    assert_eq!(linear_to_db(0.0), f32::NEG_INFINITY);
    assert!((linear_to_db(1.0) - 0.0).abs() < 1e-6);
    let plus = linear_to_db(1.5);
    assert!(plus > 3.0 && plus < 4.0);
}

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
