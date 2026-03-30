use std::collections::BTreeMap;

use crate::categorizer::rules::classify_with_priority;
use crate::core::messages::Channel;
use crate::core::pipewire_backend::{run_wpctl, run_wpctl_checked};
use crate::core::pipewire_discovery::Snapshot;
use crate::core::router::{resolve_input_target, resolve_output_target};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChannelControlTargets<'a> {
    pub virtual_input_source_name: &'a str,
    pub main_output_sink_name: &'a str,
}

pub(crate) fn apply_channel_volume(
    channel: Channel,
    volume: f32,
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, Channel>,
    targets: ChannelControlTargets<'_>,
    last_sink_volume_by_target: &mut BTreeMap<String, f32>,
    last_source_volume_by_target: &mut BTreeMap<String, f32>,
) {
    match channel {
        Channel::Mic => {
            let target =
                resolve_input_target(Some(targets.virtual_input_source_name), &snapshot.input_ids);
            let changed = last_source_volume_by_target
                .get(&target)
                .map(|prev| (*prev - volume).abs() >= 0.01)
                .unwrap_or(true);
            if changed {
                let args = vec!["set-volume".to_string(), target.clone(), volume.to_string()];
                run_wpctl(&args);
                last_source_volume_by_target.insert(target, volume);
            }
        }
        Channel::Main => {
            let target =
                resolve_output_target(Some(targets.main_output_sink_name), &snapshot.output_ids);
            let changed = last_sink_volume_by_target
                .get(&target)
                .map(|prev| (*prev - volume).abs() >= 0.01)
                .unwrap_or(true);
            if changed {
                let args = vec!["set-volume".to_string(), target.clone(), volume.to_string()];
                run_wpctl(&args);
                last_sink_volume_by_target.insert(target, volume);
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
                        "set-volume".to_string(),
                        stream.id.to_string(),
                        volume.to_string(),
                    ];
                    let _ = run_wpctl_checked(&args);
                }
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
