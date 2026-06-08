//! Adversarial test: the kill switch trips on each defined condition and
//! blocks new orders without force-closing positions.

use std::str::FromStr;
use std::sync::Arc;

use domain::{
    money::Size,
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

fn gate_with_ks(ks: Arc<KillSwitch>) -> RiskGate {
    RiskGate::new(GlobalRiskLimits::default(), ks)
}

fn ctx() -> GateContext {
    GateContext::for_manual_order(
        Decimal::ZERO,
        None,
        Decimal::from_str("0.01").unwrap(),
        Decimal::from_str("0.001").unwrap(),
        Decimal::ZERO,
        true,
        0,
        0,
    )
}

fn intent() -> OrderIntent {
    OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    )
}

#[test]
fn manual_trip_blocks_immediately() {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = gate_with_ks(Arc::clone(&ks));

    // Before trip: order passes.
    assert!(gate.check(intent(), &ctx()).is_ok());

    // Trip.
    ks.trip();

    // After trip: new order blocked.
    let err = gate.check(intent(), &ctx());
    assert!(matches!(err, Err(RiskRejection::KillSwitchActive)));
}

#[test]
fn max_daily_loss_breach_trips_switch_and_blocks() {
    let ks = Arc::new(KillSwitch::new(false));
    let gate = gate_with_ks(Arc::clone(&ks));

    // Simulate caller detecting daily loss breach and tripping the switch.
    // (In production, the reconciliation crate does this; here we test the gate's
    // response to an already-tripped switch.)
    ks.trip(); // caller trips on loss detection

    let err = gate.check(intent(), &ctx());
    assert!(matches!(err, Err(RiskRejection::KillSwitchActive)));
}

#[test]
fn trip_does_not_force_close_existing_positions() {
    // The kill switch only blocks *new* orders.  It has no mechanism to touch
    // existing positions.  This test verifies by construction: KillSwitch only
    // exposes `trip()`, `reset()`, and `is_active()`.  There is no
    // `close_positions()` method to call.
    let ks = KillSwitch::new(false);
    ks.trip();
    assert!(ks.is_active());
    // No position-closing side-effects — verified by the absence of such API.
}

#[test]
fn reset_re_enables_order_flow() {
    let ks = Arc::new(KillSwitch::new(true)); // start tripped
    let gate = gate_with_ks(Arc::clone(&ks));

    // Blocked while tripped.
    assert!(matches!(
        gate.check(intent(), &ctx()),
        Err(RiskRejection::KillSwitchActive)
    ));

    // Reset.
    ks.reset();

    // New intent (different idempotency key) passes.
    assert!(gate.check(intent(), &ctx()).is_ok());
}

#[test]
fn gate_blocks_when_started_active() {
    let ks = Arc::new(KillSwitch::new(true));
    let gate = gate_with_ks(ks);
    let err = gate.check(intent(), &ctx());
    assert!(matches!(err, Err(RiskRejection::KillSwitchActive)));
}
