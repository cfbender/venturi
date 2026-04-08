use std::collections::{BTreeMap, BTreeSet};

use crate::categorizer::rules::classify_with_priority;
use crate::core::messages::Channel;
use crate::core::pipewire_discovery::Snapshot;
use crate::core::router::category_mix_output_node_name;

pub(crate) fn collect_new_stream_route_targets(
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, Channel>,
    stream_ids_before: &BTreeSet<u32>,
) -> Vec<(u32, Channel)> {
    snapshot
        .streams
        .iter()
        .filter(|(stream_id, _)| !stream_ids_before.contains(stream_id))
        .map(|(stream_id, stream)| {
            let channel = classify_with_priority(
                overrides,
                Some(&stream.app_key),
                Some(&stream.display_name),
                stream.media_role.as_deref(),
            );
            (*stream_id, channel)
        })
        .collect()
}

pub(crate) fn collect_category_stream_route_targets(
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, Channel>,
) -> Vec<(u32, Channel)> {
    snapshot
        .streams
        .iter()
        .filter_map(|(stream_id, stream)| {
            let channel = classify_with_priority(
                overrides,
                Some(&stream.app_key),
                Some(&stream.display_name),
                stream.media_role.as_deref(),
            );

            if matches!(
                channel,
                Channel::Game | Channel::Media | Channel::Chat | Channel::Aux
            ) {
                Some((*stream_id, channel))
            } else {
                None
            }
        })
        .collect()
}

fn category_mix_sink_ids_changed(
    output_ids_before: &BTreeMap<String, u32>,
    snapshot: &Snapshot,
) -> bool {
    [Channel::Game, Channel::Media, Channel::Chat, Channel::Aux]
        .iter()
        .any(|channel| {
            let Some(node_name) = category_mix_output_node_name(*channel) else {
                return false;
            };
            output_ids_before.get(node_name) != snapshot.output_ids.get(node_name)
        })
}

pub(crate) fn collect_stream_route_targets_for_reconcile(
    snapshot: &Snapshot,
    overrides: &BTreeMap<String, Channel>,
    stream_ids_before: &BTreeSet<u32>,
    output_ids_before: &BTreeMap<String, u32>,
) -> Vec<(u32, Channel)> {
    let mut targets_by_stream = BTreeMap::new();

    for (stream_id, channel) in
        collect_new_stream_route_targets(snapshot, overrides, stream_ids_before)
    {
        targets_by_stream.insert(stream_id, channel);
    }

    if category_mix_sink_ids_changed(output_ids_before, snapshot) {
        for (stream_id, channel) in collect_category_stream_route_targets(snapshot, overrides) {
            targets_by_stream.insert(stream_id, channel);
        }
    }

    targets_by_stream.into_iter().collect()
}
