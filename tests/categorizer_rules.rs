use std::collections::BTreeMap;

use venturi::categorizer::learning::Overrides;
use venturi::categorizer::rules::{classify_with_priority, matching_key};
use venturi::core::messages::Channel;

#[test]
fn rule_priority_is_override_then_static_then_role_then_aux() {
    let mut overrides = BTreeMap::new();
    overrides.insert("spotify".to_string(), Channel::Chat);

    let c = classify_with_priority(&overrides, Some("spotify"), Some("Spotify"), Some("Music"));
    assert_eq!(c, Channel::Chat);

    let c = classify_with_priority(
        &BTreeMap::new(),
        Some("spotify"),
        Some("Spotify"),
        Some("Communication"),
    );
    assert_eq!(c, Channel::Media);

    let c = classify_with_priority(
        &BTreeMap::new(),
        Some("unknown"),
        Some("Unknown"),
        Some("Communication"),
    );
    assert_eq!(c, Channel::Chat);

    let c = classify_with_priority(&BTreeMap::new(), Some("unknown"), Some("Unknown"), None);
    assert_eq!(c, Channel::Aux);
}

#[test]
fn matching_key_prefers_binary_then_name() {
    assert_eq!(matching_key(Some("Discord"), Some("ignored")), "discord");
    assert_eq!(matching_key(None, Some("FireFox")), "firefox");
}

#[test]
fn learning_overrides_roundtrip() {
    let mut overrides = Overrides::default();
    overrides.insert("discord", Channel::Chat);
    assert_eq!(overrides.get("discord"), Some(Channel::Chat));
    assert_eq!(overrides.as_map().get("discord"), Some(&Channel::Chat));
}
