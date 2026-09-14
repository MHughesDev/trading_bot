//! One content-hash function for the whole platform (CLAUDE.md §4 "same function
//! everywhere"; SHA-256 per ADR-P0-03).

use serde::Serialize;
use sha2::{Digest, Sha256};

/// Canonical JSON: object keys sorted (serde_json's `Map` is a `BTreeMap` in this
/// workspace), array order preserved.
///
/// # Errors
/// Returns an error if `value` cannot be represented as JSON.
pub fn canonical_json<T: Serialize>(value: &T) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&serde_json::to_value(value)?)
}

/// `sha256:<hex>` over canonical JSON.
///
/// # Errors
/// Returns an error if `value` cannot be represented as JSON.
pub fn content_hash<T: Serialize>(value: &T) -> serde_json::Result<String> {
    let bytes = canonical_json(value)?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_is_irrelevant_array_order_is_not() {
        let a = content_hash(&json!({"b":1,"a":[1,2]})).unwrap();
        let b = content_hash(&json!({"a":[1,2],"b":1})).unwrap();
        let c = content_hash(&json!({"a":[2,1],"b":1})).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
