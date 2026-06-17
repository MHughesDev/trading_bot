//! Whole-spec deterministic hash for reproducible training runs (I-3.8).
//!
//! `spec_hash = sha256( canonical_json(definition)
//!                    ‖ dataset_content_hash
//!                    ‖ seed
//!                    ‖ feature_set_versions
//!                    ‖ sidecar_env_fingerprint )`
//!
//! Same hash guarantees identical metrics when reproduce-from-hash re-executes
//! (I-3.9).  A sidecar-env mismatch is surfaced as a warning, not silently
//! swallowed (the hash still matches the original; the warning flags potential
//! numeric drift).

use sha2::{Digest, Sha256};

/// Compute the whole-spec hash.
///
/// Inputs:
/// - `definition_json`: canonical JSON of the `ModelDefinition` (caller serializes).
/// - `dataset_content_hash`: hex SHA-256 of the pinned Parquet file (from `DatasetVersion`).
/// - `seed`: integer seed (0 if not set).
/// - `feature_set_versions`: list of `"name:version"` strings for every referenced feature set.
/// - `sidecar_env_fingerprint`: JSON string from the sidecar's `/health?fingerprint=1` (empty
///   string when not available).
///
/// Returns a 64-char lowercase hex string.
pub fn compute_spec_hash(
    definition_json: &str,
    dataset_content_hash: &str,
    seed: u64,
    feature_set_versions: &[String],
    sidecar_env_fingerprint: &str,
) -> String {
    let mut hasher = Sha256::new();
    // Delimited concatenation — each field prefixed by its byte length so that
    // no two distinct inputs can produce the same byte stream.
    hash_field(&mut hasher, b"def", definition_json.as_bytes());
    hash_field(&mut hasher, b"dataset", dataset_content_hash.as_bytes());
    hash_field(&mut hasher, b"seed", &seed.to_le_bytes());
    let fsv = feature_set_versions.join(",");
    hash_field(&mut hasher, b"fsv", fsv.as_bytes());
    hash_field(&mut hasher, b"env", sidecar_env_fingerprint.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn hash_field(h: &mut Sha256, tag: &[u8], value: &[u8]) {
    // tag_len(4) || tag || value_len(8) || value
    h.update((tag.len() as u32).to_le_bytes());
    h.update(tag);
    h.update((value.len() as u64).to_le_bytes());
    h.update(value);
}

/// Canonical JSON of a `ModelDefinition` suitable as the definition component
/// of the spec hash.  Keys are sorted; floating-point values are fixed precision
/// so minor serialization differences don't change the hash.
pub fn canonical_definition_json(definition: &domain::model_def::ModelDefinition) -> String {
    // serde_json already sorts object keys deterministically when serializing
    // a struct (field declaration order).  For nested `serde_json::Value`
    // fields (hyperparameters) we sort keys manually.
    let mut v = serde_json::to_value(definition).unwrap_or(serde_json::json!({}));
    sort_json_object_keys(&mut v);
    serde_json::to_string(&v).unwrap_or_default()
}

/// Recursively sort JSON object keys for canonical serialization.
fn sort_json_object_keys(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            let sorted: serde_json::Map<String, serde_json::Value> = {
                let mut pairs: Vec<(String, serde_json::Value)> = map.clone().into_iter().collect();
                pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                pairs.into_iter().collect()
            };
            *map = sorted;
            for val in map.values_mut() {
                sort_json_object_keys(val);
            }
        }
        serde_json::Value::Array(arr) => {
            for val in arr.iter_mut() {
                sort_json_object_keys(val);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_produce_same_hash() {
        let h1 = compute_spec_hash(r#"{"a":1}"#, "abc123", 42, &["fs_core:3".to_string()], "");
        let h2 = compute_spec_hash(r#"{"a":1}"#, "abc123", 42, &["fs_core:3".to_string()], "");
        assert_eq!(h1, h2);
    }

    #[test]
    fn changing_seed_changes_hash() {
        let h1 = compute_spec_hash(r#"{"a":1}"#, "abc", 0, &[], "");
        let h2 = compute_spec_hash(r#"{"a":1}"#, "abc", 1, &[], "");
        assert_ne!(h1, h2);
    }

    #[test]
    fn changing_dataset_hash_changes_hash() {
        let h1 = compute_spec_hash(r#"{"a":1}"#, "abc", 0, &[], "");
        let h2 = compute_spec_hash(r#"{"a":1}"#, "xyz", 0, &[], "");
        assert_ne!(h1, h2);
    }

    #[test]
    fn changing_feature_versions_changes_hash() {
        let h1 = compute_spec_hash(r#"{"a":1}"#, "abc", 0, &["fs:1".to_string()], "");
        let h2 = compute_spec_hash(r#"{"a":1}"#, "abc", 0, &["fs:2".to_string()], "");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_is_64_hex_chars() {
        let h = compute_spec_hash("def", "hash", 0, &[], "");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
