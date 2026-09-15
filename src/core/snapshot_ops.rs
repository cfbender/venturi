use std::collections::{BTreeMap, BTreeSet};

use crossbeam_channel::Sender;

use crate::categorizer::rules::classify_with_priority;
use crate::core::messages::{Channel, CoreEvent};
use crate::core::pipewire_discovery::Snapshot;
use crate::core::router::category_mix_output_node_name;

/// Map only Venturi-owned channel node IDs to a Channel.
///
/// This intentionally ignores app stream IDs to preserve strict separation between
/// app-local stream gain and Venturi channel bus gain.
pub(crate) fn node_id_to_channel(
    id: u32,
    snapshot: &Snapshot,
    main_output_name: &str,
    virtual_source_name: &str,
) -> Option<Channel> {
    ALL_CHANNELS.into_iter().find(|&channel| {
        channel_node_id(snapshot, channel, main_output_name, virtual_source_name) == Some(id)
    })
}

pub(crate) const ALL_CHANNELS: [Channel; 6] = [
    Channel::Main,
    Channel::Mic,
    Channel::Game,
    Channel::Media,
    Channel::Chat,
    Channel::Aux,
];

/// Resolve the PipeWire node ID of the Venturi-owned bus that a channel controls.
///
/// Returns `None` when the bus node is not (yet) present in the snapshot. Callers
/// must not fall back to `@DEFAULT_AUDIO_*@` in that case: doing so would apply
/// volume/mute to whatever the user's system default device happens to be.
pub(crate) fn channel_node_id(
    snapshot: &Snapshot,
    channel: Channel,
    main_output_name: &str,
    virtual_source_name: &str,
) -> Option<u32> {
    match channel {
        Channel::Main => snapshot.output_ids.get(main_output_name).copied(),
        Channel::Mic => snapshot.input_ids.get(virtual_source_name).copied(),
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            category_mix_output_id(snapshot, channel)
        }
    }
}

pub(crate) fn upsert_devices(
    devices: &mut Vec<crate::core::messages::DeviceEntry>,
    updates: Vec<crate::core::messages::DeviceEntry>,
) {
    for update in updates {
        if let Some(existing) = devices
            .iter_mut()
            .find(|entry| entry.kind == update.kind && entry.id == update.id)
        {
            *existing = update;
        } else {
            devices.push(update);
        }
    }
}

pub(crate) fn prune_removed_node_ids(
    snapshot: &mut Snapshot,
    removed_ids: &[u32],
    event_tx: &Sender<CoreEvent>,
) {
    if removed_ids.is_empty() {
        return;
    }

    let removed: BTreeSet<u32> = removed_ids.iter().copied().collect();

    for id in &removed {
        if snapshot.streams.remove(id).is_some() {
            let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
        }
        snapshot.volumes.remove(id);
        snapshot.mutes.remove(id);
    }

    snapshot
        .output_ids
        .retain(|_, node_id| !removed.contains(node_id));
    snapshot
        .input_ids
        .retain(|_, node_id| !removed.contains(node_id));
}

pub(crate) fn apply_structural_monitor_delta(
    snapshot: &mut Snapshot,
    partial: Snapshot,
    removed_ids: &[u32],
    structural_ids: &[u32],
    overrides: &BTreeMap<String, Channel>,
    event_tx: &Sender<CoreEvent>,
) {
    let devices_before = snapshot.devices.clone();

    let partial_stream_ids: BTreeSet<u32> = partial.streams.keys().copied().collect();
    let partial_output_ids: BTreeSet<u32> = partial.output_ids.values().copied().collect();
    let partial_input_ids: BTreeSet<u32> = partial.input_ids.values().copied().collect();
    let structural: BTreeSet<u32> = structural_ids.iter().copied().collect();

    for id in &structural {
        if !partial_stream_ids.contains(id) && snapshot.streams.remove(id).is_some() {
            let _ = event_tx.send(CoreEvent::StreamRemoved(*id));
        }
        snapshot.volumes.remove(id);
        snapshot.mutes.remove(id);
    }

    snapshot
        .output_ids
        .retain(|_, node_id| !structural.contains(node_id) || partial_output_ids.contains(node_id));
    snapshot
        .input_ids
        .retain(|_, node_id| !structural.contains(node_id) || partial_input_ids.contains(node_id));

    snapshot.output_ids.extend(partial.output_ids);
    snapshot.input_ids.extend(partial.input_ids);
    snapshot
        .output_meter_targets
        .extend(partial.output_meter_targets);
    snapshot
        .input_meter_targets
        .extend(partial.input_meter_targets);
    snapshot.volumes.extend(partial.volumes);
    snapshot.mutes.extend(partial.mutes);

    for (id, stream_info) in partial.streams {
        if !snapshot.streams.contains_key(&id) {
            let category = classify_with_priority(
                overrides,
                Some(&stream_info.app_key),
                Some(&stream_info.display_name),
                stream_info.media_role.as_deref(),
            );
            let _ = event_tx.send(CoreEvent::StreamAppeared {
                id,
                app_key: stream_info.app_key.clone(),
                name: stream_info.display_name.clone(),
                category,
            });
        }
        snapshot.streams.insert(id, stream_info);
    }

    upsert_devices(&mut snapshot.devices, partial.devices);
    prune_removed_node_ids(snapshot, removed_ids, event_tx);

    let output_node_names: BTreeSet<String> = snapshot.output_ids.keys().cloned().collect();
    let input_node_names: BTreeSet<String> = snapshot.input_ids.keys().cloned().collect();

    snapshot
        .output_meter_targets
        .retain(|node_name, _| output_node_names.contains(node_name));
    snapshot
        .input_meter_targets
        .retain(|node_name, _| input_node_names.contains(node_name));

    snapshot.devices.retain(|device| match device.kind {
        crate::core::messages::DeviceKind::Output => output_node_names.contains(&device.id),
        crate::core::messages::DeviceKind::Input => input_node_names.contains(&device.id),
    });

    if snapshot.devices != devices_before {
        let _ = event_tx.send(CoreEvent::DevicesChanged(snapshot.devices.clone()));
    }
}

pub(crate) fn snapshot_channel_volumes(
    snapshot: &Snapshot,
    main_output_name: &str,
    virtual_source_name: &str,
) -> BTreeMap<Channel, f32> {
    ALL_CHANNELS
        .into_iter()
        .filter_map(|channel| {
            channel_volume_from_snapshot(snapshot, channel, main_output_name, virtual_source_name)
                .map(|v| (channel, v))
        })
        .collect()
}

pub(crate) fn snapshot_channel_mutes(
    snapshot: &Snapshot,
    main_output_name: &str,
    virtual_source_name: &str,
) -> BTreeMap<Channel, bool> {
    ALL_CHANNELS
        .into_iter()
        .filter_map(|channel| {
            channel_mute_from_snapshot(snapshot, channel, main_output_name, virtual_source_name)
                .map(|m| (channel, m))
        })
        .collect()
}

pub(crate) fn channel_volume_from_snapshot(
    snapshot: &Snapshot,
    channel: Channel,
    main_output_name: &str,
    virtual_source_name: &str,
) -> Option<f32> {
    let id = channel_node_id(snapshot, channel, main_output_name, virtual_source_name)?;
    snapshot.volumes.get(&id).copied()
}

pub(crate) fn channel_mute_from_snapshot(
    snapshot: &Snapshot,
    channel: Channel,
    main_output_name: &str,
    virtual_source_name: &str,
) -> Option<bool> {
    let id = channel_node_id(snapshot, channel, main_output_name, virtual_source_name)?;
    snapshot.mutes.get(&id).copied()
}

pub(crate) fn category_mix_output_id(snapshot: &Snapshot, channel: Channel) -> Option<u32> {
    let node_name = category_mix_output_node_name(channel)?;
    snapshot.output_ids.get(node_name).copied()
}

pub(crate) fn apply_snapshot_volume_hint(
    snapshot: &mut Snapshot,
    channel: Channel,
    volume: f32,
    main_output_name: &str,
    virtual_source_name: &str,
) {
    if let Some(id) = channel_node_id(snapshot, channel, main_output_name, virtual_source_name) {
        snapshot.volumes.insert(id, volume);
    }
}

pub(crate) fn apply_snapshot_mute_hint(
    snapshot: &mut Snapshot,
    channel: Channel,
    muted: bool,
    main_output_name: &str,
    virtual_source_name: &str,
) {
    if let Some(id) = channel_node_id(snapshot, channel, main_output_name, virtual_source_name) {
        snapshot.mutes.insert(id, muted);
    }
}

/// Emit the current PipeWire volume and mute state of every Venturi channel bus.
pub(crate) fn emit_snapshot_channel_state(
    snapshot: &Snapshot,
    event_tx: &Sender<CoreEvent>,
    main_output_name: &str,
    virtual_source_name: &str,
) {
    snapshot_channel_volumes(snapshot, main_output_name, virtual_source_name)
        .into_iter()
        .for_each(|(channel, volume)| {
            let _ = event_tx.send(CoreEvent::VolumeChanged(channel, volume));
        });
    snapshot_channel_mutes(snapshot, main_output_name, virtual_source_name)
        .into_iter()
        .for_each(|(channel, muted)| {
            let _ = event_tx.send(CoreEvent::MuteChanged(channel, muted));
        });
}
