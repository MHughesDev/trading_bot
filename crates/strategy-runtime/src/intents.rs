//! Converts strategy `Action`s into `OrderIntent`s with idempotency keys.
//!
//! `SizeMode::Fixed` and `SizeMode::Model` (v1.1 AI sizing) are executable.
//! `PercentOfBalance` and `RiskUnit` parse successfully but are not yet
//! executable.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use domain::money::Size;
use domain::order::{OrderIntent, OrderType};
use domain::strategy_def::actions::{Action, ActionKind, SizeMode};

/// Build an `OrderIntent` from an action spec and instance context.
///
/// `size_overrides` maps a `Sizing` node's ID → its resolved decimal-string
/// size fraction; it supplies the size for `SizeMode::Model` actions.
///
/// Returns `None` for unsupported size modes, an unparseable size, or a
/// `Model` size whose referenced node produced no fraction (abstained).
pub fn build_intent_from_action(
    action: &Action,
    instrument_id: &str,
    strategy_id: &str,
    size_overrides: &HashMap<String, String>,
) -> Option<OrderIntent> {
    let ActionKind::PlaceOrder { order } = &action.kind;
    let size = match &order.size_mode {
        SizeMode::Fixed => Size::from_str(&order.size).ok()?,
        // PercentOfBalance and RiskUnit are v1.0 parse-only; not executable yet.
        SizeMode::PercentOfBalance | SizeMode::RiskUnit => return None,
        // Model sizing (v1.1): the size fraction comes from the referenced
        // Sizing node's InferenceOutput.size_fraction, supplied via
        // `size_overrides`.  Abstention (no entry) yields no order.
        SizeMode::Model { node_ref } => {
            let fraction = size_overrides.get(node_ref)?;
            Size::from_str(fraction).ok()?
        }
    };
    Some(OrderIntent::new(
        instrument_id,
        order.side,
        OrderType::Market,
        size,
        None,
        Some(strategy_id.to_owned()),
    ))
}

/// Build intents for all actions that match any of the emitted `signals`.
///
/// `size_overrides` carries resolved `Sizing` node fractions (node ID → decimal
/// string) for `SizeMode::Model` actions; pass an empty map when none apply.
pub fn build_intents_for_signals(
    actions: &[Action],
    signals: &HashSet<String>,
    instrument_id: &str,
    strategy_id: &str,
    size_overrides: &HashMap<String, String>,
) -> Vec<OrderIntent> {
    actions
        .iter()
        .filter(|a| signals.contains(&a.on_signal))
        .filter_map(|a| build_intent_from_action(a, instrument_id, strategy_id, size_overrides))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::order::Side;
    use domain::strategy_def::actions::{ActionKind, OrderSpec, SizeMode};

    fn buy_fixed(size: &str) -> Action {
        Action {
            on_signal: "long".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: SizeMode::Fixed,
                    size: size.to_owned(),
                },
            },
        }
    }

    fn buy_model(node_ref: &str) -> Action {
        Action {
            on_signal: "long".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: SizeMode::Model {
                        node_ref: node_ref.to_owned(),
                    },
                    // `size` is ignored for Model sizing.
                    size: String::new(),
                },
            },
        }
    }

    #[test]
    fn fixed_size_produces_intent() {
        let action = buy_fixed("0.01");
        let intent =
            build_intent_from_action(&action, "BTC-USDT", "ema_cross_v1", &HashMap::new()).unwrap();
        assert_eq!(intent.instrument_id, "BTC-USDT");
        assert_eq!(intent.strategy_id.as_deref(), Some("ema_cross_v1"));
        assert!(!intent.idempotency_key.is_nil());
    }

    #[test]
    fn invalid_size_returns_none() {
        let action = buy_fixed("not_a_number");
        assert!(build_intent_from_action(&action, "BTC-USDT", "s", &HashMap::new()).is_none());
    }

    #[test]
    fn percent_of_balance_returns_none() {
        let action = Action {
            on_signal: "long".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: SizeMode::PercentOfBalance,
                    size: "0.10".into(),
                },
            },
        };
        assert!(build_intent_from_action(&action, "BTC-USDT", "s", &HashMap::new()).is_none());
    }

    #[test]
    fn model_size_uses_override_fraction() {
        let action = buy_model("size1");
        let mut overrides = HashMap::new();
        overrides.insert("size1".to_owned(), "0.025".to_owned());
        let intent =
            build_intent_from_action(&action, "BTC-USDT", "s", &overrides).expect("intent");
        assert_eq!(intent.size, Size::from_str("0.025").unwrap());
    }

    #[test]
    fn model_size_abstains_without_override() {
        let action = buy_model("size1");
        // No entry for "size1" → the Sizing node abstained → no order.
        assert!(build_intent_from_action(&action, "BTC-USDT", "s", &HashMap::new()).is_none());
    }
}
