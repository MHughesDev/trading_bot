//! The content-addressed [`RunId`] — the spine of trustworthy trial counting.
//!
//! A `RunId` is the SHA-256 of a canonical encoding of *every* field of a
//! [`RunConfig`](super::config::RunConfig) except the id itself. Two configs
//! that differ in any field hash differently; two identical configs hash
//! identically and may be served from cache. Because the hash is computed (never
//! caller-supplied), you cannot silently mutate a run or re-count a cached one.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A deterministic, content-addressed run identifier (`sha256:<hex>`).
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RunId(String);

impl RunId {
    /// Hash an already-canonicalized byte buffer into a `RunId`.
    ///
    /// Callers should build the buffer with [`canonical_json`] so semantically
    /// equal configs (e.g. `params` in a different key order) collide.
    #[must_use]
    pub fn from_canonical_bytes(bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        Self(format!("sha256:{}", hex::encode(digest)))
    }

    /// The full `sha256:<hex>` string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RunId({})", self.0)
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Serialize any value to a *canonical* JSON byte buffer for hashing.
///
/// `serde_json::Value`/`Map` is backed by `BTreeMap` in this workspace (the
/// `preserve_order` feature is off), so object keys serialize in sorted order
/// regardless of insertion order. Routing through `Value` therefore makes
/// `params` key ordering irrelevant to the hash while keeping array order
/// significant — exactly the property the trial counter relies on.
///
/// # Errors
/// Returns an error if `value` cannot be represented as JSON.
pub fn canonical_json<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    let as_value = serde_json::to_value(value)?;
    serde_json::to_vec(&as_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn map_key_order_does_not_change_the_hash() {
        let a = json!({ "b": 2, "a": 1, "c": [1, 2, 3] });
        let b = json!({ "c": [1, 2, 3], "a": 1, "b": 2 });
        let ida = RunId::from_canonical_bytes(&canonical_json(&a).unwrap());
        let idb = RunId::from_canonical_bytes(&canonical_json(&b).unwrap());
        assert_eq!(ida, idb, "reordered object keys must collide");
    }

    #[test]
    fn array_order_changes_the_hash() {
        let a = json!({ "c": [1, 2, 3] });
        let b = json!({ "c": [3, 2, 1] });
        let ida = RunId::from_canonical_bytes(&canonical_json(&a).unwrap());
        let idb = RunId::from_canonical_bytes(&canonical_json(&b).unwrap());
        assert_ne!(ida, idb, "array order is significant");
    }

    #[test]
    fn id_has_prefix_and_round_trips_serde() {
        let id = RunId::from_canonical_bytes(b"hello");
        assert!(id.as_str().starts_with("sha256:"));
        let s = serde_json::to_string(&id).unwrap();
        let back: RunId = serde_json::from_str(&s).unwrap();
        assert_eq!(id, back);
    }
}
