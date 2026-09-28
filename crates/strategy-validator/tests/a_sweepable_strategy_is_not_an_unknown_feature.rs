//! A parameterised feature name is not a misspelled one.
//!
//! # The defect
//!
//! `validate()` runs five passes. Four of them read the definition
//! **materialized with its parameter defaults**, which is how the frozen v1
//! expression grammar never had to learn about `param('x')` or `{{x}}`. The
//! feature-name pass (S-5.3) read the *raw* definition instead, so it saw the
//! name as written:
//!
//! ```text
//! inputs[1].features[0] = "ema_{{fast}}"
//! ```
//!
//! `features::resolve_feature("ema_{{fast}}")` is `None`, so every definition
//! with a swept indicator period came back `unknown_feature`.
//!
//! `{{param}}` interpolation is the whole of v1.2, `strategy-lang`'s compiler
//! emits it for any `input x = ema(close, fast)`, and three of the four worked
//! skeletons use it — so the pass refused **exactly the strategies a sweep can
//! run**, and the persist route runs `validate()` before it stores anything. A
//! strategy with a hard-coded period validated; the same strategy made sweepable
//! did not.
//!
//! Measured while writing this, and worth recording because it is the opposite
//! of what it looked like: the refusal offered **no** repair. `nearest` compares
//! against each family's advertised example, `ema_{{fast}}` is twelve characters
//! against `ema_20`'s six, and the distance cap is three — so the message fell
//! through to "Call `list_features` to see what the engine computes". The author
//! was told their swept feature does not exist and pointed at a catalogue that
//! would not have explained why.
//!
//! # Why the fixtures are written out here
//!
//! `strategy-validator` does not depend on `strategy-lang` (nothing in the
//! workspace makes it, and adding the edge is not this crate's call), so the
//! documents below are hand-written in the shape the compiler emits rather than
//! compiled. `crates/strategy-lang/tests/the_json_a_human_would_have_written.rs`
//! is the test that holds the compiler to that shape; if these two ever
//! disagree, that one is the authority.

use strategy_validator::validate;

/// One `features.technical` input, one condition over it, and whatever
/// `parameters` block the caller passes.
fn definition(features: &str, expr: &str, parameters: &str) -> serde_json::Value {
    serde_json::from_str(&format!(
        r#"{{
            "strategy_id": "swept",
            "definition_version": "1.2",
            "asset_class": "crypto_spot_cex",
            "inputs": [
                {{ "lane": "market.bars.1h", "instrument": "$bound_at_init" }},
                {{ "lane": "features.technical", "instrument": "$bound_at_init",
                   "features": {features} }}
            ],
            "nodes": [
                {{ "id": "e0_cond", "type": "condition", "expr": "{expr}" }},
                {{ "id": "e0_sig", "type": "signal", "when": "e0_cond", "emit": "long" }}
            ],
            "actions": [
                {{ "on_signal": "long", "type": "place_order",
                   "order": {{ "side": "buy", "size_mode": "fixed", "size": "0.01" }} }}
            ],
            "parameters": {parameters}
        }}"#
    ))
    .expect("the fixture parses")
}

fn errors(def: &serde_json::Value) -> Vec<strategy_validator::ValidationError> {
    let def = serde_json::from_value(def.clone()).expect("a definition");
    validate(&def).err().unwrap_or_default()
}

const FAST: &str = r#"{ "fast": { "type": "int", "default": 7, "min": 5, "max": 50, "step": 1 } }"#;

#[test]
fn a_swept_indicator_period_is_not_an_unknown_feature() {
    let def = definition(r#"["ema_{{fast}}"]"#, "feature('ema_{{fast}}') > 0", FAST);
    let errs = errors(&def);
    assert!(
        !errs.iter().any(|e| e.code == "unknown_feature"),
        "`ema_{{{{fast}}}}` resolves to `ema_7` once the defaults are substituted, which is what \
         every other pass reads: {errs:?}"
    );
    assert!(
        errs.is_empty(),
        "and nothing else refuses a swept strategy either: {errs:?}"
    );
}

/// The other half of S-5.12's acceptance, as far as this crate can carry it:
/// **the skeletons validate.** This is the document `strategy_skeleton("trend")`
/// compiles to, at v1.3, with two swept parameters, a constraint over them, an
/// interpolated feature name in both the `inputs` list and the condition, and an
/// exits block — i.e. every v1.2 and v1.3 mechanism at once. It was refused
/// before this change, which is the "SLv2 authoring path is dead at its last
/// step" finding, one step further along than the version gate.
#[test]
fn the_trend_skeleton_validates() {
    let def: domain::strategy_def::StrategyDefinition = serde_json::from_str(
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
    )
    .expect("the fixture parses");
    let errs = validate(&def).err().unwrap_or_default();
    assert!(
        errs.is_empty(),
        "the platform's own worked starting point does not validate: {errs:?}"
    );
}

/// Substituting the defaults must not become a way to stop checking. A family
/// name that is misspelled around a real parameter is still a name the engine
/// will not resolve during the run, which is the whole reason this pass exists.
#[test]
fn a_misspelled_family_around_a_real_parameter_is_still_refused() {
    let def = definition(r#"["emaa_{{fast}}"]"#, "feature('emaa_{{fast}}') > 0", FAST);
    let errs = errors(&def);
    let e = errs
        .iter()
        .find(|e| e.code == "unknown_feature")
        .unwrap_or_else(|| panic!("`emaa_7` does not resolve and was accepted: {errs:?}"));
    assert_eq!(
        e.path, "inputs[1].features[0]",
        "the path must still name the input the author wrote, not a position in a rewritten copy"
    );
    assert!(
        e.message.contains("emaa_7"),
        "the message should name the resolvable form the engine will actually be asked for, so \
         the author can see what their parameter expanded to: {}",
        e.message
    );
}

/// A name with no parameter in it is unaffected: the measured case S-5.3 exists
/// for still fails, at the same path, with the same repair.
#[test]
fn the_case_this_pass_was_built_for_still_fails() {
    let def = definition(r#"["rsi14"]"#, "feature('rsi14') > 0", "{}");
    let errs = errors(&def);
    let e = errs
        .iter()
        .find(|e| e.code == "unknown_feature")
        .unwrap_or_else(|| panic!("`rsi14` was accepted: {errs:?}"));
    assert_eq!(e.path, "inputs[1].features[0]");
    assert!(e.message.contains("rsi_14"), "{}", e.message);
}

/// A reference to a parameter that was never declared is still an error — just
/// not this one. It is `params::validate_declarations`' answer, and letting the
/// feature pass shout `unknown_feature` over the top of it would send the author
/// to the wrong clause.
#[test]
fn an_undeclared_parameter_is_reported_as_a_parameter_and_not_as_a_feature() {
    let def = definition(r#"["ema_{{nope}}"]"#, "feature('ema_{{nope}}') > 0", "{}");
    let errs = errors(&def);
    assert!(
        errs.iter().any(|e| e.code == "parameter_out_of_range"),
        "an undeclared `{{{{nope}}}}` is a parameter error: {errs:?}"
    );
    assert!(
        !errs.iter().any(|e| e.code == "unknown_feature"),
        "and not also a feature error, which would point at the wrong clause: {errs:?}"
    );
}
