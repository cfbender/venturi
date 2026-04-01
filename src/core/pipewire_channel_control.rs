use std::collections::BTreeMap;

use crate::categorizer::rules::classify_with_priority;
use crate::core::messages::Channel;
use crate::core::pipewire_backend::{read_wpctl_volume, run_wpctl, run_wpctl_checked};
use crate::core::pipewire_discovery::Snapshot;
use crate::core::router::{resolve_input_target, resolve_output_target};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelControlTargets<'a> {
    pub virtual_input_source_name: &'a str,
    pub main_output_sink_name: &'a str,
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
    overrides: &BTreeMap<String, Channel>,
    targets: ChannelControlTargets<'_>,
    last_sink_volume_by_target: &mut BTreeMap<String, f32>,
    last_source_volume_by_target: &mut BTreeMap<String, f32>,
) -> Option<f32> {
    match channel {
        Channel::Mic => {
            let target =
                resolve_input_target(Some(targets.virtual_input_source_name), &snapshot.input_ids);
            let previous_applied = last_source_volume_by_target.get(&target).copied();
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
                last_source_volume_by_target.insert(target, applied);
            }
            applied
        }
        Channel::Main => {
            let target =
                resolve_output_target(Some(targets.main_output_sink_name), &snapshot.output_ids);
            let previous_applied = last_sink_volume_by_target.get(&target).copied();
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
                last_sink_volume_by_target.insert(target, applied);
            }
            applied
        }
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            let mut any_succeeded = false;
            for stream in snapshot.streams.values() {
                let stream_channel = classify_with_priority(
                    overrides,
                    Some(&stream.app_key),
                    Some(&stream.display_name),
                    stream.media_role.as_deref(),
                );
                if stream_channel == channel {
                    let args = vec![
                        "set-volume".to_string(),
                        stream.id.to_string(),
                        volume.to_string(),
                    ];
                    if run_wpctl_checked(&args).is_ok() {
                        any_succeeded = true;
                    }
                }
            }
            if any_succeeded {
                Some(volume)
            } else {
                None
            }
        }
    }
}

pub(crate) fn apply_channel_mute(
    channel: Channel,
    muted: bool,
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, Channel>,
    targets: ChannelControlTargets<'_>,
    last_sink_mute_by_target: &mut BTreeMap<String, bool>,
    last_source_mute_by_target: &mut BTreeMap<String, bool>,
) {
    let value = if muted { "1" } else { "0" };
    match channel {
        Channel::Mic => {
            let target =
                resolve_input_target(Some(targets.virtual_input_source_name), &snapshot.input_ids);
            if last_source_mute_by_target.get(&target) != Some(&muted) {
                let args = vec!["set-mute".to_string(), target.clone(), value.to_string()];
                run_wpctl(&args);
                last_source_mute_by_target.insert(target, muted);
            }
        }
        Channel::Main => {
            let target =
                resolve_output_target(Some(targets.main_output_sink_name), &snapshot.output_ids);
            if last_sink_mute_by_target.get(&target) != Some(&muted) {
                let args = vec!["set-mute".to_string(), target.clone(), value.to_string()];
                run_wpctl(&args);
                last_sink_mute_by_target.insert(target, muted);
            }
        }
        Channel::Game | Channel::Media | Channel::Chat | Channel::Aux => {
            for stream in snapshot.streams.values() {
                let stream_channel = classify_with_priority(
                    overrides,
                    Some(&stream.app_key),
                    Some(&stream.display_name),
                    stream.media_role.as_deref(),
                );
                if stream_channel == channel {
                    let args = vec![
                        "set-mute".to_string(),
                        stream.id.to_string(),
                        value.to_string(),
                    ];
                    let _ = run_wpctl_checked(&args);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
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
}
