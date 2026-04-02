use venturi_pipewire_adapter::{NodeFingerprint, StableIdMapper};

fn fp(app_name: &str, object_path: &str) -> NodeFingerprint {
    NodeFingerprint {
        app_name: app_name.to_string(),
        object_path: object_path.to_string(),
    }
}

#[test]
fn remap_preserves_stable_id_when_pw_id_changes() {
    let mut mapper = StableIdMapper::default();
    let fp = fp("Discord", "stream://discord/call");

    let id_a = mapper.upsert(10, fp.clone());
    let id_b = mapper.upsert(77, fp);
    assert_eq!(id_a, id_b);
}

#[test]
fn different_fingerprints_get_different_stable_ids() {
    let mut mapper = StableIdMapper::default();

    let discord = mapper.upsert(10, fp("Discord", "stream://discord/call"));
    let spotify = mapper.upsert(11, fp("Spotify", "stream://spotify/track"));

    assert_ne!(discord, spotify);
}

#[test]
fn same_pw_id_with_new_fingerprint_uses_fingerprint_identity() {
    let mut mapper = StableIdMapper::default();

    let first = mapper.upsert(10, fp("Discord", "stream://discord/call"));
    let second = mapper.upsert(10, fp("Firefox", "stream://firefox/tab"));

    assert_ne!(first, second);
}

#[test]
fn repeated_upserts_of_same_inputs_are_idempotent() {
    let mut mapper = StableIdMapper::default();
    let fingerprint = fp("Discord", "stream://discord/call");

    let first = mapper.upsert(10, fingerprint.clone());
    let second = mapper.upsert(10, fingerprint.clone());
    let third = mapper.upsert(10, fingerprint);

    assert_eq!(first, second);
    assert_eq!(second, third);
}
