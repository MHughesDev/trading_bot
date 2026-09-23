//! Content addressing for strategy definitions (ADR-P6-04).
//!
//! A [`RevisionHash`] is the identity of a strategy *document*, as distinct from
//! `strategy_id`, which is the identity of a *slot*. Before 0058 the platform
//! only had the slot, and a re-save replaced what everything pointed at.
//!
//! ## Canonicalisation, and what it is allowed to lose
//!
//! The hash is SHA-256 over a canonical JSON serialization in the shape RFC 8785
//! describes: object keys sorted by their UTF-16 code units, no insignificant
//! whitespace, arrays left in document order.
//!
//! Sorting keys is safe. Sorting *arrays* would not be: `nodes` is a DAG whose
//! declaration order decides nothing, but `exits.rules` is evaluated in order
//! and `actions` fire in order, so a canonicaliser that sorted arrays would make
//! two behaviourally different strategies hash alike. Nothing here sorts an
//! array, and [`canonical_json`] is deliberately not generic over "collections
//! that look unordered".
//!
//! ## Why the original bytes are kept too
//!
//! Canonicalisation is lossy in one direction that matters: a decimal written
//! `"0.10"` and one written `"0.1"` are the same JSON number and not the same
//! string, and anything that round-trips a price through `f64` can change the
//! last digit. The revision table stores `definition_bytes` alongside the
//! canonical form so that reproducing a result means replaying what was
//! actually sent, not what we later decided it meant.
//!
//! ## What the hash is not
//!
//! It is not a behaviour fingerprint. Two documents that differ only in a
//! comment-like field hash differently and trade identically; two that differ in
//! one threshold hash differently and may trade identically too. Search
//! deduplication needs a behaviour fingerprint — the correlation of realised
//! returns — and using the content hash for it would count a thousand
//! cosmetically distinct twins as a thousand independent trials, which is
//! exactly the error `N_eff` exists to prevent.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::StrategyDefinition;

/// The content address of one strategy document: lowercase hex SHA-256.
///
/// No public constructor from a string that is not a hash — [`RevisionHash::parse`]
/// checks the shape, because a 63-character "hash" that reaches the database
/// fails a CHECK constraint at 3 a.m. instead of a type at compile time.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RevisionHash(String);

impl RevisionHash {
    /// Hash a definition. The only way to mint one from a document.
    ///
    /// # Errors
    /// When the definition cannot be serialized to JSON at all.
    pub fn of(def: &StrategyDefinition) -> Result<Self, serde_json::Error> {
        let canonical = canonical_json(&serde_json::to_value(def)?);
        Ok(Self(hex::encode(Sha256::digest(canonical.as_bytes()))))
    }

    /// Accept an existing hash, checking its shape.
    ///
    /// # Errors
    /// When `s` is not 64 lowercase hex characters.
    pub fn parse(s: impl Into<String>) -> Result<Self, RevisionHashError> {
        let s = s.into();
        let ok = s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then_some(Self(s.clone()))
            .ok_or(RevisionHashError::Malformed(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The first twelve characters, for logs and UI. Never for equality: a
    /// prefix is not an identity, and treating it as one is how two revisions
    /// become one in a join.
    #[must_use]
    pub fn short(&self) -> &str {
        &self.0[..12]
    }
}

impl std::fmt::Display for RevisionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<RevisionHash> for String {
    fn from(h: RevisionHash) -> Self {
        h.0
    }
}

impl TryFrom<String> for RevisionHash {
    type Error = RevisionHashError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(s)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RevisionHashError {
    #[error("'{0}' is not a revision hash: expected 64 lowercase hex characters")]
    Malformed(String),
}

/// Serialize a JSON value in canonical form: sorted object keys, no
/// insignificant whitespace, **array order preserved**.
///
/// See the module docs for why arrays are never sorted.
#[must_use]
pub fn canonical_json(v: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            // `serde_json::Map` is a BTreeMap unless the `preserve_order`
            // feature is on, so collecting and sorting makes the ordering
            // independent of that feature rather than quietly dependent on it.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::Value::String((*k).clone()).to_string());
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
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
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy_def::exits::{ExitDistance, ExitPolicy, ExitRule};
    use rust_decimal_macros::dec;
    use serde_json::json;

    fn def() -> StrategyDefinition {
        serde_json::from_value(json!({
            "strategy_id": "ema_cross_v1",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [
                { "id": "n1", "type": "condition", "expr": "feature('ema_7') > feature('ema_21')" },
                { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
            ],
            "actions": [{
                "on_signal": "long", "type": "place_order",
                "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" }
            }]
        }))
        .unwrap()
    }

    #[test]
    fn the_same_document_hashes_the_same_way_every_time() {
        assert_eq!(
            RevisionHash::of(&def()).unwrap(),
            RevisionHash::of(&def()).unwrap()
        );
    }

    #[test]
    fn a_hash_is_sixty_four_lowercase_hex_characters() {
        let h = RevisionHash::of(&def()).unwrap();
        assert_eq!(h.as_str().len(), 64);
        assert!(h
            .as_str()
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_eq!(h.short().len(), 12);
    }

    /// The change this whole migration exists for: editing a strategy produces
    /// a different artifact rather than replacing one.
    #[test]
    fn changing_anything_changes_the_hash() {
        let a = RevisionHash::of(&def()).unwrap();
        let mut b = def();
        b.definition_version = "1.1".into();
        assert_ne!(a, RevisionHash::of(&b).unwrap());
    }

    /// Adding an exit policy must change the hash. If it did not, two runs with
    /// the same `strategy_version` could close positions differently — the
    /// precise defect the typed `exits` block was added to close.
    #[test]
    fn declaring_exits_changes_the_hash() {
        let bare = RevisionHash::of(&def()).unwrap();

        let mut delegated = def();
        delegated.exits = Some(ExitPolicy::RiskLayerOnly);
        let delegated_hash = RevisionHash::of(&delegated).unwrap();

        let mut stopped = def();
        stopped.exits = Some(ExitPolicy::Rules {
            rules: vec![ExitRule::StopLoss {
                distance: ExitDistance::of_entry(dec!(0.02)),
            }],
        });
        let stopped_hash = RevisionHash::of(&stopped).unwrap();

        assert_ne!(
            bare, delegated_hash,
            "pre-1.3 and 'risk layer only' are different claims"
        );
        assert_ne!(delegated_hash, stopped_hash);
        assert_ne!(bare, stopped_hash);
    }

    /// Two rule lists in different orders behave differently on a bar that
    /// breaches both, so they must not hash alike. This is the test that stops
    /// somebody adding array sorting to the canonicaliser.
    #[test]
    fn reordering_exit_rules_changes_the_hash() {
        let rules = |a: ExitRule, b: ExitRule| {
            let mut d = def();
            d.exits = Some(ExitPolicy::Rules { rules: vec![a, b] });
            RevisionHash::of(&d).unwrap()
        };
        let stop = ExitRule::StopLoss {
            distance: ExitDistance::of_entry(dec!(0.02)),
        };
        let trail = ExitRule::TrailingStop {
            distance: ExitDistance::of_entry(dec!(0.03)),
        };
        assert_ne!(rules(stop.clone(), trail.clone()), rules(trail, stop));
    }

    // ── the canonicaliser ────────────────────────────────────────────────────

    #[test]
    fn object_keys_are_sorted_and_arrays_are_not() {
        let v = json!({ "b": 1, "a": 2, "c": [3, 1, 2] });
        assert_eq!(canonical_json(&v), r#"{"a":2,"b":1,"c":[3,1,2]}"#);
    }

    #[test]
    fn key_order_in_the_input_does_not_change_the_output() {
        let one: serde_json::Value = serde_json::from_str(r#"{"z":1,"a":{"y":2,"b":3}}"#).unwrap();
        let two: serde_json::Value = serde_json::from_str(r#"{"a":{"b":3,"y":2},"z":1}"#).unwrap();
        assert_eq!(canonical_json(&one), canonical_json(&two));
    }

    #[test]
    fn whitespace_is_not_significant() {
        let a: serde_json::Value = serde_json::from_str(r#"{"a":  1 ,  "b": [ 2 ]}"#).unwrap();
        let b: serde_json::Value = serde_json::from_str(r#"{"a":1,"b":[2]}"#).unwrap();
        assert_eq!(canonical_json(&a), canonical_json(&b));
    }

    #[test]
    fn nesting_is_canonicalised_all_the_way_down() {
        let v = json!({ "x": [{ "d": 1, "c": 2 }, { "b": 3, "a": 4 }] });
        assert_eq!(canonical_json(&v), r#"{"x":[{"c":2,"d":1},{"a":4,"b":3}]}"#);
    }

    #[test]
    fn strings_needing_escapes_survive_canonicalisation() {
        let v = json!({ "expr": "feature('a') > \"b\"\n\t" });
        let back: serde_json::Value = serde_json::from_str(&canonical_json(&v)).unwrap();
        assert_eq!(back, v);
    }

    // ── parsing ──────────────────────────────────────────────────────────────

    #[test]
    fn a_malformed_hash_is_refused_at_the_type_rather_than_the_database() {
        assert!(RevisionHash::parse("a".repeat(64)).is_ok());
        assert!(RevisionHash::parse("a".repeat(63)).is_err());
        assert!(
            RevisionHash::parse("A".repeat(64)).is_err(),
            "uppercase is a different string"
        );
        assert!(RevisionHash::parse("g".repeat(64)).is_err(), "g is not hex");
        assert!(RevisionHash::parse("").is_err());
    }

    #[test]
    fn a_hash_round_trips_through_json_and_refuses_junk() {
        let h = RevisionHash::of(&def()).unwrap();
        let s = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<RevisionHash>(&s).unwrap(), h);
        assert!(serde_json::from_str::<RevisionHash>(r#""nope""#).is_err());
    }
}
