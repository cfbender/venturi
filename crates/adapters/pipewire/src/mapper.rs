use std::collections::HashMap;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NodeFingerprint {
    pub app_name: String,
    pub object_path: String,
}

#[derive(Default)]
pub struct StableIdMapper {
    next_id: u64,
    stable_by_fingerprint: HashMap<NodeFingerprint, String>,
    stable_by_pw_id: HashMap<u32, String>,
}

impl StableIdMapper {
    pub fn upsert(&mut self, pw_id: u32, fingerprint: NodeFingerprint) -> String {
        let stable_id = self
            .stable_by_fingerprint
            .entry(fingerprint)
            .or_insert_with(|| {
                let id = format!("node-{}", self.next_id);
                self.next_id += 1;
                id
            })
            .clone();

        self.stable_by_pw_id.insert(pw_id, stable_id.clone());
        stable_id
    }
}
