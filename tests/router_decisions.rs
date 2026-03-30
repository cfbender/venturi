use std::collections::BTreeMap;

use venturi::core::messages::Channel;
use venturi::core::router::{
    RoutingMode, build_device_route_plan, build_fallback_link_commands, build_metadata_target_args,
    channel_node_name, choose_routing_mode, force_link_routing_enabled, resolve_input_target,
    resolve_output_target, routing_mode_from_flag,
};

#[test]
fn chooses_metadata_first_by_default() {
    assert_eq!(choose_routing_mode(false), RoutingMode::MetadataFirst);
    assert_eq!(choose_routing_mode(true), RoutingMode::FallbackLinks);
}

#[test]
fn parses_force_link_routing_flag_values() {
    assert!(!force_link_routing_enabled(None));
    assert!(!force_link_routing_enabled(Some("0")));
    assert!(force_link_routing_enabled(Some("1")));
    assert!(force_link_routing_enabled(Some("TRUE")));
    assert_eq!(
        routing_mode_from_flag(Some("yes")),
        RoutingMode::FallbackLinks
    );
}

#[test]
fn maps_channel_to_expected_node_name() {
    assert_eq!(channel_node_name(Channel::Game), "Venturi-Game");
    assert_eq!(channel_node_name(Channel::Main), "Venturi-Output");
}

#[test]
fn builds_device_route_plan_with_defaults() {
    let plan = build_device_route_plan(None, Some("alsa_input.usb-Blue_Yeti"));
    assert_eq!(plan.output_target, "default");
    assert_eq!(plan.input_target, "alsa_input.usb-Blue_Yeti");
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
            "-n".to_string(),
            "settings".to_string(),
            "77".to_string(),
            "target.node".to_string(),
            "Venturi-Chat".to_string(),
        ]
    );
}

#[test]
fn builds_fallback_link_commands_for_stereo_outputs() {
    assert_eq!(
        build_fallback_link_commands(99, Channel::Game),
        vec![
            vec![
                "--passive".to_string(),
                "99:output_FL".to_string(),
                "Venturi-Game:input_FL".to_string(),
            ],
            vec![
                "--passive".to_string(),
                "99:output_FR".to_string(),
                "Venturi-Game:input_FR".to_string(),
            ],
        ]
    );
}
