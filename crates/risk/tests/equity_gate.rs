//! P6-T03: Adversarial tests proving the risk gate correctly handles equity
//! session and halt constraints.
//!
//! The gate never branches on AssetClass.  The three checks below are driven
//! entirely by the `is_in_session`, `is_halted`, and `halt_policy` fields
//! of `GateContext` — values that the production caller pre-computes from
//! `TradingSchedule` and the exchange halt feed.

use std::str::FromStr;
use std::sync::Arc;

use domain::{
    instrument::HaltPolicy,
    money::Size,
    order::{OrderIntent, OrderType, Side},
    RiskRejection, TrustTier,
};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

fn gate() -> RiskGate {
    RiskGate::new(
        GlobalRiskLimits::default(),
        Arc::new(KillSwitch::new(false)),
    )
}

/// Equity context: Regulated trust, NYSE session.
/// `is_in_session` and `is_halted` are overridden per test.
fn equity_ctx(is_in_session: bool, is_halted: bool) -> GateContext {
    GateContext {
        current_position: Decimal::ZERO,
        market_price: None,
        tick_size: Decimal::from_str("0.01").unwrap(),
        lot_size: Decimal::ONE,
        daily_loss_usd: Decimal::ZERO,
        event_trust_tier: TrustTier::Regulated,
        strategy_min_trust_tier: TrustTier::Regulated,
        risk_overrides: Default::default(),
        instrument_active: true,
        recent_orders_last_second: 0,
        recent_orders_last_minute: 0,
        is_in_session,
        is_halted,
        halt_policy: HaltPolicy::Haltable,
    }
}

fn aapl_buy() -> OrderIntent {
    OrderIntent::new(
        "AAPL",
        Side::Buy,
        OrderType::Market,
        Size::from_str("10").unwrap(),
        None,
        None,
    )
}

// ── P6-T03-A: Order outside session ─────────────────────────────────────────

#[test]
fn equity_order_outside_session_is_rejected() {
    let err = gate()
        .check(aapl_buy(), &equity_ctx(false, false))
        .unwrap_err();
    assert!(
        matches!(err, RiskRejection::OutsideTradingHours { ref instrument_id }
            if instrument_id == "AAPL"),
        "expected OutsideTradingHours(AAPL), got {err:?}"
    );
}

// ── P6-T03-B: Order against a halted instrument ──────────────────────────────

#[test]
fn equity_order_during_halt_is_rejected() {
    let err = gate()
        .check(aapl_buy(), &equity_ctx(true, true))
        .unwrap_err();
    assert!(
        matches!(err, RiskRejection::InstrumentHalted { ref instrument_id }
            if instrument_id == "AAPL"),
        "expected InstrumentHalted(AAPL), got {err:?}"
    );
}

// ── P6-T03-C: Valid in-session, not-halted equity order approved ─────────────

#[test]
fn equity_order_in_session_not_halted_is_approved() {
    let result = gate().check(aapl_buy(), &equity_ctx(true, false));
    assert!(result.is_ok(), "expected approval, got {result:?}");
    assert_eq!(result.unwrap().intent.instrument_id, "AAPL");
}

// ── P6-T03-D: SPY variant — same gate, different symbol ─────────────────────

#[test]
fn spy_order_outside_session_is_rejected() {
    let intent = OrderIntent::new(
        "SPY",
        Side::Buy,
        OrderType::Market,
        Size::from_str("5").unwrap(),
        None,
        None,
    );
    let err = gate()
        .check(intent, &equity_ctx(false, false))
        .unwrap_err();
    assert!(
        matches!(err, RiskRejection::OutsideTradingHours { ref instrument_id }
            if instrument_id == "SPY"),
        "expected OutsideTradingHours(SPY), got {err:?}"
    );
}
