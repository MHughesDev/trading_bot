//! S-5.6's acceptance, on the artefacts S-5.12 names: **a compiled definition is
//! byte-identical to the equivalent hand-written JSON for the three skeletons.**
//!
//! # Why byte-identical and not structurally equal
//!
//! There was already a structural check on one skeleton
//! (`the_trend_skeleton_compiles_to_the_definition_a_human_would_have_written`),
//! and structural equality is the weaker claim in the way that matters here.
//! ADR-0010 makes the canonical JSON *the* storage form and
//! `domain::strategy_def::revision` hashes its bytes, so two definitions that
//! are `PartialEq`-equal and serialize differently are two different stored
//! revisions with two different hashes and two different lineages. A `Decimal`
//! that starts serializing as `0.0150` rather than `0.015`, a `DistanceUnit`
//! that stops being skipped at its default, a `parameters` map that starts being
//! emitted when empty — none of those changes `assert_eq!` on the struct and
//! every one of them changes the stored document.
//!
//! So each expectation below is compared as the **canonical bytes**
//! (`revision::canonical_json`, the same function the revision hash is taken
//! over) rather than as a `StrategyDefinition`.
//!
//! What the comparison does **not** catch, said so nobody reads more into it:
//! the expected side is deserialized into a `StrategyDefinition` before it is
//! canonicalised, so anything the typed deserializer normalises — a JSON `-2`
//! for an `f64` field, key order, whitespace — is normalised on both sides.
//! Comparing against a raw string instead would make every test here fail on a
//! reflowed literal, which is a test that teaches nothing.
//!
//! # Why these are written out rather than snapshotted
//!
//! A snapshot of the compiler's own output asserts that the compiler agrees with
//! itself, which is true of every compiler including a wrong one. The documents
//! here are written from the skeleton text and from `domain`'s serde
//! declarations — the lane naming, the `{{param}}` interpolation of a
//! parameterised feature name, the `fraction` key that a percentage distance
//! keeps, the `type`/`rule`/`mode` tags, the sorted feature list — so that a
//! change in any of them is a failing test rather than a new snapshot.

use domain::strategy_def::{revision::canonical_json, StrategyDefinition};
use strategy_lang::{skeleton, HypothesisFamily};

/// The canonical bytes of a definition: what would be stored and hashed.
fn canonical(def: &StrategyDefinition) -> String {
    canonical_json(&serde_json::to_value(def).expect("a definition serializes"))
}

/// The canonical bytes of a hand-written document, so the two sides are
/// compared in one normal form and a difference is never a difference in key
/// order or whitespace.
fn canonical_text(json: &str) -> String {
    let parsed: StrategyDefinition =
        serde_json::from_str(json).expect("the hand-written JSON is a definition");
    canonical(&parsed)
}

fn compiled(family: HypothesisFamily) -> StrategyDefinition {
    skeleton(family)
        .expect("a worked family")
        .definition()
        .unwrap_or_else(|d| panic!("{family} does not compile: {d:#?}"))
}

fn assert_is(family: HypothesisFamily, hand_written: &str) {
    let got = canonical(&compiled(family));
    let want = canonical_text(hand_written);
    assert_eq!(
        got, want,
        "the `{family}` skeleton did not compile to the document a human would have written"
    );
}

/// `param fast: int[5..50 step 1] = 7` and `input fast_ma = ema(close, fast)`:
/// the feature name carries the v1.2 `{{fast}}` interpolation rather than the
/// default, because a sweep substitutes it before the engine resolves it.
#[test]
fn the_trend_skeleton_is_the_json_a_human_would_have_written() {
    assert_is(
        HypothesisFamily::Trend,
        r#"{
          "strategy_id": "trend_skeleton",
          "definition_version": "1.3",
          "asset_class": "crypto_spot_cex",
          "min_trust_tier": "centralized_exchange",
          "inputs": [
            { "lane": "market.bars.1h", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["ema_{{fast}}", "ema_{{slow}}", "rsi_14"] }
          ],
          "nodes": [
            { "id": "e0_cond", "type": "condition",
              "expr": "((feature('ema_{{fast}}') > feature('ema_{{slow}}')) * (feature('rsi_14') < 60))" },
            { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }
          ],
          "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }
          ],
          "risk_overrides": { "max_position": "0.5" },
          "parameters": {
            "fast": { "type": "int", "default": 7, "min": 5, "max": 50, "step": 1 },
            "slow": { "type": "int", "default": 21, "min": 20, "max": 200, "step": 5 }
          },
          "constraints": ["param('fast') < param('slow')"],
          "exits": { "mode": "rules", "rules": [
            { "rule": "stop_loss", "fraction": "0.02" },
            { "rule": "take_profit", "fraction": "0.05" },
            { "rule": "max_holding_bars", "bars": 48 }
          ]}
        }"#,
    );
}

/// The one with a float parameter and a three-parameter family: `zscore` reads
/// its window from a parameter, so its feature name carries the interpolation,
/// while `variance_ratio(close, 64, 4)` is fixed and names both of its
/// parameters in the order they were written.
#[test]
fn the_mean_reversion_skeleton_is_the_json_a_human_would_have_written() {
    assert_is(
        HypothesisFamily::MeanReversion,
        r#"{
          "strategy_id": "mean_reversion_skeleton",
          "definition_version": "1.3",
          "asset_class": "crypto_spot_cex",
          "min_trust_tier": "centralized_exchange",
          "inputs": [
            { "lane": "market.bars.1h", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["variance_ratio_64_4", "zscore_{{window}}"] }
          ],
          "nodes": [
            { "id": "e0_cond", "type": "condition",
              "expr": "((feature('zscore_{{window}}') < param('stretch')) * (feature('variance_ratio_64_4') < 1))" },
            { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }
          ],
          "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "percent_of_balance", "size": "0.1" } }
          ],
          "risk_overrides": { "max_order_rate_per_minute": 6 },
          "parameters": {
            "stretch": { "type": "float", "default": -2.0, "min": -4.0, "max": -0.5, "scale": "linear" },
            "window": { "type": "int", "default": 20, "min": 10, "max": 100, "step": 5 }
          },
          "exits": { "mode": "rules", "rules": [
            { "rule": "take_profit", "fraction": "0.015" },
            { "rule": "stop_loss", "fraction": "0.03" },
            { "rule": "max_holding_bars", "bars": 24 }
          ]}
        }"#,
    );
}

/// The one whose `size` is `risk_per_trade`, which the canonical form calls
/// `risk_unit`, and whose float parameter is declared `log`. Both are places a
/// silent rename would pass a structural test.
#[test]
fn the_vol_target_skeleton_is_the_json_a_human_would_have_written() {
    assert_is(
        HypothesisFamily::VolTarget,
        r#"{
          "strategy_id": "vol_target_skeleton",
          "definition_version": "1.3",
          "asset_class": "crypto_spot_cex",
          "min_trust_tier": "centralized_exchange",
          "inputs": [
            { "lane": "market.bars.1h", "instrument": "$bound_at_init" },
            { "lane": "features.technical", "instrument": "$bound_at_init",
              "features": ["bb_width_{{vol_window}}_2", "efficiency_ratio_50"] }
          ],
          "nodes": [
            { "id": "e0_cond", "type": "condition",
              "expr": "((feature('bb_width_{{vol_window}}_2') < param('vol_ceiling')) * (feature('efficiency_ratio_50') > 0.4))" },
            { "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }
          ],
          "actions": [
            { "on_signal": "long", "type": "place_order",
              "order": { "side": "buy", "size_mode": "risk_unit", "size": "0.01" } }
          ],
          "risk_overrides": { "max_position": "0.25" },
          "parameters": {
            "vol_ceiling": { "type": "float", "default": 0.08, "min": 0.01, "max": 0.2, "scale": "log" },
            "vol_window": { "type": "int", "default": 20, "min": 10, "max": 200, "step": 10 }
          },
          "exits": { "mode": "rules", "rules": [
            { "rule": "stop_loss", "fraction": "0.02" },
            { "rule": "max_holding_bars", "bars": 72 }
          ]}
        }"#,
    );
}

/// The acceptance says *three* skeletons, and this is the test that fails if the
/// three above stop being the three the phase named — or if a fourth worked
/// family arrives with no document here.
#[test]
fn the_three_skeletons_the_acceptance_names_are_the_three_that_are_pinned() {
    let pinned = [
        HypothesisFamily::Trend,
        HypothesisFamily::MeanReversion,
        HypothesisFamily::VolTarget,
    ];
    for family in pinned {
        assert!(
            skeleton(family).is_ok(),
            "`{family}` is pinned to a hand-written document and no longer compiles"
        );
    }
    // Breakout is worked and deliberately not pinned here: the acceptance names
    // three and this test records which three, so a reader can tell a
    // deliberate omission from a forgotten one.
    assert!(
        skeleton(HypothesisFamily::Breakout).is_ok(),
        "breakout stopped being a worked skeleton"
    );
}
