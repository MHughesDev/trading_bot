//! Adversarial test: the gate must not double-approve a redelivered intent.

use std::str::FromStr;
use std::sync::Arc;

use domain::{
    money::Size,
    order::{OrderIntent, OrderType, Side},
};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

fn gate() -> RiskGate {
    RiskGate::new(
        GlobalRiskLimits::default(),
        Arc::new(KillSwitch::new(false)),
    )
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

#[test]
fn redelivered_approved_intent_returns_approved_without_rerunning() {
    let gate = gate();
    let intent = OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    );
    let key = intent.idempotency_key;

    // First submission — must be approved.
    let first = gate.check(intent.clone(), &ctx());
    assert!(first.is_ok(), "first check should be approved");
    assert_eq!(first.unwrap().intent.idempotency_key, key);

    // Second submission with same key — must return the cached approval.
    let mut intent2 = intent.clone();
    intent2.idempotency_key = key; // same key (simulating NATS redelivery)
    let second = gate.check(intent2, &ctx());
    assert!(
        second.is_ok(),
        "redelivered intent should still be approved"
    );
    assert_eq!(second.unwrap().intent.idempotency_key, key);
}

#[test]
fn redelivered_rejected_intent_returns_same_rejection() {
    let gate = gate();
    let intent = OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    );
    let key = intent.idempotency_key;

    // First submission — inject a condition that forces rejection.
    let mut bad_ctx = ctx();
    bad_ctx.instrument_active = false;

    let first = gate.check(intent.clone(), &bad_ctx);
    assert!(first.is_err(), "should be rejected");

    // Second submission — same key, now with a *valid* context — must still be rejected
    // (cached decision wins; the gate does not re-run checks).
    let mut intent2 = intent;
    intent2.idempotency_key = key;
    let second = gate.check(intent2, &ctx()); // valid context this time
    assert!(
        second.is_err(),
        "redelivered rejected intent must remain rejected"
    );
}

#[test]
fn different_keys_are_independent_decisions() {
    let gate = gate();

    let intent_a = OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        None,
    );
    let mut intent_b = intent_a.clone();
    intent_b.idempotency_key = uuid::Uuid::new_v4(); // distinct key

    assert!(gate.check(intent_a, &ctx()).is_ok());
    assert!(gate.check(intent_b, &ctx()).is_ok());
}
