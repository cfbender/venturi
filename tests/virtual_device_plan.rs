use venturi::core::virtual_devices::{default_mix_links, default_nodes, stale_venturi_nodes};

#[test]
fn creates_expected_venturi_nodes_with_autoconnect_disabled() {
    let nodes = default_nodes();
    assert_eq!(nodes.len(), 8);
    assert!(nodes.iter().all(|node| !node.autoconnect));
    assert!(nodes.iter().any(
        |node| node.name == "Venturi-VirtualMic" && node.media_class == "Audio/Source/Virtual"
    ));
}

#[test]
fn identifies_stale_venturi_nodes_by_prefix() {
    let stale = stale_venturi_nodes(["Venturi-Game", "alsa_output.foo", "Venturi-Output"]);
    assert_eq!(
        stale,
        vec!["Venturi-Game".to_string(), "Venturi-Output".to_string()]
    );
}

#[test]
fn mix_links_are_passive() {
    let links = default_mix_links();
    assert!(!links.is_empty());
    assert!(links.iter().all(|link| link.passive));
}
