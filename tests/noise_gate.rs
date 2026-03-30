use venturi::audio::noise_gate::{GateConfig, process_buffer};

#[test]
fn gate_closes_below_threshold_and_opens_above() {
    let cfg = GateConfig {
        threshold_db: -20.0,
        attack_ms: 1,
        release_ms: 10,
    };

    let low = vec![0.01; 8];
    let high = vec![0.5; 8];

    let out_low = process_buffer(&low, cfg);
    let out_high = process_buffer(&high, cfg);

    assert!(out_low.iter().all(|s| s.abs() < 0.01));
    assert!(out_high.iter().any(|s| *s > 0.1));
}
