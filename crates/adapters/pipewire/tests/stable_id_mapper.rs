use venturi_pipewire_adapter::{NodeFingerprint, StableIdMapper};

#[test]
fn remap_preserves_stable_id_when_pw_id_changes() {
    let mut mapper = StableIdMapper::default();
    let fp = NodeFingerprint {
        app_name: "Discord".to_string(),
        object_path: "stream://discord/call".to_string(),
    };

    let id_a = mapper.upsert(10, fp.clone());
    let id_b = mapper.upsert(77, fp);
    assert_eq!(id_a, id_b);
}
