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
}

impl StableIdMapper {
    /// Inserts or updates a PipeWire node fingerprint and returns its stable ID.
    ///
    /// Stable IDs are keyed by fingerprint identity (`app_name` + `object_path`).
    /// A fingerprint always maps to the same stable ID, even if its PipeWire
    /// runtime ID changes. Reusing a runtime ID for a different fingerprint
    /// produces that fingerprint's stable ID.
    pub fn upsert(&mut self, _pw_id: u32, fingerprint: NodeFingerprint) -> String {
        self.stable_by_fingerprint
            .entry(fingerprint)
            .or_insert_with(|| {
                let id = format!("node-{}", self.next_id);
                self.next_id += 1;
                id
            })
            .clone()
    }
}
