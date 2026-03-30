use venturi::core::messages::Channel;
use venturi::core::router::{
    RoutingMode, build_device_route_plan, channel_node_name, choose_routing_mode,
};

#[test]
fn chooses_metadata_first_by_default() {
    assert_eq!(choose_routing_mode(false), RoutingMode::MetadataFirst);
    assert_eq!(choose_routing_mode(true), RoutingMode::FallbackLinks);
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
