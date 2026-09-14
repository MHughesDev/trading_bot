//! Canonical manifests and the idempotency hash (COMP-005 §4).
//!
//! A job is identified by what it *is*, not by when it was asked for. Two identical
//! submissions must collapse into one job (JB-02), and that only works if the
//! manifest serialises byte-identically every time — regardless of the order a
//! caller happened to build its JSON object in.
//!
//! So the manifest is canonicalised with JSON Canonicalization Scheme (RFC 8785)
//! before hashing: object keys sorted by UTF-16 code unit, no insignificant
//! whitespace, no trailing zeros on numbers.

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Serialises a value to RFC 8785 canonical JSON.
///
/// Scope note: integers are emitted exactly, and floating point uses serde_json's
/// shortest round-trip form, which agrees with RFC 8785's ES6 `Number::toString` for
/// every value a manifest realistically carries. Manifests should prefer integers
/// and strings anyway — a float that differs in its last bit between two callers
/// would defeat deduplication, and [`hash`] cannot detect that.
pub fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_json_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, val)) in sorted_entries(map).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(key, out);
                out.push(':');
                write_canonical(val, out);
            }
            out.push('}');
        }
    }
}

/// Object members sorted by UTF-16 code unit, as RFC 8785 requires.
///
/// This is not the same as Rust's `str` ordering, which compares by Unicode scalar
/// value: the two disagree for characters above the BMP, because a surrogate pair
/// sorts below U+E000..U+FFFF in UTF-16 but above it by scalar value. Manifest keys
/// are ASCII in practice, but getting this wrong would produce a hash that a
/// conforming implementation disagrees with, and such a bug is invisible until the
/// day it is not.
fn sorted_entries(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut entries: Vec<(&String, &Value)> = map.iter().collect();
    entries.sort_by_key(|(key, _)| utf16_units(key));
    entries
}

fn utf16_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The idempotency hash of a submission (COMP-005 §4).
///
/// `sha256(canonical(manifest) ‖ sorted(code_hashes) ‖ data_snapshot_id ‖ kind)`.
///
/// `code_hashes` are sorted so that the order a caller lists its code snapshots in
/// cannot change the identity of the job. `data_snapshot_id` pins the data version,
/// so the *same* strategy over *different* data is a different job — without it,
/// re-running a backtest after a backfill would silently dedupe to the old result.
pub fn hash(
    manifest: &Value,
    code_hashes: &[String],
    data_snapshot_id: &str,
    kind: &str,
) -> String {
    let mut sorted = code_hashes.to_vec();
    sorted.sort();

    let mut hasher = Sha256::new();
    hasher.update(canonical(manifest).as_bytes());
    hasher.update(b"\x1f");
    for code_hash in &sorted {
        hasher.update(code_hash.as_bytes());
        hasher.update(b",");
    }
    hasher.update(b"\x1f");
    hasher.update(data_snapshot_id.as_bytes());
    hasher.update(b"\x1f");
    hasher.update(kind.as_bytes());
    hex::encode(hasher.finalize())
}

/// Content address of a byte stream, used for artifact handles (COMP-005 §8.3).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// `art_` plus the first 32 hex characters of the sha256.
pub fn artifact_handle(sha256: &str) -> String {
    format!("art_{}", &sha256[..32.min(sha256.len())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_the_canonical_form() {
        let a = json!({"b": 1, "a": 2, "c": {"z": 1, "y": 2}});
        let b = json!({"c": {"y": 2, "z": 1}, "a": 2, "b": 1});
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(canonical(&a), r#"{"a":2,"b":1,"c":{"y":2,"z":1}}"#);
    }

    #[test]
    fn array_order_is_significant() {
        // Arrays carry meaning in their order — a list of instruments or a sequence
        // of steps is not a set. Only object keys are reordered.
        let a = json!({"xs": [1, 2, 3]});
        let b = json!({"xs": [3, 2, 1]});
        assert_ne!(canonical(&a), canonical(&b));
    }

    #[test]
    fn strings_are_escaped_per_rfc_8785() {
        let value = json!({"s": "a\"b\\c\nd\te\u{1}"});
        let out = canonical(&value);
        // Asserted as properties rather than one exact literal. An expected string
        // for this case is itself easy to get wrong — a raw control byte looks
        // identical to its escape in most editors — and a test whose expectation is
        // wrong in the same direction as the code proves nothing.
        assert!(
            out.contains(r"\u0001"),
            "control char must be escaped: {out:?}"
        );
        assert!(out.contains(r"\n"), "newline must be escaped: {out:?}");
        assert!(out.contains(r"\t"), "tab must be escaped: {out:?}");
        assert!(out.contains(r#"\""#), "quote must be escaped: {out:?}");
        assert!(out.contains(r"\\"), "backslash must be escaped: {out:?}");
        assert!(
            !out.chars().any(|c| (c as u32) < 0x20),
            "no raw control character may survive canonicalisation: {out:?}"
        );
    }

    #[test]
    fn no_insignificant_whitespace() {
        let v = json!({"a": [1, {"b": null}], "c": true});
        let out = canonical(&v);
        assert!(!out.contains(' '), "got {out}");
        assert!(!out.contains('\n'));
    }

    #[test]
    fn hash_is_stable_across_key_order() {
        let a = json!({"instrument": "BTC-USD", "window": {"to": 2, "from": 1}});
        let b = json!({"window": {"from": 1, "to": 2}, "instrument": "BTC-USD"});
        assert_eq!(
            hash(&a, &[], "snap1", "backtest"),
            hash(&b, &[], "snap1", "backtest")
        );
    }

    #[test]
    fn hash_is_stable_across_code_hash_order() {
        let m = json!({"x": 1});
        let one = hash(&m, &["bbb".into(), "aaa".into()], "snap1", "backtest");
        let two = hash(&m, &["aaa".into(), "bbb".into()], "snap1", "backtest");
        assert_eq!(one, two);
    }

    #[test]
    fn kind_data_snapshot_and_code_all_change_the_hash() {
        let m = json!({"x": 1});
        let base = hash(&m, &["aaa".into()], "snap1", "backtest");
        assert_ne!(base, hash(&m, &["aaa".into()], "snap1", "study"));
        assert_ne!(base, hash(&m, &["aaa".into()], "snap2", "backtest"));
        assert_ne!(base, hash(&m, &["bbb".into()], "snap1", "backtest"));
        assert_ne!(
            base,
            hash(&json!({"x": 2}), &["aaa".into()], "snap1", "backtest")
        );
    }

    #[test]
    fn separators_stop_field_boundaries_from_blurring() {
        // Without a delimiter between the concatenated parts, ("ab","c") and
        // ("a","bc") would hash identically and two different jobs would dedupe
        // into one.
        let m = json!({});
        assert_ne!(
            hash(&m, &[], "ab", "c"),
            hash(&m, &[], "a", "bc"),
            "concatenated fields must not blur into one another"
        );
    }

    #[test]
    fn rerun_nonce_produces_a_distinct_job() {
        // An explicit re-run must be a new job and a new trial (COMP-005 §4).
        let plain = json!({"strategy": "x"});
        let rerun = json!({"strategy": "x", "rerun_nonce": "01J8"});
        assert_ne!(
            hash(&plain, &[], "snap1", "backtest"),
            hash(&rerun, &[], "snap1", "backtest")
        );
    }

    #[test]
    fn artifact_handles_are_derived_from_the_content_hash() {
        let sha = sha256_hex(b"hello");
        let handle = artifact_handle(&sha);
        assert!(handle.starts_with("art_"));
        assert_eq!(handle.len(), 4 + 32);
        assert_eq!(
            artifact_handle(&sha),
            artifact_handle(&sha256_hex(b"hello"))
        );
        assert_ne!(handle, artifact_handle(&sha256_hex(b"hello!")));
    }

    #[test]
    fn utf16_ordering_differs_from_scalar_ordering_where_it_matters() {
        // U+1F600 is a surrogate pair in UTF-16 (0xD83D 0xDE00), so it sorts BELOW
        // U+FF01 (0xFF01) by code unit, and ABOVE it by scalar value. RFC 8785 wants
        // the former.
        let mut map = Map::new();
        map.insert("\u{1F600}".into(), json!(1));
        map.insert("\u{FF01}".into(), json!(2));
        let keys: Vec<&str> = sorted_entries(&map)
            .into_iter()
            .map(|(k, _)| k.as_str())
            .collect();
        assert_eq!(
            keys,
            vec!["\u{1F600}", "\u{FF01}"],
            "keys must sort by UTF-16 code unit, not scalar value"
        );
    }
}
