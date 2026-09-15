use crate::core::messages::{DeviceEntry, DeviceKind};

pub fn fallback_to_default_device() -> &'static str {
    "Default"
}

/// Pick the hardware source that should feed the virtual mic.
///
/// A specific selection always wins. For "Default" we use the system default
/// source unless it is one of Venturi's own nodes (the user typically sets
/// `Venturi-VirtualMic` as their system default, and remapping the virtual mic
/// onto itself produces a dead mic). In that case fall back to the first real
/// input device PipeWire knows about.
pub(crate) fn resolve_virtual_mic_master(
    selected_input: Option<&str>,
    default_source: Option<&str>,
    input_devices: &[DeviceEntry],
    virtual_source_name: &str,
) -> Option<String> {
    if let Some(name) = selected_input
        && !name.is_empty()
        && !name.eq_ignore_ascii_case(fallback_to_default_device())
    {
        return Some(name.to_string());
    }

    if let Some(default) = default_source
        && !default.is_empty()
        && !is_venturi_node(default, virtual_source_name)
    {
        return Some(default.to_string());
    }

    input_devices
        .iter()
        .find(|device| {
            device.kind == DeviceKind::Input && !is_venturi_node(&device.id, virtual_source_name)
        })
        .map(|device| device.id.clone())
}

fn is_venturi_node(name: &str, virtual_source_name: &str) -> bool {
    name.eq_ignore_ascii_case(virtual_source_name)
        || name.starts_with("Venturi-")
        || name.starts_with("input.Venturi-")
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

#[cfg(test)]
mod tests {
    use super::resolve_virtual_mic_master;
    use crate::core::messages::{DeviceEntry, DeviceKind};

    const VIRTUAL_MIC: &str = "Venturi-VirtualMic";

    fn device(kind: DeviceKind, id: &str) -> DeviceEntry {
        DeviceEntry {
            kind,
            id: id.to_string(),
            label: id.to_string(),
        }
    }

    #[test]
    fn specific_input_selection_wins_over_default_source() {
        let master = resolve_virtual_mic_master(
            Some("alsa_input.headset"),
            Some("alsa_input.webcam"),
            &[],
            VIRTUAL_MIC,
        );

        assert_eq!(master.as_deref(), Some("alsa_input.headset"));
    }

    #[test]
    fn default_selection_uses_system_default_source() {
        let master = resolve_virtual_mic_master(
            Some("Default"),
            Some("alsa_input.webcam"),
            &[device(DeviceKind::Input, "alsa_input.headset")],
            VIRTUAL_MIC,
        );

        assert_eq!(master.as_deref(), Some("alsa_input.webcam"));
    }

    #[test]
    fn default_selection_skips_venturi_virtual_mic_as_system_default() {
        let devices = [
            device(DeviceKind::Output, "alsa_output.speakers"),
            device(DeviceKind::Input, "alsa_input.headset"),
            device(DeviceKind::Input, "alsa_input.webcam"),
        ];

        let master =
            resolve_virtual_mic_master(Some("Default"), Some(VIRTUAL_MIC), &devices, VIRTUAL_MIC);

        assert_eq!(master.as_deref(), Some("alsa_input.headset"));
    }

    #[test]
    fn default_selection_skips_other_venturi_nodes_and_no_input_returns_none() {
        let devices = [
            device(DeviceKind::Output, "alsa_output.speakers"),
            device(DeviceKind::Input, "Venturi-Output.monitor"),
        ];

        let master =
            resolve_virtual_mic_master(None, Some("Venturi-Output.monitor"), &devices, VIRTUAL_MIC);

        assert_eq!(master, None);
    }
}
