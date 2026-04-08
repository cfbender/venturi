use crate::core::messages::Channel;

pub(crate) fn set_persisted_channel_volume(
    state: &mut crate::config::schema::State,
    channel: Channel,
    volume: f32,
) {
    let normalized = volume.clamp(0.0, 1.0);
    *match channel {
        Channel::Main => &mut state.volumes.main,
        Channel::Game => &mut state.volumes.game,
        Channel::Media => &mut state.volumes.media,
        Channel::Chat => &mut state.volumes.chat,
        Channel::Aux => &mut state.volumes.aux,
        Channel::Mic => &mut state.volumes.mic,
    } = normalized;
}

pub(crate) fn set_persisted_channel_mute(
    state: &mut crate::config::schema::State,
    channel: Channel,
    muted: bool,
) {
    *match channel {
        Channel::Main => &mut state.muted.main,
        Channel::Game => &mut state.muted.game,
        Channel::Media => &mut state.muted.media,
        Channel::Chat => &mut state.muted.chat,
        Channel::Aux => &mut state.muted.aux,
        Channel::Mic => &mut state.muted.mic,
    } = muted;
}
