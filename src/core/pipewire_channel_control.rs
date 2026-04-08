use std::collections::BTreeMap;

use crate::core::messages::Channel;
use crate::core::pipewire_backend::{read_wpctl_volume, run_wpctl, run_wpctl_checked};
use crate::core::pipewire_discovery::Snapshot;
use crate::core::router::{
    category_mix_output_node_name, resolve_input_target, resolve_output_target,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelControlTargets<'a> {
    pub virtual_input_source_name: &'a str,
    pub main_output_sink_name: &'a str,
}

fn category_mix_output_target(channel: Channel, snapshot: &Snapshot) -> Option<String> {
    let node_name = category_mix_output_node_name(channel)?;
    snapshot
        .output_ids
        .get(node_name)
        .copied()
        .map(|id| id.to_string())
}

fn resolve_applied_volume_for_update(
    previous_applied: Option<f32>,
    requested: f32,
    changed: bool,
    set_succeeded: bool,
    readback: Option<f32>,
) -> Option<f32> {
    if !changed {
        return previous_applied;
    }

    if !set_succeeded {
        return None;
    }

    Some(readback.unwrap_or(requested))
}

pub(crate) fn apply_channel_volume(
    channel: Channel,
    volume: f32,
    snapshot: &Snapshot,
    targets: ChannelControlTargets<'_>,
    last_sink_volume_by_target: &mut BTreeMap<String, f32>,
    last_source_volume_by_target: &mut BTreeMap<String, f32>,
) -> Option<f32> {
    let (target, cache) = match channel {
        Channel::Mic => {
            let t =
                resolve_input_target(Some(targets.virtual_input_source_name), &snapshot.input_ids);
            (
                t,
                last_source_volume_by_target as &mut BTreeMap<String, f32>,
            )
        }
        Channel::Main => {
            let t =
                resolve_output_target(Some(targets.main_output_sink_name), &snapshot.output_ids);
            (t, last_sink_volume_by_target as &mut BTreeMap<String, f32>)
        }
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            let t = category_mix_output_target(channel, snapshot)?;
            (t, last_sink_volume_by_target as &mut BTreeMap<String, f32>)
        }
    };

    set_volume_for_target(target, volume, cache)
}

fn set_volume_for_target(
    target: String,
    volume: f32,
    cache: &mut BTreeMap<String, f32>,
) -> Option<f32> {
    let previous_applied = cache.get(&target).copied();
    let changed = previous_applied
        .map(|prev| (prev - volume).abs() >= 0.01)
        .unwrap_or(true);

    let (set_succeeded, readback) = if changed {
        let args = vec!["set-volume".to_string(), target.clone(), volume.to_string()];
        if run_wpctl_checked(&args).is_ok() {
            (true, read_wpctl_volume(&target).ok())
        } else {
            (false, None)
        }
    } else {
        (true, None)
    };

    let applied = resolve_applied_volume_for_update(
        previous_applied,
        volume,
        changed,
        set_succeeded,
        readback,
    );
    if let Some(applied) = applied {
        cache.insert(target, applied);
    }
    applied
}

pub(crate) fn apply_channel_mute(
    channel: Channel,
    muted: bool,
    snapshot: &Snapshot,
    targets: ChannelControlTargets<'_>,
    last_sink_mute_by_target: &mut BTreeMap<String, bool>,
    last_source_mute_by_target: &mut BTreeMap<String, bool>,
) {
    let (target, cache) = match channel {
        Channel::Mic => {
            let t =
                resolve_input_target(Some(targets.virtual_input_source_name), &snapshot.input_ids);
            (
                Some(t),
                last_source_mute_by_target as &mut BTreeMap<String, bool>,
            )
        }
        Channel::Main => {
            let t =
                resolve_output_target(Some(targets.main_output_sink_name), &snapshot.output_ids);
            (
                Some(t),
                last_sink_mute_by_target as &mut BTreeMap<String, bool>,
            )
        }
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            let t = category_mix_output_target(channel, snapshot);
            (t, last_sink_mute_by_target as &mut BTreeMap<String, bool>)
        }
    };

    let Some(target) = target else { return };
    if cache.get(&target) != Some(&muted) {
        let value = if muted { "1" } else { "0" };
        let args = vec!["set-mute".to_string(), target.clone(), value.to_string()];
        run_wpctl(&args);
        cache.insert(target, muted);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::core::messages::Channel;
    use crate::core::pipewire_discovery::Snapshot;

    #[test]
    fn unchanged_volume_uses_cached_applied_value() {
        let applied = super::resolve_applied_volume_for_update(Some(0.98), 1.0, false, true, None);

        assert_eq!(applied, Some(0.98));
    }

    #[test]
    fn failed_set_volume_returns_none() {
        let applied = super::resolve_applied_volume_for_update(Some(0.50), 0.2, true, false, None);

        assert_eq!(applied, None);
    }

    #[test]
    fn category_mix_output_target_resolves_media_sink_id() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Media".to_string(), 412);

        let target = super::category_mix_output_target(Channel::Media, &snapshot);

        assert_eq!(target, Some("412".to_string()));
    }

    #[test]
    fn category_mix_output_target_returns_none_for_main() {
        let snapshot = Snapshot::default();

        let target = super::category_mix_output_target(Channel::Main, &snapshot);

        assert_eq!(target, None);
    }

    #[test]
    fn category_mix_output_target_returns_none_when_sink_missing() {
        let snapshot = Snapshot::default();

        let target = super::category_mix_output_target(Channel::Chat, &snapshot);

        assert_eq!(target, None);
    }

    #[test]
    fn category_mute_updates_mix_sink_target_cache() {
        let mut snapshot = Snapshot::default();
        snapshot.output_ids.insert("Venturi-Media".to_string(), 412);

        let mut sink_mute_by_target = BTreeMap::new();
        let mut source_mute_by_target = BTreeMap::new();

        super::apply_channel_mute(
            Channel::Media,
            true,
            &snapshot,
            super::ChannelControlTargets {
                virtual_input_source_name: "Venturi-VirtualMic",
                main_output_sink_name: "Venturi-Output",
            },
            &mut sink_mute_by_target,
            &mut source_mute_by_target,
        );

        assert_eq!(sink_mute_by_target.get("412"), Some(&true));
    }
}
