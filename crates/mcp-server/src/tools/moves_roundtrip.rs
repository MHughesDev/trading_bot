//! `apply_move` and `diff_strategies` agree, through the tools (S-5.10;
//! AGENT-011 §4, P5; AC-6).
//!
//! The acceptance: *"every move kind round-trips from `apply_move` through
//! `diff_strategies`; `is_single_change` is false for a two-move version."*
//!
//! `strategy-lang` proves it for the library (`apply.rs`:
//! `every_move_kind_an_edit_can_reach_round_trips_through_the_differ`). This is
//! the same property at the surface a model reaches: JSON in, JSON out, two
//! separate tool calls with nothing shared between them but the definition. The
//! tool layer is where a wire shape can drift from the library's — an address
//! spelled differently, a field renamed — and it is the layer the acceptance is
//! about, since "round-trips from `apply_move` through `diff_strategies`" names
//! the two tools.

use serde_json::{json, Value};

use crate::tools::strategy::{apply_move, compile_strategy, diff_strategies};

const BASE: &str = "\
strategy moves_base on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21) and rsi(close, 14) < 30
size fixed 0.01
exit stop 2 % from entry
exit time 48 bars
";

fn base() -> Value {
    let out = compile_strategy(&json!({ "text": BASE }));
    out["detail"]["definition"].clone()
}

fn rsi_filter(n: u32, bound: &str) -> Value {
    json!({
        "filter": "compare",
        "lhs": {"operand": "feature", "family": "rsi", "args": [n]},
        "op": "lt",
        "rhs": {"operand": "number", "value": bound},
    })
}

/// One edit per move kind the tools can produce, with the kind it should be.
fn edits() -> Vec<(&'static str, Value)> {
    vec![
        (
            "change_threshold",
            json!({"edit": "threshold", "clause": "entry.long[0].clause[1]", "to": "45"}),
        ),
        (
            "add_filter",
            json!({
                "edit": "add_filter", "entry": "entry.long[0]",
                "filter": {
                    "filter": "compare",
                    "lhs": {"operand": "feature", "family": "atr", "args": [14]},
                    "op": "gt",
                    "rhs": {"operand": "number", "value": "1"},
                },
            }),
        ),
        (
            "remove_filter",
            json!({"edit": "remove_filter", "clause": "entry.long[0].clause[1]"}),
        ),
        (
            "swap_feature",
            json!({
                "edit": "replace_filter", "clause": "entry.long[0].clause[1]",
                "with": rsi_filter(21, "30"),
            }),
        ),
        ("change_timeframe", json!({"edit": "timeframe", "to": "4h"})),
        (
            "change_sizing",
            json!({"edit": "sizing", "to": {"size": "risk_per_trade", "fraction": "0.01"}}),
        ),
        (
            "add_exit",
            json!({"edit": "add_exit", "rule": {"exit": "target", "percent": "5"}}),
        ),
        (
            "change_exit",
            json!({
                "edit": "replace_exit", "index": 0,
                "with": {"exit": "trailing", "percent": "2"},
            }),
        ),
    ]
}

fn apply(def: &Value, edit: &Value) -> Value {
    let out = apply_move(&json!({ "definition": def, "edit": edit }));
    assert!(out.get("error").is_none(), "{edit} was refused: {out:#}");
    out
}

fn diff(before: &Value, after: &Value) -> Value {
    let out = diff_strategies(&json!({ "before": before, "after": after }));
    assert!(out.get("error").is_none(), "diff failed: {out:#}");
    out
}

#[test]
fn every_move_kind_round_trips_from_apply_move_through_diff_strategies() {
    let before = base();
    for (want, edit) in edits() {
        let applied = apply(&before, &edit);
        let kind = applied["detail"]["kind"].as_str().expect("a kind");
        assert_eq!(kind, want, "{edit} came back as `{kind}`");

        // A second tool call, with nothing but the two definitions.
        let after = &applied["detail"]["definition"];
        let d = diff(&before, after);
        assert_eq!(
            d["detail"]["is_single_change"], true,
            "`{want}`'s own output is not a single change: {d:#}"
        );
        assert_eq!(d["detail"]["n_moves"], 1);
        let seen = d["detail"]["changes"][0]["kind"].as_str().expect("a kind");
        assert_eq!(
            seen, want,
            "apply_move said `{want}` and diff_strategies said `{seen}`"
        );
    }
}

#[test]
fn a_two_move_version_is_not_a_single_change() {
    let before = base();
    let first = apply(&before, &edits()[0].1);
    let second = apply(
        &first["detail"]["definition"],
        &json!({"edit": "timeframe", "to": "4h"}),
    );
    let d = diff(&before, &second["detail"]["definition"]);
    assert_eq!(d["detail"]["is_single_change"], false, "{d:#}");
    assert_eq!(d["detail"]["n_moves"], 2, "{d:#}");
    // And each step on its own was one.
    assert_eq!(
        diff(&before, &first["detail"]["definition"])["detail"]["is_single_change"],
        true
    );
}

/// An edit that changes nothing is not a move, and the tool says so rather than
/// returning a version indistinguishable from its parent.
#[test]
fn an_edit_that_changes_nothing_is_refused_by_the_tool() {
    let before = base();
    let out = apply_move(&json!({
        "definition": before,
        "edit": {"edit": "threshold", "clause": "entry.long[0].clause[1]", "to": "30"},
    }));
    assert!(out["error"]["code"].is_string(), "{out:#}");
}

/// An address written by hand that is not one is refused at the boundary, not
/// defaulted to clause zero.
#[test]
fn a_malformed_address_is_refused_not_defaulted() {
    let out = apply_move(&json!({
        "definition": base(),
        "edit": {"edit": "threshold", "clause": "entry.long.1", "to": "45"},
    }));
    assert_eq!(out["error"]["code"], "unreadable_edit", "{out:#}");
}
