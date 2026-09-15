//! Converts strategy `Action`s into `OrderIntent`s with idempotency keys.
//!
//! This is the **one** sizer.  Live evaluation and replay both call
//! [`build_intents_for_signals`], so a size computed here is the same size in
//! both (ADR-0008).
//!
//! ## Why this returns a `Result`
//!
//! It used to return `Option<OrderIntent>`, and `SizeMode::PercentOfBalance`
//! returned `None`.  `None` also means "the strategy deliberately did not
//! trade", so a strategy that declared percent-of-equity sizing produced no
//! order and no error — indistinguishable, from the outside, from a strategy
//! that decided to stay flat.  The two are now different values:
//! `Ok(None)` is an abstention and [`SizingError`] is a failure that is logged
//! and surfaced.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use rust_decimal::Decimal;
use thiserror::Error;

use domain::money::{Price, Size};
use domain::order::{OrderIntent, OrderType};
use domain::strategy_def::actions::{Action, ActionKind, SizeMode};

/// A size could not be computed.  Never silently swallowed: the caller logs it
/// and, on the live path, it is visible as a refused order rather than as
/// nothing happening.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SizingError {
    #[error("size '{size}' is not a valid decimal quantity")]
    UnparseableSize { size: String },

    #[error("percent-of-balance sizing needs account equity and a mark; {missing} was not available")]
    MissingSizingContext { missing: &'static str },

    #[error("percent-of-balance fraction '{fraction}' is not a positive decimal fraction")]
    InvalidFraction { fraction: String },

    #[error("percent-of-balance sizing produced a non-positive quantity ({qty}) — equity {equity}, mark {mark}, fraction {fraction}")]
    NonPositiveQuantity {
        qty: String,
        equity: String,
        mark: String,
        fraction: String,
    },

    /// `RiskUnit` sizing is "R" — the entry-to-stop distance — which has no
    /// meaning until exit/stop declarations exist (Set E 4.5, blocked on 4.4).
    /// It errors rather than returning no order, so a strategy declaring it is
    /// told, not ignored.
    #[error("'risk_unit' sizing is not executable: it needs a declared stop distance (Set E 4.5)")]
    RiskUnitNotExecutable,
}

/// Account and market state the sizer needs for equity-relative size modes.
///
/// Live: account equity from the paper account snapshot, mark from the mark
/// board.  Replay: per-bar equity from the SDK account, mark from the bar
/// close.  Locked decision 10 — total account equity (cash + open-position
/// value), the identical figure on both sides.
#[derive(Clone, Copy, Debug)]
pub struct SizingContext {
    pub account_equity: Decimal,
    pub mark_price: Price,
    /// Quantity increment to quantize to.  `Decimal::ZERO` means the
    /// instrument has no lot constraint (fractional crypto).
    pub lot_size: Decimal,
}

impl SizingContext {
    #[must_use]
    pub fn new(account_equity: Decimal, mark_price: Price, lot_size: Decimal) -> Self {
        Self {
            account_equity,
            mark_price,
            lot_size,
        }
    }
}

/// Round `qty` down to a whole multiple of `lot_size`.
///
/// Down, never nearest: rounding up would place an order larger than the
/// fraction of equity the strategy asked to risk.
fn quantize_down(qty: Decimal, lot_size: Decimal) -> Decimal {
    if lot_size.is_zero() || lot_size.is_sign_negative() {
        return qty;
    }
    (qty / lot_size).floor() * lot_size
}

/// Compute the quantity for one action, or say why it cannot be computed.
fn size_for(
    order: &domain::strategy_def::actions::OrderSpec,
    size_overrides: &HashMap<String, String>,
    sizing: Option<&SizingContext>,
) -> Result<Option<Size>, SizingError> {
    match &order.size_mode {
        SizeMode::Fixed => Size::from_str(&order.size)
            .map(Some)
            .map_err(|_| SizingError::UnparseableSize {
                size: order.size.clone(),
            }),

        SizeMode::PercentOfBalance => {
            let Some(ctx) = sizing else {
                return Err(SizingError::MissingSizingContext {
                    missing: "account equity / mark",
                });
            };
            let fraction = Decimal::from_str(order.size.trim()).map_err(|_| {
                SizingError::InvalidFraction {
                    fraction: order.size.clone(),
                }
            })?;
            if fraction <= Decimal::ZERO {
                return Err(SizingError::InvalidFraction {
                    fraction: order.size.clone(),
                });
            }
            let mark = ctx.mark_price.inner();
            if mark <= Decimal::ZERO {
                return Err(SizingError::MissingSizingContext {
                    missing: "a positive mark price",
                });
            }
            let raw = fraction * ctx.account_equity / mark;
            let qty = quantize_down(raw, ctx.lot_size);
            if qty <= Decimal::ZERO {
                return Err(SizingError::NonPositiveQuantity {
                    qty: qty.to_string(),
                    equity: ctx.account_equity.to_string(),
                    mark: mark.to_string(),
                    fraction: fraction.to_string(),
                });
            }
            Ok(Some(Size::from_decimal(qty)))
        }

        SizeMode::RiskUnit => Err(SizingError::RiskUnitNotExecutable),

        // Model sizing (v1.1): the size fraction comes from the referenced
        // Sizing node's InferenceOutput.size_fraction, supplied via
        // `size_overrides`.  Abstention (no entry) yields no order — this is
        // the one legitimate "no order, no error" outcome.
        SizeMode::Model { node_ref } => match size_overrides.get(node_ref) {
            None => Ok(None),
            Some(fraction) => Size::from_str(fraction).map(Some).map_err(|_| {
                SizingError::UnparseableSize {
                    size: fraction.clone(),
                }
            }),
        },
    }
}

/// Build an `OrderIntent` from an action spec and instance context.
///
/// `size_overrides` maps a `Sizing` node's ID → its resolved decimal-string
/// size fraction; it supplies the size for `SizeMode::Model` actions.
/// `sizing` supplies account equity and the mark for equity-relative modes;
/// pass `None` where no account state is available — percent-of-balance then
/// **errors** rather than silently producing nothing.
///
/// `Ok(None)` means the strategy abstained.  `Err` means a size was asked for
/// and could not be produced.
pub fn build_intent_from_action(
    action: &Action,
    instrument_id: &str,
    strategy_id: &str,
    size_overrides: &HashMap<String, String>,
    sizing: Option<&SizingContext>,
) -> Result<Option<OrderIntent>, SizingError> {
    let ActionKind::PlaceOrder { order } = &action.kind;
    let Some(size) = size_for(order, size_overrides, sizing)? else {
        return Ok(None);
    };
    Ok(Some(OrderIntent::new(
        instrument_id,
        order.side,
        OrderType::Market,
        size,
        None,
        Some(strategy_id.to_owned()),
    )))
}

/// The outcome of evaluating every action against this tick's signals.
#[derive(Debug, Default)]
pub struct SizedIntents {
    pub intents: Vec<OrderIntent>,
    /// Actions that asked for a size and could not get one.  The caller must
    /// surface these — dropping them is the defect this type exists to prevent.
    pub errors: Vec<(String, SizingError)>,
}

impl SizedIntents {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.intents.is_empty() && self.errors.is_empty()
    }
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
    sizing: Option<&SizingContext>,
) -> SizedIntents {
    let mut out = SizedIntents::default();
    for action in actions.iter().filter(|a| signals.contains(&a.on_signal)) {
        match build_intent_from_action(action, instrument_id, strategy_id, size_overrides, sizing) {
            Ok(Some(intent)) => out.intents.push(intent),
            Ok(None) => {}
            Err(e) => out.errors.push((action.on_signal.clone(), e)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::order::Side;
    use domain::strategy_def::actions::{ActionKind, OrderSpec, SizeMode};

    fn action(mode: SizeMode, size: &str) -> Action {
        Action {
            on_signal: "long".into(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: mode,
                    size: size.to_owned(),
                },
            },
        }
    }

    fn buy_fixed(size: &str) -> Action {
        action(SizeMode::Fixed, size)
    }

    fn buy_model(node_ref: &str) -> Action {
        action(
            SizeMode::Model {
                node_ref: node_ref.to_owned(),
            },
            "",
        )
    }

    fn ctx(equity: &str, mark: &str, lot: &str) -> SizingContext {
        SizingContext::new(
            Decimal::from_str(equity).unwrap(),
            Price::from_str(mark).unwrap(),
            Decimal::from_str(lot).unwrap(),
        )
    }

    #[test]
    fn fixed_size_produces_intent() {
        let intent = build_intent_from_action(
            &buy_fixed("0.01"),
            "BTC-USDT",
            "ema_cross_v1",
            &HashMap::new(),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(intent.instrument_id, "BTC-USDT");
        assert_eq!(intent.strategy_id.as_deref(), Some("ema_cross_v1"));
        assert!(!intent.idempotency_key.is_nil());
    }

    #[test]
    fn invalid_size_is_an_error_not_a_silent_abstention() {
        let err = build_intent_from_action(
            &buy_fixed("not_a_number"),
            "BTC-USDT",
            "s",
            &HashMap::new(),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, SizingError::UnparseableSize { .. }));
    }

    // ── PercentOfBalance (Set E 4.2) ─────────────────────────────────────────

    #[test]
    fn percent_of_balance_sizes_against_equity_and_mark() {
        // 10% of 100 000 equity at a mark of 50 000 = 0.2 units.
        let intent = build_intent_from_action(
            &action(SizeMode::PercentOfBalance, "0.10"),
            "BTC-USD",
            "s",
            &HashMap::new(),
            Some(&ctx("100000", "50000", "0")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(intent.size, Size::from_str("0.2").unwrap());
    }

    #[test]
    fn percent_of_balance_quantizes_down_to_the_lot() {
        // 0.2 units against a 0.15 lot floors to 0.15, never rounds up to 0.3.
        let intent = build_intent_from_action(
            &action(SizeMode::PercentOfBalance, "0.10"),
            "BTC-USD",
            "s",
            &HashMap::new(),
            Some(&ctx("100000", "50000", "0.15")),
        )
        .unwrap()
        .unwrap();
        assert_eq!(intent.size, Size::from_str("0.15").unwrap());
    }

    #[test]
    fn percent_of_balance_without_context_errors_loudly() {
        let err = build_intent_from_action(
            &action(SizeMode::PercentOfBalance, "0.10"),
            "BTC-USD",
            "s",
            &HashMap::new(),
            None,
        )
        .unwrap_err();
        assert!(
            matches!(err, SizingError::MissingSizingContext { .. }),
            "no account state must be an error, never a silent no-op"
        );
    }

    #[test]
    fn percent_of_balance_that_rounds_to_zero_errors() {
        // 1% of $10 at a mark of 50 000 with a 1-unit lot is zero units.
        // That must be refused, not submitted as a zero-size order.
        let err = build_intent_from_action(
            &action(SizeMode::PercentOfBalance, "0.01"),
            "BTC-USD",
            "s",
            &HashMap::new(),
            Some(&ctx("10", "50000", "1")),
        )
        .unwrap_err();
        assert!(matches!(err, SizingError::NonPositiveQuantity { .. }));
    }

    #[test]
    fn percent_of_balance_rejects_a_non_positive_fraction() {
        for f in ["0", "-0.1", "abc"] {
            let err = build_intent_from_action(
                &action(SizeMode::PercentOfBalance, f),
                "BTC-USD",
                "s",
                &HashMap::new(),
                Some(&ctx("100000", "50000", "0")),
            )
            .unwrap_err();
            assert!(
                matches!(err, SizingError::InvalidFraction { .. }),
                "fraction '{f}' must be refused"
            );
        }
    }

    #[test]
    fn risk_unit_says_why_it_cannot_run() {
        let err = build_intent_from_action(
            &action(SizeMode::RiskUnit, "1"),
            "BTC-USD",
            "s",
            &HashMap::new(),
            Some(&ctx("100000", "50000", "0")),
        )
        .unwrap_err();
        assert_eq!(err, SizingError::RiskUnitNotExecutable);
    }

    // ── Model sizing ─────────────────────────────────────────────────────────

    #[test]
    fn model_size_uses_override_fraction() {
        let mut overrides = HashMap::new();
        overrides.insert("size1".to_owned(), "0.025".to_owned());
        let intent =
            build_intent_from_action(&buy_model("size1"), "BTC-USDT", "s", &overrides, None)
                .unwrap()
                .unwrap();
        assert_eq!(intent.size, Size::from_str("0.025").unwrap());
    }

    #[test]
    fn model_size_abstains_without_override() {
        // The one legitimate "no order and no error": the Sizing node abstained.
        let out =
            build_intent_from_action(&buy_model("size1"), "BTC-USDT", "s", &HashMap::new(), None)
                .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn errors_are_returned_alongside_intents_not_dropped() {
        let actions = vec![buy_fixed("0.01"), action(SizeMode::RiskUnit, "1")];
        let mut signals = HashSet::new();
        signals.insert("long".to_owned());
        let out =
            build_intents_for_signals(&actions, &signals, "BTC-USD", "s", &HashMap::new(), None);
        assert_eq!(out.intents.len(), 1);
        assert_eq!(out.errors.len(), 1);
    }
}
