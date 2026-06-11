//! Converts strategy `Action`s into `OrderIntent`s with idempotency keys.
//!
//! Only `SizeMode::Fixed` is supported in v1.0; other modes parse successfully
//! but are not yet executable.

use std::collections::HashSet;
use std::str::FromStr;

use domain::money::Size;
use domain::order::{OrderIntent, OrderType};
use domain::strategy_def::actions::{Action, ActionKind, SizeMode};

/// Build an `OrderIntent` from an action spec and instance context.
///
/// Returns `None` for unsupported size modes (parse-only fields in v1.0).
pub fn build_intent_from_action(
    action: &Action,
    instrument_id: &str,
    strategy_id: String,
) -> Option<OrderIntent> {
    let ActionKind::PlaceOrder { order } = &action.kind;
    match order.size_mode {
        SizeMode::Fixed => {
            let size = Size::from_str(&order.size).ok()?;
            Some(OrderIntent::new(
                instrument_id,
                order.side,
                OrderType::Market,
                size,
                None,
                Some(strategy_id),
            ))
        }
        // PercentOfBalance and RiskUnit are v1.0 parse-only; not executable yet.
        SizeMode::PercentOfBalance | SizeMode::RiskUnit => None,
    }
}

/// Build intents for all actions that match any of the emitted `signals`.
pub fn build_intents_for_signals(
    actions: &[Action],
    signals: &HashSet<String>,
    instrument_id: &str,
    strategy_id: &str,
) -> Vec<OrderIntent> {
    actions
        .iter()
        .filter(|a| signals.contains(&a.on_signal))
        .filter_map(|a| build_intent_from_action(a, instrument_id, strategy_id.to_owned()))
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

    #[test]
    fn fixed_size_produces_intent() {
        let action = buy_fixed("0.01");
        let intent =
            build_intent_from_action(&action, "BTC-USDT", "ema_cross_v1".to_owned()).unwrap();
        assert_eq!(intent.instrument_id, "BTC-USDT");
        assert_eq!(intent.strategy_id.as_deref(), Some("ema_cross_v1"));
        assert!(!intent.idempotency_key.is_nil());
    }

    #[test]
    fn invalid_size_returns_none() {
        let action = buy_fixed("not_a_number");
        assert!(build_intent_from_action(&action, "BTC-USDT", "s".to_owned()).is_none());
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
        assert!(build_intent_from_action(&action, "BTC-USDT", "s".to_owned()).is_none());
    }
}
