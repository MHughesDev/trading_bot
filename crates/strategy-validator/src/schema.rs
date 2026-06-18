//! Structural validator for the frozen v1.0 strategy definition format.
//!
//! Checks:
//! - `definition_version` == "1.0"
//! - `strategy_id` is non-empty
//! - All node IDs are unique within the definition
//! - Every `signal.when` references an existing `condition` node ID
//! - Every `action.on_signal` matches some `signal.emit` value

use std::collections::HashSet;

use domain::strategy_def::{nodes::NodeKind, StrategyDefinition};

use crate::ValidationError;

/// Validate the structural correctness of a definition.
pub fn validate_schema(def: &StrategyDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if def.definition_version != "1.0" && def.definition_version != "1.1" {
        errors.push(ValidationError {
            path: "definition_version".into(),
            message: format!(
                "expected '1.0' or '1.1', got '{}'; only v1.0/v1.1 definitions are accepted",
                def.definition_version
            ),
        });
    }

    if def.strategy_id.trim().is_empty() {
        errors.push(ValidationError {
            path: "strategy_id".into(),
            message: "must not be empty".into(),
        });
    }

    // Collect IDs and check uniqueness.
    let mut seen_ids: HashSet<&str> = HashSet::new();
    for node in &def.nodes {
        if !seen_ids.insert(node.id.as_str()) {
            errors.push(ValidationError {
                path: format!("nodes[{}].id", node.id),
                message: format!("duplicate node id '{}'", node.id),
            });
        }
    }

    // Nodes a `signal.when` may reference: anything that produces a boolean.
    // That is both expression `Condition` nodes and v1.1 `ModelForecast` nodes
    // (the runtime evaluates ModelForecast into the same condition results map).
    let condition_ids: HashSet<&str> = def
        .nodes
        .iter()
        .filter_map(|n| {
            if matches!(
                n.kind,
                NodeKind::Condition { .. } | NodeKind::ModelForecast { .. }
            ) {
                Some(n.id.as_str())
            } else {
                None
            }
        })
        .collect();

    // Validate v1.1 ModelForecast node fields.
    for node in &def.nodes {
        if let NodeKind::ModelForecast {
            model_ref,
            direction,
            min_confidence,
            ..
        } = &node.kind
        {
            if model_ref.trim().is_empty() {
                errors.push(ValidationError {
                    path: format!("nodes[{}].model_ref", node.id),
                    message: "must not be empty; select a model".into(),
                });
            }
            if !matches!(direction.as_str(), "bullish" | "bearish" | "any") {
                errors.push(ValidationError {
                    path: format!("nodes[{}].direction", node.id),
                    message: format!(
                        "expected 'bullish', 'bearish', or 'any', got '{direction}'"
                    ),
                });
            }
            if !(0.0..=1.0).contains(min_confidence) {
                errors.push(ValidationError {
                    path: format!("nodes[{}].min_confidence", node.id),
                    message: format!("must be between 0.0 and 1.0, got {min_confidence}"),
                });
            }
        }
    }

    let signal_emits: HashSet<&str> = def
        .nodes
        .iter()
        .filter_map(|n| {
            if let NodeKind::Signal { emit, .. } = &n.kind {
                Some(emit.as_str())
            } else {
                None
            }
        })
        .collect();

    // Every signal.when must reference an existing condition node.
    for node in &def.nodes {
        if let NodeKind::Signal { when, .. } = &node.kind {
            if !condition_ids.contains(when.as_str()) {
                errors.push(ValidationError {
                    path: format!("nodes[{}].when", node.id),
                    message: format!(
                        "references unknown condition node '{}'; \
                         add a condition node with that id",
                        when
                    ),
                });
            }
        }
    }

    // Every action.on_signal must match a signal emit.
    for (i, action) in def.actions.iter().enumerate() {
        if !signal_emits.contains(action.on_signal.as_str()) {
            errors.push(ValidationError {
                path: format!("actions[{i}].on_signal"),
                message: format!(
                    "references unknown signal '{}'; \
                     add a signal node that emits this name",
                    action.on_signal
                ),
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_def::{
        actions::{Action, ActionKind, OrderSpec, SizeMode},
        inputs::InputDeclaration,
        nodes::{Node, NodeKind},
        risk_overrides::RiskOverrides,
        StrategyDefinition,
    };
    use domain::{order::Side, TrustTier};

    fn minimal() -> StrategyDefinition {
        StrategyDefinition {
            strategy_id: "test".into(),
            definition_version: "1.0".into(),
            asset_class: "crypto_spot_cex".into(),
            min_trust_tier: TrustTier::CentralizedExchange,
            inputs: vec![InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            }],
            nodes: vec![
                Node {
                    id: "c1".into(),
                    kind: NodeKind::Condition {
                        expr: "feature('ema_7') > feature('ema_21')".into(),
                    },
                },
                Node {
                    id: "s1".into(),
                    kind: NodeKind::Signal {
                        when: "c1".into(),
                        emit: "long".into(),
                    },
                },
            ],
            actions: vec![Action {
                on_signal: "long".into(),
                kind: ActionKind::PlaceOrder {
                    order: OrderSpec {
                        side: Side::Buy,
                        size_mode: SizeMode::Fixed,
                        size: "0.01".into(),
                    },
                },
            }],
            risk_overrides: RiskOverrides::default(),
        }
    }

    #[test]
    fn valid_definition_passes() {
        assert!(validate_schema(&minimal()).is_empty());
    }

    #[test]
    fn wrong_version_rejected() {
        let mut d = minimal();
        d.definition_version = "2.0".into();
        let errs = validate_schema(&d);
        assert!(errs.iter().any(|e| e.path == "definition_version"));
    }

    fn with_model_node(direction: &str, min_confidence: f64, model_ref: &str) -> StrategyDefinition {
        let mut d = minimal();
        d.definition_version = "1.1".into();
        d.nodes = vec![
            Node {
                id: "m1".into(),
                kind: NodeKind::ModelForecast {
                    model_ref: model_ref.into(),
                    alias: "production".into(),
                    direction: direction.into(),
                    min_confidence,
                },
            },
            Node {
                id: "s1".into(),
                kind: NodeKind::Signal {
                    when: "m1".into(),
                    emit: "long".into(),
                },
            },
        ];
        d
    }

    #[test]
    fn v1_1_model_forecast_passes() {
        // A signal watching a ModelForecast node is valid, and v1.1 is accepted.
        assert!(validate_schema(&with_model_node("bullish", 0.6, "btc_forecaster")).is_empty());
    }

    #[test]
    fn model_forecast_bad_fields_rejected() {
        let errs = validate_schema(&with_model_node("sideways", 1.5, "   "));
        assert!(errs.iter().any(|e| e.path.ends_with("direction")));
        assert!(errs.iter().any(|e| e.path.ends_with("min_confidence")));
        assert!(errs.iter().any(|e| e.path.ends_with("model_ref")));
    }

    #[test]
    fn empty_strategy_id_rejected() {
        let mut d = minimal();
        d.strategy_id = "   ".into();
        let errs = validate_schema(&d);
        assert!(errs.iter().any(|e| e.path == "strategy_id"));
    }

    #[test]
    fn duplicate_node_id_rejected() {
        let mut d = minimal();
        d.nodes.push(Node {
            id: "c1".into(),
            kind: NodeKind::Condition {
                expr: "bar('close') > 0.0".into(),
            },
        });
        let errs = validate_schema(&d);
        assert!(errs.iter().any(|e| e.path.contains("c1")));
    }

    #[test]
    fn signal_referencing_missing_condition_rejected() {
        let mut d = minimal();
        d.nodes.push(Node {
            id: "s2".into(),
            kind: NodeKind::Signal {
                when: "missing".into(),
                emit: "exit".into(),
            },
        });
        let errs = validate_schema(&d);
        assert!(errs.iter().any(|e| e.path.contains("s2")));
    }

    #[test]
    fn action_referencing_missing_signal_rejected() {
        let mut d = minimal();
        d.actions.push(Action {
            on_signal: "exit".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Sell,
                    size_mode: SizeMode::Fixed,
                    size: "0.01".into(),
                },
            },
        });
        let errs = validate_schema(&d);
        assert!(errs.iter().any(|e| e.path.contains("on_signal")));
    }
}
