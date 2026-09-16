//! Not writing twice (AGENT-006 §2.4b, Set S task S-1.19).
//!
//! A write that runs twice is worse than a write that fails: the second run is
//! invisible, and every count downstream — trials, orders, ledger rows — is off by
//! one with nothing to point at. The loop retries, the driver reconnects, a process
//! restarts mid-dispatch; all three look identical from here, and all three must
//! produce one action.
//!
//! So every write and destructive call carries a key derived from
//! `(run_id, step, tool, canonical(arguments))`. Same key, same action: the first
//! call performs it and the result is kept, and a call with a key already seen
//! returns that result without touching the platform.
//!
//! ## Why the canonicaliser is duplicated
//!
//! `jobs::manifest::canonical` is the same function and is not called here, for the
//! reason [`crate::spec::Envelope`] is a local mirror of `ledger::audit::Envelope`:
//! `crates/jobs` depends on `sqlx`, `ledger` and `storage`, and pulling that graph
//! into this crate for one pure string function would cost every build of it.
//!
//! A copy is only safe if it cannot drift, so it does not rely on not drifting:
//! `crates/invariants` dev-depends on both and asserts they agree byte for byte over
//! a corpus built from the cases that actually differ between implementations — key
//! ordering, escapes, integral floats, nesting. A divergence fails that test rather
//! than producing two different keys for one argument object, and it caught one on
//! its first run: see [`canonical_number`], where the mirror follows `jobs` away from
//! the RFC because `jobs`' output is already hashed into stored manifests.
//!
//! ## What this does not do
//!
//! **It is in-memory and per-context.** An in-run retry and a reconnect are covered;
//! a post-crash resume is not, because the record dies with the process. Making it
//! durable is a table and a write on the dispatch path, and it belongs with the
//! run-persistence work rather than here. Stated rather than implied: a caller that
//! needs crash-safety does not have it yet.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::Value;
use uuid::Uuid;

/// Serialise to canonical JSON, RFC 8785 with one documented deviation.
///
/// Object keys sorted by their UTF-16 code units, no insignificant whitespace.
/// **Arrays are never sorted** — order is content in an array, and sorting one would
/// make two different sequences of actions hash the same.
///
/// Numbers are `serde_json`'s spelling rather than the RFC's; see
/// [`canonical_number`] for why, and for what it costs.
#[must_use]
pub fn canonical(value: &Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&canonical_number(n)),
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
            // RFC 8785 sorts by UTF-16 code unit. `BTreeMap` over `String` sorts by
            // byte, and the two disagree only above the BMP — a key containing an
            // emoji, say. Collected and sorted explicitly so the rule is the one
            // written down rather than the one the container happened to have.
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by_key(|k| k.encode_utf16().collect::<Vec<u16>>());
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
    }
}

/// Numbers, spelled the way `jobs::manifest::canonical` spells them.
///
/// **This deviates from RFC 8785 on purpose, and the deviation is not mine to
/// fix here.** The spec says to render numbers by ECMAScript `Number::toString`,
/// under which `1.0` is `"1"` — so `{"n": 1}` and `{"n": 1.0}` are one JSON value
/// and should canonicalise identically. `serde_json` renders them `"1"` and `"1.0"`,
/// and `jobs` takes that as-is.
///
/// Matching the RFC here would make this mirror disagree with the original, which
/// is the one thing a mirror must not do. Fixing it in `jobs` instead would change
/// the canonical form of every job manifest, and manifests are **hashed and
/// stored**: the new hash would disagree with every recorded one, in the same shape
/// as the sqlx migration-checksum failure that produced `.gitattributes`.
///
/// So the copy matches the original, and the latent defect is written down rather
/// than half-fixed: an argument object whose number arrives as `1` on one call and
/// `1.0` on another gets two idempotency keys, and the write happens twice. That is
/// narrow — a tool schema types its fields, so a field is an integer or it is not —
/// but it is real, and it belongs in OPEN-QUESTIONS rather than in a comment that
/// claims the property holds.
fn canonical_number(n: &serde_json::Number) -> String {
    n.to_string()
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// The key for one call.
///
/// `step` is in the key on purpose. The same tool with the same arguments at a later
/// step is a **different act** — placing the same order twice is what a trader means
/// to do when they do it twice — so deduplicating across steps would silently drop
/// an intended repeat. What this collapses is one step retried, which is the case
/// where the repeat was never intended.
#[must_use]
pub fn key(run_id: Uuid, step: u32, tool: &str, arguments: &Value) -> String {
    let material = format!("{run_id}|{step}|{tool}|{}", canonical(arguments));
    // FNV-1a over the material: no dependency, and this is a deduplication key
    // rather than anything anyone should be unable to forge. A caller who can forge
    // it already holds the run.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in material.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// What one context has already done, so it is not done again.
#[derive(Debug, Default)]
pub struct Ledger {
    seen: Mutex<BTreeMap<String, Value>>,
}

impl Ledger {
    /// The result of a call already made under this key, if any.
    ///
    /// A poisoned lock returns `None` rather than propagating, and that direction is
    /// deliberate: the failure is then a duplicate write, which is bad, against a
    /// dead dispatch path, which is worse. It is also unreachable in practice —
    /// nothing under this lock can panic.
    #[must_use]
    pub fn replay(&self, key: &str) -> Option<Value> {
        self.seen.lock().ok()?.get(key).cloned()
    }

    /// Record what a call returned, so a repeat returns the same thing.
    pub fn record(&self, key: &str, result: &Value) {
        if let Ok(mut seen) = self.seen.lock() {
            seen.insert(key.to_string(), result.clone());
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.seen.lock().map(|s| s.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RUN: Uuid = Uuid::nil();

    #[test]
    fn key_order_does_not_change_the_key() {
        let a = json!({ "instrument": "BTC-USD", "size": 1, "side": "buy" });
        let b = json!({ "side": "buy", "size": 1, "instrument": "BTC-USD" });
        assert_eq!(canonical(&a), canonical(&b));
        assert_eq!(key(RUN, 3, "place", &a), key(RUN, 3, "place", &b));
    }

    /// Array order IS content. Sorting one would make two different orders — two
    /// different sequences of actions — hash the same.
    #[test]
    fn array_order_does_change_the_key() {
        let a = json!({ "legs": ["buy", "sell"] });
        let b = json!({ "legs": ["sell", "buy"] });
        assert_ne!(key(RUN, 1, "t", &a), key(RUN, 1, "t", &b));
    }

    /// `1` and `1.0` are the same JSON number and DO hash differently, because
    /// this mirrors `jobs` and `jobs` renders them differently.
    ///
    /// Asserted rather than left implicit. The property the RFC promises does not
    /// hold here, and a test that quietly skipped it would leave the next reader
    /// assuming it did — see `canonical_number` for why it is not fixed in either
    /// place, and OPEN-QUESTIONS for the decision.
    #[test]
    fn an_integral_float_does_not_hash_as_an_integer_and_that_is_known() {
        assert_ne!(
            canonical(&json!({ "n": 1 })),
            canonical(&json!({ "n": 1.0 })),
            "if this starts passing, `jobs` changed and every stored manifest hash moved"
        );
    }

    /// The same call at a later step is a different act. Placing the same order
    /// twice is what a trader means when they do it twice.
    #[test]
    fn the_same_call_at_a_later_step_is_a_different_key() {
        let args = json!({ "instrument": "BTC-USD" });
        assert_ne!(key(RUN, 1, "t", &args), key(RUN, 2, "t", &args));
    }

    #[test]
    fn different_runs_and_tools_do_not_collide() {
        let args = json!({});
        assert_ne!(key(RUN, 1, "a", &args), key(RUN, 1, "b", &args));
        assert_ne!(key(RUN, 1, "a", &args), key(Uuid::from_u128(1), 1, "a", &args));
    }

    /// The whole point: one action, and the repeat gets the first result rather
    /// than a second action's.
    #[test]
    fn a_replayed_key_returns_the_original_result_and_does_not_act_again() {
        let ledger = Ledger::default();
        let k = key(RUN, 1, "create_strategy", &json!({ "slug": "ema_v1" }));
        assert!(ledger.replay(&k).is_none(), "nothing has happened yet");

        let mut actions = 0;
        let mut perform = |ledger: &Ledger| {
            if let Some(prior) = ledger.replay(&k) {
                return prior;
            }
            actions += 1;
            let result = json!({ "strategy_id": "ema_v1", "created": true });
            ledger.record(&k, &result);
            result
        };

        let first = perform(&ledger);
        let second = perform(&ledger);
        let third = perform(&ledger);
        assert_eq!(actions, 1, "the action happened once");
        assert_eq!(first, second);
        assert_eq!(second, third, "and every repeat got the original result");
    }

    /// Control characters are escaped rather than emitted raw, or the canonical
    /// form is not parseable JSON.
    #[test]
    fn control_characters_are_escaped() {
        let out = canonical(&json!({ "s": "a\u{1}b\nc" }));
        assert!(out.contains("\\u0001"), "{out}");
        assert!(out.contains("\\n"), "{out}");
    }
}
