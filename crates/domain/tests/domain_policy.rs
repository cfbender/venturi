use std::collections::HashSet;

use venturi_domain::{
    channel_node_name, collision_safe_name, decay_peak, Channel, DeviceKind, RoutePlan,
    StableDeviceId,
};

#[test]
fn channel_node_name_maps_main_and_mic() {
    assert_eq!(channel_node_name(Channel::Main), "Venturi-Output");
    assert_eq!(channel_node_name(Channel::Mic), "Venturi-Mic");
}

#[test]
fn decay_peak_trends_from_previous_toward_current_over_time() {
    let previous = 1.0;
    let current = 0.2;

    let short = decay_peak(previous, current, 30);
    let long = decay_peak(previous, current, 250);

    assert!(short < previous);
    assert!(short > current);
    assert!(long < short);
    assert!(long > current);
}

#[test]
fn collision_safe_name_adds_suffix_when_name_conflicts() {
    let existing = HashSet::from(["drum.wav".to_string(), "drum-1.wav".to_string()]);
    assert_eq!(collision_safe_name(&existing, "drum.wav"), "drum-2.wav");
}

#[test]
fn route_plan_retains_stable_device_ids() {
    let stream = StableDeviceId("stream-1".to_string());
    let target = StableDeviceId("target-2".to_string());
    let plan = RoutePlan {
        stream: stream.clone(),
        target: target.clone(),
    };

    assert_eq!(plan.stream, stream);
    assert_eq!(plan.target, target);
}

#[test]
fn device_kind_debug_includes_output() {
    assert_eq!(format!("{:?}", DeviceKind::Output), "Output");
}
