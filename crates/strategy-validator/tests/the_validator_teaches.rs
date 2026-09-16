//! A rejection says what to change (AGENT-011 §5; Set S task S-5.3).
//!
//! The acceptance, in its own terms: the six §5 error cases return the listed
//! `fix`, a repeated rejection on one path returns a worked example, and
//! `rejects_loosening` still refuses a widened risk envelope (that last one has
//! its own file and is unchanged).
//!
//! What this file adds beyond the unit tests in `teach.rs` is the end-to-end
//! half: that a **real definition**, rejected by the real validator, comes back
//! carrying the teaching. The table being right and the error carrying it are
//! two different claims, and the second is the one that was false — every
//! rejection this crate produced reached its caller as `{path, message}` with
//! the fix left behind.

use domain::strategy_def::StrategyDefinition;
use strategy_validator::validate;

fn def(json: &str) -> StrategyDefinition {
    serde_json::from_str(json).expect("the fixture parses as a definition")
}

/// The base: a definition that validates, so every case below differs from it
/// in exactly one way and the rejection can only be about that.
const GOOD: &str = r#"{
    "strategy_id": "teach_base_v1",
    "definition_version": "1.0",
    "asset_class": "crypto_spot_cex",
    "inputs": [
        { "lane": "market.bars.1m", "instrument": "$bound_at_init" },
        { "lane": "features.technical", "instrument": "$bound_at_init",
          "features": ["ema_10", "ema_30"] }
    ],
    "nodes": [
        { "id": "n1", "type": "condition", "expr": "feature('ema_10') > feature('ema_30')" },
        { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }
    ],
    "actions": [
        { "on_signal": "long", "type": "place_order",
          "order": { "side": "buy", "size_mode": "fixed", "size": "1" } }
    ]
}"#;

#[test]
fn the_base_fixture_validates() {
    validate(&def(GOOD)).expect("the base fixture must validate, or nothing below isolates");
}

/// Every rejection the validator produces carries a fix.
///
/// Walked over real rejections rather than over the table, because the table
/// being complete and the error carrying its entry are different properties.
#[test]
fn every_real_rejection_carries_a_fix() {
    let broken = [
        // `when` naming something that is not a condition node.
        GOOD.replace("\"when\": \"n1\"", "\"when\": \"feature('ema_10') > 0\""),
        // An action naming a signal nothing emits.
        GOOD.replace("\"on_signal\": \"long\"", "\"on_signal\": \"short\""),
        // Two nodes with one id.
        GOOD.replace("\"id\": \"n2\"", "\"id\": \"n1\""),
        // A version nothing parses.
        GOOD.replace("\"definition_version\": \"1.0\"", "\"definition_version\": \"9.9\""),
        // No slug.
        GOOD.replace("\"strategy_id\": \"teach_base_v1\"", "\"strategy_id\": \"\""),
    ];

    let mut seen = 0;
    for json in &broken {
        let Ok(parsed) = serde_json::from_str::<StrategyDefinition>(json) else {
            // A mutation the *parser* rejects never reaches the validator, and
            // this test is about the validator.
            continue;
        };
        let Err(errors) = validate(&parsed) else {
            continue;
        };
        for e in &errors {
            seen += 1;
            assert!(
                e.fix().is_some(),
                "`{}` at `{}` rejected with no fix. A rejection that says what is wrong and \
                 not what to change leaves a small model guessing, and every guess costs a turn.",
                e.code,
                e.path
            );
            assert!(
                !e.code.is_empty(),
                "a rejection at `{}` has an empty code, so nothing can branch on it",
                e.path
            );
        }
    }
    assert!(
        seen >= 4,
        "only {seen} rejections were produced, so this test is asserting almost nothing — the \
         mutations above are no longer breaking the definition"
    );
}

/// The `see` anchor points into the guide, in the form the guide uses.
#[test]
fn every_anchor_is_shaped_like_a_guide_anchor() {
    for t in strategy_validator::teach::TEACHINGS {
        let Some(see) = t.see else { continue };
        assert!(
            see.starts_with("guide#"),
            "`{}`'s anchor is `{see}`; the guide is served by `get_authoring_guide` and \
             rendered in several places, so a bare URL would be right in one of them",
            t.code
        );
        assert!(
            see.len() > "guide#".len(),
            "`{}`'s anchor names no section",
            t.code
        );
    }
}

/// A repeated rejection on one clause can offer a worked example.
///
/// §5's "three rejections on the same `path`" is counted by the caller — this
/// crate is a pure function of one document and has no run — so what is asserted
/// here is that the example exists and is about the right clause.
#[test]
fn a_rejected_clause_can_show_a_correct_instance() {
    let broken = GOOD.replace("\"when\": \"n1\"", "\"when\": \"nope\"");
    let parsed = def(&broken);
    let errors = validate(&parsed).expect_err("an unknown `when` target is rejected");
    let e = errors.first().expect("at least one error");

    let example = e
        .example()
        .expect("a rejection inside `nodes` can show a correct `nodes` block");
    assert!(
        example.contains("\"type\": \"signal\"") && example.contains("\"when\""),
        "the example offered for a `{}` rejection does not show the clause that failed: {example}",
        e.path
    );
    // It has to be copyable. An example a model pastes and cannot parse is worse
    // than none, because the next rejection is about the example.
    serde_json::from_str::<serde_json::Value>(&format!("{{{example}}}"))
        .expect("the worked example is valid JSON");
}
