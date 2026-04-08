use crate::core::pipewire_backend::PwPlayProcess;
use std::collections::BTreeMap;

pub(crate) const VENTURI_SOUNDBOARD_SINK: &str = "Venturi-Sound";
pub(crate) const VENTURI_MAIN_OUTPUT: &str = "Venturi-Output";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SoundboardPlaybackRoute {
    VirtualMicInput,
    MainOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SoundboardPlaybackMode {
    Full,
    Preview,
}

const SOUNDBOARD_PLAYBACK_ROUTES_FULL: [SoundboardPlaybackRoute; 2] = [
    SoundboardPlaybackRoute::VirtualMicInput,
    SoundboardPlaybackRoute::MainOutput,
];

const SOUNDBOARD_PLAYBACK_ROUTES_PREVIEW: [SoundboardPlaybackRoute; 1] =
    [SoundboardPlaybackRoute::MainOutput];

pub(crate) fn soundboard_playback_routes(
    mode: SoundboardPlaybackMode,
) -> &'static [SoundboardPlaybackRoute] {
    match mode {
        SoundboardPlaybackMode::Full => SOUNDBOARD_PLAYBACK_ROUTES_FULL.as_slice(),
        SoundboardPlaybackMode::Preview => SOUNDBOARD_PLAYBACK_ROUTES_PREVIEW.as_slice(),
    }
}

pub(crate) fn soundboard_playback_target_for_route(route: SoundboardPlaybackRoute) -> &'static str {
    match route {
        SoundboardPlaybackRoute::VirtualMicInput => VENTURI_SOUNDBOARD_SINK,
        SoundboardPlaybackRoute::MainOutput => VENTURI_MAIN_OUTPUT,
    }
}

#[cfg(test)]
pub(crate) fn soundboard_playback_targets(mode: SoundboardPlaybackMode) -> Vec<&'static str> {
    soundboard_playback_routes(mode)
        .iter()
        .map(|route| soundboard_playback_target_for_route(*route))
        .collect()
}

pub(crate) fn handle_play_sound(
    soundboard_players: &mut BTreeMap<(u32, SoundboardPlaybackRoute), PwPlayProcess>,
    pad_id: u32,
    file: &str,
    mode: SoundboardPlaybackMode,
    event_tx: &crossbeam_channel::Sender<crate::core::messages::CoreEvent>,
) {
    let trimmed_file = file.trim();
    if trimmed_file.is_empty() {
        let _ = event_tx.send(crate::core::messages::CoreEvent::Error(format!(
            "cannot play soundboard pad {pad_id}: no file configured"
        )));
        return;
    }

    stop_sound(soundboard_players, pad_id);

    for route in soundboard_playback_routes(mode) {
        let target = soundboard_playback_target_for_route(*route);
        match PwPlayProcess::spawn(target, trimmed_file) {
            Ok(player) => {
                soundboard_players.insert((pad_id, *route), player);
            }
            Err(err) => {
                let _ = event_tx.send(crate::core::messages::CoreEvent::Error(format!(
                    "failed to play soundboard pad {pad_id} on {target}: {err}"
                )));
            }
        }
    }
}

pub(crate) fn stop_sound(
    soundboard_players: &mut BTreeMap<(u32, SoundboardPlaybackRoute), PwPlayProcess>,
    pad_id: u32,
) {
    soundboard_players.retain(|(existing_pad_id, _), player| {
        if *existing_pad_id == pad_id {
            player.stop();
            false
        } else {
            true
        }
    });
}

pub(crate) fn cleanup_soundboard_players(
    soundboard_players: &mut BTreeMap<(u32, SoundboardPlaybackRoute), PwPlayProcess>,
) {
    soundboard_players.retain(|_, player| !player.is_finished());
}
