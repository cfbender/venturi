use crate::core::messages::{DeviceEntry, DeviceKind};

pub fn fallback_to_default_device() -> &'static str {
    "Default"
}

pub(crate) fn resolve_selected_input_name(
    selected_input: Option<&str>,
) -> Result<Option<String>, String> {
    match selected_input {
        Some(name) if !name.is_empty() && name != fallback_to_default_device() => {
            Ok(Some(name.to_string()))
        }
        _ => crate::core::pipewire_backend::current_default_source_name(),
    }
}

pub(crate) fn config_device_value(device: &str) -> String {
    if device.eq_ignore_ascii_case(fallback_to_default_device()) {
        "default".to_string()
    } else {
        device.to_string()
    }
}

pub(crate) fn resolve_output_loopback_target(
    device: &str,
    default_sink: Option<&str>,
    main_output_name: &str,
) -> Option<String> {
    if !device.eq_ignore_ascii_case(fallback_to_default_device()) {
        return Some(device.to_string());
    }

    default_sink
        .filter(|name| !name.eq_ignore_ascii_case(main_output_name))
        .map(ToOwned::to_owned)
}

pub(crate) fn should_skip_output_device_reconcile(
    current_selection: Option<&str>,
    requested_device: &str,
    force: bool,
) -> bool {
    !force && current_selection == Some(requested_device)
}

pub(crate) fn selected_device_available(
    devices: &[DeviceEntry],
    kind: DeviceKind,
    selected: Option<&str>,
) -> bool {
    let Some(selected) = selected else {
        return false;
    };

    selected.eq_ignore_ascii_case(fallback_to_default_device())
        || devices
            .iter()
            .any(|device| device.kind == kind && device.id == selected)
}
