use std::collections::BTreeMap;

use crate::core::messages::{Channel, CoreCommand};

/// Coalesce a batch of commands: keep only the last SetVolume per channel,
/// preserve all other commands in order, and stop early on Shutdown.
pub(crate) fn coalesce_commands(commands: Vec<CoreCommand>) -> Vec<CoreCommand> {
    let mut volume_map: BTreeMap<Channel, f32> = BTreeMap::new();
    let mut result: Vec<CoreCommand> = Vec::new();

    for cmd in commands {
        match cmd {
            CoreCommand::SetVolume(channel, vol) => {
                volume_map.insert(channel, vol);
            }
            CoreCommand::Shutdown => {
                // Shutdown discards pending volumes and remaining commands, but keeps
                // previously queued non-volume commands in order.
                result.push(CoreCommand::Shutdown);
                return result;
            }
            other => {
                result.push(other);
            }
        }
    }

    // Append coalesced volumes in deterministic Channel order
    for (channel, vol) in volume_map {
        result.push(CoreCommand::SetVolume(channel, vol));
    }

    result
}
