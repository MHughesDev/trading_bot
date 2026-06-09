//! P6-T04: Cross-asset abstraction proof.
//!
//! The same risk gate, the same `GlobalRiskLimits`, and the same `run_checks`
//! code path handles both a crypto instrument (BTC-USDT) and an equity
//! instrument (AAPL).  The only differences are values in `GateContext` that
//! callers derive from instrument metadata — the gate itself never inspects
//! `AssetClass`.
//!
//! This test suite is the compiler-enforced proof of that invariant: if the
//! gate ever added an `if asset_class == Equity { … }` branch, the gate API
//! would have to expose `AssetClass`, and this test file would need to import
//! it.  The fact that it compiles without any `AssetClass` import is the proof.

use std::str::FromStr;
use std::sync::Arc;

use domain::{
    instrument::HaltPolicy,
    money::Size,
    order::{OrderIntent, OrderType, Side},
    TrustTier,
};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

fn shared_gate() -> RiskGate {
    RiskGate::new(
        GlobalRiskLimits::default(),
        Arc::new(KillSwitch::new(false)),
    )
}

/// Context for a crypto instrument (BTC-USDT on Kraken).
/// 24/7 schedule: is_in_session always true; NonHaltable.
fn crypto_ctx() -> GateContext {
    GateContext {
        current_position: Decimal::ZERO,
        market_price: None,
        tick_size: Decimal::from_str("0.01").unwrap(),
        lot_size: Decimal::from_str("0.00001").unwrap(),
        daily_loss_usd: Decimal::ZERO,
        event_trust_tier: TrustTier::CentralizedExchange,
        strategy_min_trust_tier: TrustTier::CentralizedExchange,
        risk_overrides: Default::default(),
        instrument_active: true,
        recent_orders_last_second: 0,
        recent_orders_last_minute: 0,
        is_in_session: true,
        is_halted: false,
        halt_policy: HaltPolicy::NonHaltable,
    }
}

/// Context for an equity instrument (AAPL on Alpaca).
/// NYSE session: is_in_session must be explicitly supplied; Haltable.
fn equity_ctx(is_in_session: bool) -> GateContext {
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
        is_halted: false,
        halt_policy: HaltPolicy::Haltable,
    }
}

fn buy_intent(instrument_id: &str) -> OrderIntent {
    OrderIntent::new(
        instrument_id,
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    )
}

// ── Approval path ─────────────────────────────────────────────────────────────

#[test]
fn crypto_buy_approved_through_shared_gate() {
    let gate = shared_gate();
    let result = gate.check(buy_intent("BTC-USDT"), &crypto_ctx());
    assert!(
        result.is_ok(),
        "BTC-USDT buy should be approved: {result:?}"
    );
    assert_eq!(result.unwrap().intent.instrument_id, "BTC-USDT");
}

#[test]
fn equity_buy_in_session_approved_through_shared_gate() {
    let gate = shared_gate();
    let result = gate.check(buy_intent("AAPL"), &equity_ctx(true));
    assert!(
        result.is_ok(),
        "AAPL buy in-session should be approved: {result:?}"
    );
    assert_eq!(result.unwrap().intent.instrument_id, "AAPL");
}

// ── Both instruments use the same gate instance ───────────────────────────────

#[test]
fn single_gate_approves_both_asset_classes() {
    let gate = shared_gate();

    let crypto_result = gate.check(buy_intent("BTC-USDT"), &crypto_ctx());
    let equity_result = gate.check(buy_intent("AAPL"), &equity_ctx(true));

    assert!(
        crypto_result.is_ok(),
        "crypto should pass: {crypto_result:?}"
    );
    assert!(
        equity_result.is_ok(),
        "equity should pass: {equity_result:?}"
    );
}

// ── Rejection path — each asset class fails for asset-specific reasons ────────

#[test]
fn equity_outside_session_rejected_crypto_unaffected() {
    let gate = shared_gate();

    // Equity order during market hours: approved.
    let in_session = gate.check(buy_intent("AAPL"), &equity_ctx(true));
    assert!(in_session.is_ok());

    // Equity order outside market hours: rejected.
    let out_session = gate.check(buy_intent("AAPL"), &equity_ctx(false));
    assert!(
        out_session.is_err(),
        "equity outside session must be rejected"
    );

    // Crypto order always passes (24/7, NonHaltable).
    let crypto = gate.check(buy_intent("BTC-USDT"), &crypto_ctx());
    assert!(
        crypto.is_ok(),
        "crypto must not be affected by equity session rules"
    );
}

// ── Kill switch kills both asset classes equally ──────────────────────────────

#[test]
fn kill_switch_blocks_both_asset_classes() {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));

    ks.trip();

    use domain::RiskRejection;
    let crypto_err = gate
        .check(buy_intent("BTC-USDT"), &crypto_ctx())
        .unwrap_err();
    let equity_err = gate
        .check(buy_intent("AAPL"), &equity_ctx(true))
        .unwrap_err();

    assert!(matches!(crypto_err, RiskRejection::KillSwitchActive));
    assert!(matches!(equity_err, RiskRejection::KillSwitchActive));
}
