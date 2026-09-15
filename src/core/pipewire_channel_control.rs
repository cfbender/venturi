use crate::core::messages::Channel;
use crate::core::pipewire_backend::{read_wpctl_volume, run_wpctl_checked};
use crate::core::pipewire_discovery::Snapshot;
use crate::core::snapshot_ops::{channel_node_id, channel_volume_from_snapshot};

/// Volume deltas below this threshold are treated as "already applied" so we
/// don't spam `wpctl` while a slider is being dragged over an unchanged value.
const VOLUME_EPSILON: f32 = 0.005;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelControlTargets<'a> {
    pub virtual_input_source_name: &'a str,
    pub main_output_sink_name: &'a str,
}

/// Resolve the PipeWire node id that owns a Venturi channel bus.
///
/// Channel controls only ever target Venturi's own virtual nodes. When the
/// node is not present in the snapshot we refuse to act instead of falling
/// back to `@DEFAULT_AUDIO_SINK@`/`@DEFAULT_AUDIO_SOURCE@`, which would mute
/// or change the volume of whatever hardware device the user has selected.
fn channel_target(
    channel: Channel,
    snapshot: &Snapshot,
    targets: ChannelControlTargets<'_>,
) -> Result<String, String> {
    channel_node_id(
        snapshot,
        channel,
        targets.main_output_sink_name,
        targets.virtual_input_source_name,
    )
    .map(|id| id.to_string())
    .ok_or_else(|| format!("{channel:?} bus is not available in PipeWire yet"))
}

fn volume_needs_update(current: Option<f32>, requested: f32) -> bool {
    current.is_none_or(|current| (current - requested).abs() >= VOLUME_EPSILON)
}

/// Apply `volume` to the channel's node and return the volume PipeWire
/// reports afterwards (or `volume` if the readback fails).
pub(crate) fn apply_channel_volume(
    channel: Channel,
    volume: f32,
    snapshot: &Snapshot,
    targets: ChannelControlTargets<'_>,
) -> Result<f32, String> {
    let target = channel_target(channel, snapshot, targets)?;
    let current = channel_volume_from_snapshot(
        snapshot,
        channel,
        targets.main_output_sink_name,
        targets.virtual_input_source_name,
    );
    if !volume_needs_update(current, volume) {
        return Ok(current.unwrap_or(volume));
    }

    let args = vec!["set-volume".to_string(), target.clone(), volume.to_string()];
    run_wpctl_checked(&args)?;
    Ok(read_wpctl_volume(&target).unwrap_or(volume))
}

/// Mute or unmute the channel's node. Always issues the command so a stale
/// local view of the mute state can never turn a user's click into a no-op.
pub(crate) fn apply_channel_mute(
    channel: Channel,
    muted: bool,
    snapshot: &Snapshot,
    targets: ChannelControlTargets<'_>,
) -> Result<(), String> {
    let target = channel_target(channel, snapshot, targets)?;
    let value = if muted { "1" } else { "0" };
    let args = vec!["set-mute".to_string(), target, value.to_string()];
    run_wpctl_checked(&args)
}

#[cfg(test)]
mod tests {
    use crate::core::messages::Channel;
    use crate::core::pipewire_discovery::Snapshot;

    const TARGETS: super::ChannelControlTargets<'static> = super::ChannelControlTargets {
        virtual_input_source_name: "Venturi-VirtualMic",
        main_output_sink_name: "Venturi-Output",
    };

    #[test]
    fn channel_target_resolves_media_mix_sink_id() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Media".to_string(), 412);

        let target = super::channel_target(Channel::Media, &snapshot, TARGETS);

        assert_eq!(target, Ok("412".to_string()));
    }

    #[test]
    fn channel_target_resolves_main_and_mic_to_venturi_nodes_only() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("Venturi-Output".to_string(), 128);
        snapshot
            .output_ids
            .insert("alsa_output.usb".to_string(), 129);
        snapshot
            .input_ids
            .insert("Venturi-VirtualMic".to_string(), 281);
        snapshot.input_ids.insert("alsa_input.usb".to_string(), 282);

        assert_eq!(
            super::channel_target(Channel::Main, &snapshot, TARGETS),
            Ok("128".to_string())
        );
        assert_eq!(
            super::channel_target(Channel::Mic, &snapshot, TARGETS),
            Ok("281".to_string())
        );
    }

    #[test]
    fn channel_target_never_falls_back_to_default_devices() {
        let mut snapshot = Snapshot::default();
        snapshot
            .output_ids
            .insert("alsa_output.usb".to_string(), 129);
        snapshot.input_ids.insert("alsa_input.usb".to_string(), 282);

        assert!(super::channel_target(Channel::Main, &snapshot, TARGETS).is_err());
        assert!(super::channel_target(Channel::Mic, &snapshot, TARGETS).is_err());
        assert!(super::channel_target(Channel::Chat, &snapshot, TARGETS).is_err());
    }

    #[test]
    fn volume_update_skipped_only_within_epsilon() {
        assert!(!super::volume_needs_update(Some(0.500), 0.503));
        assert!(super::volume_needs_update(Some(0.500), 0.506));
        assert!(super::volume_needs_update(None, 0.5));
    }

    #[test]
    fn mute_on_missing_bus_reports_error_instead_of_touching_defaults() {
        let snapshot = Snapshot::default();

        let result = super::apply_channel_mute(Channel::Main, true, &snapshot, TARGETS);

        assert!(result.is_err());
    }
}
