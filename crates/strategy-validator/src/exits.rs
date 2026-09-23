//! The exit-policy pass (ADR-P6-01).
//!
//! `ValidatedDefinition` has no public constructor, so everything this pass
//! refuses is a document that cannot be persisted. That is what makes the rules
//! below type-level rather than advisory.
//!
//! Three things are checked, and one of them is the whole point:
//!
//! 1. **A 1.3 document must declare an exit policy.** Silence was the old
//!    behaviour and it was indistinguishable from a deliberate choice.
//! 2. **`Rules` must contain at least one rule.** An empty rule list is
//!    `RiskLayerOnly` wearing a disguise, and the two must not be able to hash
//!    differently while behaving identically.
//! 3. **A rule's parameters must be in range.** A stop at 150% never fires on a
//!    long and a stop at 0% fires on the entry bar; both are typos that a
//!    backtest renders as a result.
//!
//! A fourth rule is deliberately *not* here: nothing requires a strategy to have
//! exits. `RiskLayerOnly` stays legal, because a strategy whose entry signal
//! reverses cleanly genuinely does not need one. The requirement is that the
//! choice be stated.
//!
//! ## Units (1.4, ADR-D-09)
//!
//! From 1.4 a distance carries a [`DistanceUnit`]. Two checks come with it, and
//! both are version checks rather than value checks:
//!
//! 4. **A scaled unit needs 1.4.** A document that declares 1.3 and carries
//!    `unit: atr` is refused, for the same reason `exits_version_too_low` exists:
//!    a runtime reading it at its declared version has no unit field, would read
//!    the magnitude as a fraction of entry, and would place a 150%-of-entry stop
//!    where a 1.5-ATR stop was meant. That is the one failure mode worse than a
//!    refusal.
//! 5. **A lookback must be one the engine will compute.** `atr(0)` is not "no
//!    lookback", it is the feature name `atr_0`, which nothing serves — and which
//!    would be discovered after the strategy was stored.
//!
//! The `(0, 1]` bound on a fraction is *not* applied to a scaled magnitude:
//! `1.5` as a fraction of entry is a stop that can never fire on a long, and
//! `1.5 atr` is the most ordinary stop in the domain. Holding one bound over both
//! units is what made the language unable to say the second.

use domain::strategy_def::exits::{DistanceUnit, ExitDistance, ExitPolicy, ExitRule};
use domain::strategy_def::{StrategyDefinition, DEFINITION_VERSION_V1_3, DEFINITION_VERSION_V1_4};

use crate::version::DefinitionVersion;
use crate::ValidationError;

/// Validate the `exits` block.
#[must_use]
pub fn validate_exits(def: &StrategyDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Ordered as numbers, not as text. `"1.10" >= "1.3"` is false lexically and
    // true numerically, so the old string compare would have read a 1.10
    // document as pre-exits and refused its stop loss while leaving its entries
    // legal — the one direction that trades an unprotected position.
    //
    // A version that does not parse gets **no** verdict from this pass rather
    // than a defaulted one: `validate_schema` already refuses it by name, and a
    // second error guessing at what the author meant would be an absent
    // measurement reported as a reading.
    let Some(version) = DefinitionVersion::parse(&def.definition_version) else {
        return errors;
    };
    let is_v13_or_later = version
        >= DefinitionVersion::parse(DEFINITION_VERSION_V1_3).expect("1.3 is a valid version");

    match (&def.exits, is_v13_or_later) {
        (None, true) => errors.push(ValidationError {
            code: "exits_required",
            path: "exits".into(),
            message: format!(
                "definition_version {} requires an `exits` block. If the risk layer's position \
                 limits are the entire exit logic, say so: {{\"mode\":\"risk_layer_only\"}}. \
                 Leaving it out used to mean the same thing and looked like an oversight, which \
                 is why it is now refused.",
                def.definition_version
            ),
        }),
        (Some(_), false) => errors.push(ValidationError {
            code: "exits_version_too_low",
            path: "exits".into(),
            message: format!(
                "an `exits` block needs definition_version {DEFINITION_VERSION_V1_3} or later; \
                 this document declares {}. A runtime reading it as {} would ignore the exits \
                 and trade the entries anyway.",
                def.definition_version, def.definition_version
            ),
        }),
        (Some(policy), true) => errors.extend(validate_policy(policy, version)),
        (None, false) => {}
    }

    errors
}

fn validate_policy(policy: &ExitPolicy, version: DefinitionVersion) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if let ExitPolicy::Rules { rules } = policy {
        if rules.is_empty() {
            errors.push(ValidationError {
                code: "exits_empty_rules",
                path: "exits.rules".into(),
                message: "an exit policy of `rules` with an empty list is `risk_layer_only` \
                          under another name, and the two must not hash differently while \
                          behaving identically. Declare a rule, or declare \
                          {\"mode\":\"risk_layer_only\"}."
                    .into(),
            });
        }

        let is_v14_or_later = version
            >= DefinitionVersion::parse(DEFINITION_VERSION_V1_4).expect("1.4 is a valid version");

        for (i, rule) in rules.iter().enumerate() {
            if !rule.is_well_formed() {
                errors.push(ValidationError {
                    code: explain_code(rule),
                    path: format!("exits.rules[{i}]"),
                    message: explain(rule),
                });
            }
            // A unit the declared version cannot store. Refused rather than
            // downgraded: `unit: atr` dropped by a 1.3 reader leaves a magnitude
            // of `1.5` in a field that reader calls `fraction`, and a 150% stop
            // is not a near-miss of a 1.5-ATR stop.
            if !rule.unit().is_fraction_of_entry() && !is_v14_or_later {
                errors.push(ValidationError {
                    code: "exit_unit_version_too_low",
                    path: format!("exits.rules[{i}].unit"),
                    message: format!(
                        "`{}` measures its distance in `{}`, which needs definition_version \
                         {DEFINITION_VERSION_V1_4} or later; this document declares {version}. A \
                         runtime reading it at {version} has no `unit` field, would take `{}` for \
                         a fraction of the entry price, and would place a stop nothing like the \
                         one written.",
                        rule.kind(),
                        rule.unit(),
                        rule.distance()
                            .map_or_else(String::new, |d| d.magnitude.to_string()),
                    ),
                });
            }
        }

        // Two rules of the same kind **in the same unit** are almost always a
        // copy-paste: the first one shadows the second for every state, so the
        // second can never fire.
        //
        // The unit is part of that key on purpose. A 2% stop and a 1.5-ATR stop
        // are two stops where neither dominates — whichever is tighter on this
        // bar fires, which is how "stop at the tighter of the two" is written in
        // a first-match-wins policy. Keying on the kind alone would refuse a
        // legitimate and common rule.
        for (i, rule) in rules.iter().enumerate() {
            if let Some(j) = rules[..i]
                .iter()
                .position(|r| r.kind() == rule.kind() && r.unit() == rule.unit())
            {
                errors.push(ValidationError {
                    code: "exit_rule_shadowed",
                    path: format!("exits.rules[{i}]"),
                    message: format!(
                        "a second `{}` rule in `{}` after the one at index {j}: rules are \
                         evaluated in order and the first match wins, so whichever of the two is \
                         looser can never fire. Keep one. (Two stops in *different* units are \
                         legal — neither dominates.)",
                        rule.kind(),
                        rule.unit()
                    ),
                });
            }
        }
    }

    errors
}

/// The teaching code for a malformed rule.
///
/// Branches exactly where [`explain`] branches. One code for every rule kind
/// would be a code nobody can act on differently — a fraction out of range and a
/// zero holding bound are different edits — and one code for all of them would
/// make the fix generic enough to be useless.
fn explain_code(rule: &ExitRule) -> &'static str {
    match rule.distance() {
        // A bad lookback and a bad magnitude are different edits, so they are
        // different codes: `atr(0)` is fixed by writing a period, `0 atr(14)` by
        // writing a multiple.
        Some(d) if !d.unit.is_well_formed() => "exit_unit_period_out_of_range",
        Some(d) if d.unit.is_fraction_of_entry() => "exit_fraction_out_of_range",
        Some(_) => "exit_distance_not_positive",
        None => match rule {
            ExitRule::MaxHoldingBars { .. } => "exit_holding_bars_zero",
            _ => "exit_signal_name_empty",
        },
    }
}

fn explain(rule: &ExitRule) -> String {
    match rule {
        ExitRule::StopLoss { distance }
        | ExitRule::TakeProfit { distance }
        | ExitRule::TrailingStop { distance } => explain_distance(rule.kind(), distance),
        ExitRule::MaxHoldingBars { .. } => {
            "`max_holding_bars` must be at least 1: a bound of 0 closes the position on the bar \
             it opened, which is a round trip paid for nothing."
                .into()
        }
        ExitRule::OnSignal { .. } => {
            "`on_signal` needs a non-empty signal name to match against.".into()
        }
    }
}

/// Why one distance is malformed. Branches on the unit, because the legal range
/// of the magnitude depends on it.
fn explain_distance(kind: &str, d: &ExitDistance) -> String {
    if !d.unit.is_well_formed() {
        let period = d.unit.period().unwrap_or(0);
        return format!(
            "`{kind}` measures its distance in `{}` over a {period}-bar lookback, and the feature \
             engine computes `{}` for a lookback of {}..={} only. A period of {period} is not \
             \"no lookback\": it is a feature name nothing serves, discovered after the strategy \
             is stored.",
            d.unit.as_str(),
            d.unit.as_str(),
            domain::strategy_def::exits::MIN_UNIT_PERIOD,
            domain::strategy_def::exits::MAX_UNIT_PERIOD,
        );
    }
    match d.unit {
        DistanceUnit::FractionOfEntry => format!(
            "`{kind}` takes a fraction of the reference price in (0, 1]; got {}. A fraction \
             above 1 can never fire on a long, and 0 or below fires on the entry bar.",
            d.magnitude
        ),
        DistanceUnit::Atr { .. } | DistanceUnit::Sigma { .. } => format!(
            "`{kind}` takes a positive multiple of `{}`; got {}. There is no upper bound — a 5-ATR \
             stop is wide, not malformed — but zero or negative is a stop on the wrong side of \
             the entry.",
            d.unit, d.magnitude
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;
    use serde_json::json;

    fn def_at(version: &str, exits: Option<serde_json::Value>) -> StrategyDefinition {
        let mut v = json!({
            "strategy_id": "s",
            "definition_version": version,
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [{ "id": "n1", "type": "condition", "expr": "bar('close') > 1" },
                      { "id": "n2", "type": "signal", "when": "n1", "emit": "long" }],
            "actions": [{ "on_signal": "long", "type": "place_order",
                          "order": { "side": "buy", "size_mode": "fixed", "size": "0.01" } }]
        });
        if let Some(e) = exits {
            v["exits"] = e;
        }
        serde_json::from_value(v).unwrap()
    }

    fn messages(def: &StrategyDefinition) -> String {
        validate_exits(def)
            .iter()
            .map(|e| format!("{}: {}", e.path, e.message))
            .collect::<Vec<_>>()
            .join(" | ")
    }

    // ── the rule that closes the hole ────────────────────────────────────────

    /// The whole point: at 1.3, having no exits is a thing you say, not a thing
    /// that happens to you.
    #[test]
    fn a_v13_document_without_exits_is_refused() {
        let errs = validate_exits(&def_at("1.3", None));
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("risk_layer_only"), "{}", errs[0].message);
    }

    #[test]
    fn a_v13_document_that_delegates_to_the_risk_layer_is_accepted() {
        let d = def_at("1.3", Some(json!({ "mode": "risk_layer_only" })));
        assert!(validate_exits(&d).is_empty(), "{}", messages(&d));
    }

    /// Older documents are untouched. The migration is additive in the parser
    /// and the pass has to match that, or every stored strategy fails to load.
    #[test]
    fn pre_13_documents_are_not_required_to_have_exits() {
        for v in ["1.0", "1.1", "1.2"] {
            assert!(validate_exits(&def_at(v, None)).is_empty(), "version {v}");
        }
    }

    /// The reverse mistake: exits on a document whose declared version has no
    /// runtime support for them. A runtime reading it at 1.2 would trade the
    /// entries and ignore the exits, which is the worst of both.
    #[test]
    fn exits_on_a_pre_13_document_are_refused() {
        let d = def_at("1.2", Some(json!({ "mode": "risk_layer_only" })));
        let errs = validate_exits(&d);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("1.3 or later"), "{}", errs[0].message);
    }

    // ── empty rules ──────────────────────────────────────────────────────────

    #[test]
    fn an_empty_rule_list_is_refused_because_it_is_delegation_in_disguise() {
        let d = def_at("1.3", Some(json!({ "mode": "rules", "rules": [] })));
        let errs = validate_exits(&d);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].message.contains("under another name"), "{}", errs[0].message);
    }

    // ── well-formedness ──────────────────────────────────────────────────────

    #[test]
    fn a_stop_above_one_hundred_percent_is_refused() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [{ "rule": "stop_loss", "fraction": "1.5" }] })),
        );
        let m = messages(&d);
        assert!(m.contains("never fire on a long"), "{m}");
    }

    #[test]
    fn a_zero_stop_is_refused() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [{ "rule": "stop_loss", "fraction": "0" }] })),
        );
        assert!(messages(&d).contains("fires on the entry bar"));
    }

    #[test]
    fn a_zero_bar_time_exit_is_refused() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [{ "rule": "max_holding_bars", "bars": 0 }] })),
        );
        assert!(messages(&d).contains("round trip paid for nothing"));
    }

    #[test]
    fn a_nameless_signal_exit_is_refused() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [{ "rule": "on_signal", "emit": " " }] })),
        );
        assert!(messages(&d).contains("non-empty signal name"));
    }

    // ── shadowing ────────────────────────────────────────────────────────────

    /// Two stops means one of them can never fire. Almost always a paste.
    #[test]
    fn a_duplicated_rule_kind_is_refused_because_the_second_can_never_fire() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [
                { "rule": "stop_loss", "fraction": "0.02" },
                { "rule": "stop_loss", "fraction": "0.05" }
            ]})),
        );
        let m = messages(&d);
        assert!(m.contains("can never fire"), "{m}");
        assert!(m.contains("exits.rules[1]"), "{m}");
    }

    #[test]
    fn different_rule_kinds_together_are_fine() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [
                { "rule": "stop_loss", "fraction": "0.02" },
                { "rule": "take_profit", "fraction": "0.06" },
                { "rule": "trailing_stop", "fraction": "0.03" },
                { "rule": "max_holding_bars", "bars": 240 },
                { "rule": "on_signal", "emit": "flat" }
            ]})),
        );
        assert!(validate_exits(&d).is_empty(), "{}", messages(&d));
    }

    /// Every problem in one round trip — the validator's contract everywhere
    /// else, and this pass has to keep it.
    #[test]
    fn every_problem_is_reported_at_once() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [
                { "rule": "stop_loss", "fraction": "5" },
                { "rule": "stop_loss", "fraction": "0" },
                { "rule": "max_holding_bars", "bars": 0 }
            ]})),
        );
        // two malformed fractions + one zero bound + one shadowed duplicate
        assert_eq!(validate_exits(&d).len(), 4, "{}", messages(&d));
    }

    #[test]
    fn a_well_formed_boundary_fraction_of_exactly_one_is_accepted() {
        let d = def_at(
            "1.3",
            Some(json!({ "mode": "rules", "rules": [{ "rule": "stop_loss", "fraction": "1" }] })),
        );
        assert!(validate_exits(&d).is_empty(), "{}", messages(&d));
        let _ = dec!(1); // keep the macro import honest
    }
}
