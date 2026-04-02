use venturi_application::Channel;
use venturi_domain::StableDeviceId;
use venturi_runtime::test_harness;

#[test]
fn routing_and_device_selection_keep_typed_snapshot_in_sync() {
    let harness = test_harness();

    harness.set(Channel::Main, 0.82);
    harness.set(Channel::Game, 0.37);
    harness.route(42, Channel::Chat);
    harness.select_output(Some(StableDeviceId("alsa_output.pci-1".to_string())));
    harness.select_input(Some(StableDeviceId("alsa_input.usb-2".to_string())));

    let view = harness.snapshot().view();
    assert_eq!(view.volume_for(Channel::Main), Some(0.82));
    assert_eq!(view.volume_for(Channel::Game), Some(0.37));
    assert_eq!(view.stream_channel(42), Some(Channel::Chat));
    assert_eq!(
        view.selected_output(),
        Some(&StableDeviceId("alsa_output.pci-1".to_string()))
    );
    assert_eq!(
        view.selected_input(),
        Some(&StableDeviceId("alsa_input.usb-2".to_string()))
    );
}
