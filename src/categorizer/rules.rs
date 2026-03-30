use std::collections::BTreeMap;

use crate::core::messages::Channel;

pub fn classify(binary_or_name: &str, media_role: Option<&str>) -> Channel {
    match binary_or_name {
        "discord" | "mumble" | "teamspeak" | "zoom" | "slack" => Channel::Chat,
        "steam" | "gamescope" | "lutris" | "heroic" => Channel::Game,
        "firefox" | "chromium" | "spotify" | "vlc" | "mpv" => Channel::Media,
        _ => match media_role {
            Some("Game") => Channel::Game,
            Some("Music") | Some("Movie") | Some("Video") => Channel::Media,
            Some("Communication") | Some("Phone") => Channel::Chat,
            _ => Channel::Aux,
        },
    }
}

pub fn matching_key(binary: Option<&str>, app_name: Option<&str>) -> String {
    binary
        .or(app_name)
        .unwrap_or("unknown")
        .to_ascii_lowercase()
}

pub fn classify_with_priority(
    overrides: &BTreeMap<String, Channel>,
    binary: Option<&str>,
    app_name: Option<&str>,
    media_role: Option<&str>,
) -> Channel {
    let key = matching_key(binary, app_name);
    if let Some(channel) = overrides.get(&key) {
        return *channel;
    }

    classify(&key, media_role)
}
