//! `mcp-server` mirrors `jobs::manifest::canonical`, and the two must not drift.
//!
//! The copy exists because `crates/jobs` depends on `sqlx`, `ledger` and `storage`,
//! and pulling that graph into `crates/mcp-server` for one pure string function
//! would cost every build of it — the same reason `spec::Envelope` mirrors
//! `ledger::audit::Envelope` rather than importing it.
//!
//! A copy is only safe if it cannot drift silently. This is the test that makes the
//! drift loud: `crates/invariants` is the one crate that dev-depends on both, and a
//! divergence fails here rather than producing two different idempotency keys for
//! one argument object — which would look like a duplicate write nobody ordered.
//!
//! The corpus is built from the cases implementations actually disagree on, not from
//! easy ones: key ordering (including above the BMP, where UTF-16 and byte order
//! part company), the escape set, integral floats, and nesting.

use serde_json::{json, Value};

fn corpus() -> Vec<Value> {
    vec![
        json!({}),
        json!([]),
        json!(null),
        json!(true),
        // Ordering, plain and reversed: the canonical form must be identical.
        json!({ "a": 1, "b": 2, "c": 3 }),
        json!({ "c": 3, "b": 2, "a": 1 }),
        // Above the BMP. A `BTreeMap` sorts by byte and RFC 8785 by UTF-16 code
        // unit, and this is where the two part company.
        json!({ "\u{1F600}": 1, "z": 2 }),
        json!({ "z": 2, "\u{1F600}": 1 }),
        // Numbers. NOTE: `1` and `1.0` canonicalise DIFFERENTLY in both
        // implementations, which deviates from RFC 8785 — see
        // `idempotency::canonical_number`. Both cases are in the corpus so that a
        // change to either side fails here.
        json!({ "n": 1 }),
        json!({ "n": 1.0 }),
        json!({ "n": -0.5, "m": 1e10, "k": 12_345_678_901_234.0 }),
        // Strings: every escape the spec names, plus raw control characters.
        json!({ "s": "quote\" tab\t nl\n cr\r ff\u{c} bs\u{8}" }),
        json!({ "s": "backslash\\ slash/" }),
        json!({ "s": "ctrl\u{1}\u{1f}" }),
        json!({ "s": "unicode: caf\u{e9} \u{4e2d}\u{6587}" }),
        // Arrays keep their order, and nesting is where a naive writer drops commas.
        json!({ "legs": ["buy", "sell"], "nested": [{ "b": 1, "a": 2 }, []] }),
        json!([[[1, 2], [3]], { "x": [true, null] }]),
        // The shape an actual tool call has.
        json!({
            "instrument_id": "BTC-USD",
            "timeframe": "1h",
            "params": { "slow": 21, "fast": 7 },
            "seed": 42
        }),
    ]
}

#[test]
fn the_mirrored_canonicaliser_agrees_with_the_job_services_own() {
    for value in corpus() {
        let mirrored = mcp_server_lib::idempotency::canonical(&value);
        let original = jobs::manifest::canonical(&value);
        assert_eq!(
            mirrored, original,
            "canonical JSON diverged on {value}\n  mirrored: {mirrored}\n  original: {original}"
        );
    }
}

/// And the property the copy exists to provide: two orderings of one object
/// canonicalise identically, in BOTH implementations. A test that only compared the
/// two against each other would pass if they were identically wrong.
#[test]
fn both_implementations_are_order_independent() {
    let a = json!({ "a": 1, "b": 2, "c": 3 });
    let b = json!({ "c": 3, "a": 1, "b": 2 });
    assert_eq!(
        mcp_server_lib::idempotency::canonical(&a),
        mcp_server_lib::idempotency::canonical(&b)
    );
    assert_eq!(jobs::manifest::canonical(&a), jobs::manifest::canonical(&b));
}

/// The output is parseable JSON, and canonicalising it again does not change it.
/// A canonicaliser that emits a raw control character produces a string no reader
/// can take back, and the failure surfaces far from here.
#[test]
fn every_canonical_form_round_trips() {
    for value in corpus() {
        let text = mcp_server_lib::idempotency::canonical(&value);
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("canonical form is not JSON: {text}\n{e}"));
        assert_eq!(
            mcp_server_lib::idempotency::canonical(&parsed),
            text,
            "canonicalising twice changed it: {text}"
        );
    }
}
