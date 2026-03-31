use std::time::Duration;

use venturi::app::pump_event;
use venturi::core::messages::{Channel, CoreEvent, DeviceEntry, DeviceKind};
use venturi::core::pipewire_manager::{fallback_to_default_device, reconnect_delay};
use venturi::gui::mixer_tab::NO_DEVICES_FOUND;
use venturi::gui::window::MainWindow;

#[test]
fn reconnect_policy_is_two_seconds() {
    assert_eq!(reconnect_delay(), Duration::from_secs(2));
}

#[test]
fn device_disconnect_resets_to_default_and_toasts() {
    let mut window = MainWindow::new("~/.config/venturi".to_string(), "v0.1".to_string());
    let devices = vec![
        DeviceEntry {
            kind: DeviceKind::Output,
            id: "Headphones".to_string(),
            label: "Headphones".to_string(),
        },
        DeviceEntry {
            kind: DeviceKind::Input,
            id: "Mic".to_string(),
            label: "Mic".to_string(),
        },
    ];
    window
        .mixer
        .devices
        .set_from_devices_changed(devices.as_slice());

    window.mixer.on_device_disconnect();

    assert_eq!(window.mixer.devices.output_label(), "Default");
    assert_eq!(window.mixer.devices.input_label(), "Default");
    assert_eq!(fallback_to_default_device(), "Default");
    assert!(window
        .mixer
        .toast
        .as_deref()
        .is_some_and(|m| m.contains("Reset to Default")));
}

#[test]
fn corrupt_config_emits_reset_toast() {
    let mut window = MainWindow::new("~/.config/venturi".to_string(), "v0.1".to_string());
    window.on_config_corrupt();
    assert_eq!(
        window.mixer.toast.as_deref(),
        Some("Config was reset due to errors.")
    );
}

#[test]
fn no_devices_changed_shows_empty_device_state() {
    let mut window = MainWindow::new("~/.config/venturi".to_string(), "v0.1".to_string());
    pump_event(&mut window, CoreEvent::DevicesChanged(vec![]));
    assert_eq!(window.mixer.devices.output_label(), NO_DEVICES_FOUND);
    assert_eq!(window.mixer.devices.input_label(), NO_DEVICES_FOUND);
}

#[test]
fn stream_events_update_viewmodel_once() {
    let mut window = MainWindow::new("~/.config/venturi".to_string(), "v0.1".to_string());

    pump_event(
        &mut window,
        CoreEvent::StreamAppeared {
            id: 5,
            name: "Discord".to_string(),
            category: Channel::Chat,
        },
    );
    pump_event(&mut window, CoreEvent::StreamRemoved(5));

    let chips = window
        .mixer
        .chips
        .get(&Channel::Chat)
        .cloned()
        .unwrap_or_default();
    assert!(chips.is_empty());
}
