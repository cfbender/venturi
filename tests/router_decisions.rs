use std::collections::BTreeMap;

use venturi::core::messages::Channel;
use venturi::core::router::{
    build_metadata_legacy_target_args, build_metadata_target_args, channel_node_name,
    resolve_input_target, resolve_output_target,
};

#[test]
fn maps_channel_to_expected_node_name() {
    assert_eq!(channel_node_name(Channel::Game), "Venturi-Game");
    assert_eq!(channel_node_name(Channel::Main), "Venturi-Output");
}

#[test]
fn resolves_output_target_from_selected_device_name() {
    let mut outputs = BTreeMap::new();
    outputs.insert("Headphones".to_string(), 51_u32);

    assert_eq!(
        resolve_output_target(Some("Headphones"), &outputs),
        "51".to_string()
    );
    assert_eq!(
        resolve_output_target(Some("Default"), &outputs),
        "@DEFAULT_AUDIO_SINK@".to_string()
    );
    assert_eq!(
        resolve_output_target(Some("Unknown"), &outputs),
        "@DEFAULT_AUDIO_SINK@".to_string()
    );
}

#[test]
fn resolves_input_target_from_selected_device_name() {
    let mut inputs = BTreeMap::new();
    inputs.insert("Cam Link 4K Analog Stereo".to_string(), 63_u32);

    assert_eq!(
        resolve_input_target(Some("Cam Link 4K Analog Stereo"), &inputs),
        "63".to_string()
    );
    assert_eq!(
        resolve_input_target(Some("Default"), &inputs),
        "@DEFAULT_AUDIO_SOURCE@".to_string()
    );
    assert_eq!(
        resolve_input_target(None, &inputs),
        "@DEFAULT_AUDIO_SOURCE@".to_string()
    );
}

#[test]
fn builds_metadata_target_args_for_stream_move() {
    assert_eq!(
        build_metadata_target_args(77, Channel::Chat),
        vec![
            "77".to_string(),
            "target.object".to_string(),
            "Venturi-Chat".to_string(),
        ]
    );
}

#[test]
fn builds_metadata_legacy_target_args_for_stream_move() {
    assert_eq!(
        build_metadata_legacy_target_args(77, Channel::Chat),
        vec![
            "77".to_string(),
            "target.node".to_string(),
            "Venturi-Chat".to_string(),
        ]
    );
}
