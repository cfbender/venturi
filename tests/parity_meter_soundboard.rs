use venturi_application::{Channel, MeterSnapshot};
use venturi_runtime::test_harness;

#[test]
fn meter_and_soundboard_updates_are_reflected_in_snapshot_view() {
    let harness = test_harness();

    harness.meter(MeterSnapshot {
        channel: Channel::Mic,
        level: 0.24,
        peak: 0.91,
    });
    harness.play(7, "sounds/airhorn.wav".to_string());

    let before_stop = harness.snapshot().view();
    assert_eq!(before_stop.meter_for(Channel::Mic), Some((0.24, 0.91)));
    assert_eq!(before_stop.playing_file(7), Some("sounds/airhorn.wav"));

    harness.stop(7);

    let after_stop = harness.snapshot().view();
    assert_eq!(after_stop.playing_file(7), None);
}
