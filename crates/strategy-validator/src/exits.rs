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

use domain::strategy_def::exits::{ExitPolicy, ExitRule};
use domain::strategy_def::{StrategyDefinition, DEFINITION_VERSION_V1_3};

use crate::ValidationError;

/// Validate the `exits` block.
#[must_use]
pub fn validate_exits(def: &StrategyDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let is_v13_or_later = def.definition_version.as_str() >= DEFINITION_VERSION_V1_3;

    match (&def.exits, is_v13_or_later) {
        (None, true) => errors.push(ValidationError {
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
            path: "exits".into(),
            message: format!(
                "an `exits` block needs definition_version {DEFINITION_VERSION_V1_3} or later; \
                 this document declares {}. A runtime reading it as {} would ignore the exits \
                 and trade the entries anyway.",
                def.definition_version, def.definition_version
            ),
        }),
        (Some(policy), true) => errors.extend(validate_policy(policy)),
        (None, false) => {}
    }

    errors
}

fn validate_policy(policy: &ExitPolicy) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if let ExitPolicy::Rules { rules } = policy {
        if rules.is_empty() {
            errors.push(ValidationError {
                path: "exits.rules".into(),
                message: "an exit policy of `rules` with an empty list is `risk_layer_only` \
                          under another name, and the two must not hash differently while \
                          behaving identically. Declare a rule, or declare \
                          {\"mode\":\"risk_layer_only\"}."
                    .into(),
            });
        }

        for (i, rule) in rules.iter().enumerate() {
            if !rule.is_well_formed() {
                errors.push(ValidationError {
                    path: format!("exits.rules[{i}]"),
                    message: explain(rule),
                });
            }
        }

        // Two rules of the same kind are almost always a copy-paste: the first
        // one shadows the second for every state, so the second can never fire.
        for (i, rule) in rules.iter().enumerate() {
            if let Some(j) = rules[..i].iter().position(|r| r.kind() == rule.kind()) {
                errors.push(ValidationError {
                    path: format!("exits.rules[{i}]"),
                    message: format!(
                        "a second `{}` rule after the one at index {j}: rules are evaluated in \
                         order and the first match wins, so whichever of the two is looser can \
                         never fire. Keep one.",
                        rule.kind()
                    ),
                });
            }
        }
    }

    errors
}

fn explain(rule: &ExitRule) -> String {
    match rule {
        ExitRule::StopLoss { fraction }
        | ExitRule::TakeProfit { fraction }
        | ExitRule::TrailingStop { fraction } => format!(
            "`{}` takes a fraction of the reference price in (0, 1]; got {fraction}. A fraction \
             above 1 can never fire on a long, and 0 or below fires on the entry bar.",
            rule.kind()
        ),
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
