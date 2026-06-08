//! Per-subscription snapshot cache for snapshot-on-connect and reconnect recovery.

use std::sync::RwLock;

use serde_json::Value;

/// Stores the latest JSON snapshot for a `(lane, instrument)` pair.
///
/// New subscribers receive this immediately on connect, before the first live frame arrives.
pub struct SnapshotStore {
    inner: RwLock<Option<Value>>,
}

impl SnapshotStore {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
        }
    }

    /// Replace the stored snapshot with `snapshot`.
    pub fn update(&self, snapshot: Value) {
        *self.inner.write().unwrap() = Some(snapshot);
    }

    /// Return the current snapshot, if one has been set.
    pub fn get(&self) -> Option<Value> {
        self.inner.read().unwrap().clone()
    }
}

impl Default for SnapshotStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_store_returns_none() {
        let store = SnapshotStore::new();
        assert!(store.get().is_none());
    }

    #[test]
    fn update_then_get_returns_latest() {
        let store = SnapshotStore::new();
        store.update(json!({ "bids": [], "asks": [], "sequence": 1 }));
        let snap = store.get().unwrap();
        assert_eq!(snap["sequence"], 1);

        store.update(
            json!({ "bids": [{"price": "50000", "size": "1"}], "asks": [], "sequence": 2 }),
        );
        let snap2 = store.get().unwrap();
        assert_eq!(snap2["sequence"], 2);
        assert_eq!(snap2["bids"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn reconnect_receives_fresh_snapshot() {
        let store = SnapshotStore::new();
        store.update(json!({ "sequence": 42 }));
        // Simulate reconnect: new client reads the snapshot.
        let snap = store.get().unwrap();
        assert_eq!(snap["sequence"], 42);
    }
}
