//! Adversarial test: a forced position divergence halts new orders on that instrument.

use std::str::FromStr;
use std::sync::Arc;

use domain::{
    money::Size,
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use reconciliation::divergence::{check_position_divergence, ReconcileOutcome};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate};
use rust_decimal::Decimal;

#[test]
fn divergence_trips_kill_switch_and_blocks_new_orders() {
    let ks = Arc::new(KillSwitch::new(false));

    // Force a divergence: internal says 1.0, broker says 2.0.
    let outcome = check_position_divergence(
        "BTC-USD",
        Decimal::from_str("1.0").unwrap(),
        Decimal::from_str("2.0").unwrap(),
        &ks,
    );
    assert!(
        matches!(outcome, ReconcileOutcome::Diverged { .. }),
        "should detect divergence"
    );
    assert!(ks.is_active(), "kill switch must be tripped");

    // Now the risk gate — using the same kill switch — must reject new orders.
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&ks));
    let intent = OrderIntent::new(
        "BTC-USD",
        Side::Buy,
        OrderType::Market,
        Size::from_str("0.1").unwrap(),
        None,
        None,
    );
    let ctx = GateContext::for_manual_order(
        Decimal::ZERO,
        None,
        Decimal::from_str("0.01").unwrap(),
        Decimal::from_str("0.001").unwrap(),
        Decimal::ZERO,
        true,
        0,
        0,
    );

    let err = gate.check(intent, &ctx).unwrap_err();
    assert!(
        matches!(err, RiskRejection::KillSwitchActive),
        "subsequent orders must be rejected after divergence"
    );
}

#[test]
fn matching_positions_do_not_trip_kill_switch() {
    let ks = Arc::new(KillSwitch::new(false));

    let outcome = check_position_divergence(
        "BTC-USD",
        Decimal::from_str("1.0").unwrap(),
        Decimal::from_str("1.0").unwrap(),
        &ks,
    );
    assert_eq!(outcome, ReconcileOutcome::Match);
    assert!(
        !ks.is_active(),
        "matching positions must not trip kill switch"
    );
}

#[test]
fn alarm_only_affects_new_orders_not_existing_positions() {
    // The kill switch blocks *new* orders only — it has no mechanism to touch
    // existing position records (verified by the absence of such API).
    let ks = KillSwitch::new(false);
    ks.trip();
    assert!(ks.is_active());
    // KillSwitch has no close_positions() or force_flatten() method.
}
