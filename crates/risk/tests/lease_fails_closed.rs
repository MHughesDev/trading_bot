//! COMP-007 §4, §8.1, §8.2 — the order gate fails **closed** on the monitor lease.
//!
//! These are the adversarial half of the fail-closed design. It is easy to write a
//! monitor that halts trading when it decides to; the property that matters is that
//! trading stops when the monitor is *not there to decide anything*, and that no
//! later caller can switch that off.
//!
//! ```bash
//! cargo test -j 2 -p risk --test lease_fails_closed
//! ```

use std::str::FromStr;
use std::sync::Arc;

use domain::Price;
use domain::{
    money::Size,
    order::{OrderIntent, OrderType, Side},
    RiskRejection,
};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate, Scope};
use rust_decimal::Decimal;

fn ctx() -> GateContext {
    GateContext::for_manual_order(
        Decimal::ZERO,
        // A real mark. The gate fails closed without one — a limit that cannot
        // fire must refuse, never default — so `None` here refuses every order
        // for `MarketDataUnavailable` and these tests would pass or fail on
        // something other than the lease.
        Some(Price::from_str("50000").unwrap()),
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

fn gate(ks: &Arc<KillSwitch>) -> RiskGate {
    RiskGate::new(GlobalRiskLimits::default(), Arc::clone(ks))
}

#[test]
fn a_gate_with_no_monitor_seen_refuses_every_order() {
    // The single most important assertion in this track. Arm the lease and see
    // nothing: orders stop. Not "log a warning", not "carry on until told
    // otherwise" — stop.
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    let err = gate(&ks).check(intent(), &ctx());
    assert!(
        matches!(err, Err(RiskRejection::MonitorUnavailable)),
        "expected monitor_unavailable, got {err:?}"
    );
}

#[test]
fn losing_the_lease_stops_order_flow() {
    // COMP-007 §8.1 / §8.2 from the runtime's side: kill the monitor, the lease
    // lapses, the next order is refused. No coordination, no message, no shared
    // process — the absence of a renewal is the whole signal.
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    ks.observe_lease(true);
    let g = gate(&ks);
    assert!(
        g.check(intent(), &ctx()).is_ok(),
        "a live lease permits orders"
    );

    ks.observe_lease(false); // the monitor died
    assert!(matches!(
        g.check(intent(), &ctx()),
        Err(RiskRejection::MonitorUnavailable)
    ));
}

#[test]
fn a_restarted_monitor_does_not_resume_orders_until_its_lease_is_valid_again() {
    // §8.1's second sentence. The dangerous shape is a restart that clears the
    // refusal because *something* came back, rather than because a lease was
    // actually observed unexpired.
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    ks.observe_lease(true);
    ks.observe_lease(false);
    let g = gate(&ks);
    assert!(g.check(intent(), &ctx()).is_err());
    // A restart with a still-expired lease: still refused.
    ks.observe_lease(false);
    assert!(matches!(
        g.check(intent(), &ctx()),
        Err(RiskRejection::MonitorUnavailable)
    ));
    // Only an observed, valid lease reopens the gate.
    ks.observe_lease(true);
    assert!(g.check(intent(), &ctx()).is_ok());
}

#[test]
fn a_redelivered_approved_order_is_refused_once_the_lease_is_gone() {
    // The idempotency cache is a fast path that returns a prior *approval*. If it
    // skipped the lease check, a JetStream redelivery would walk straight past a
    // dead monitor — and redeliveries are exactly what happens when a platform is
    // in trouble.
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    ks.observe_lease(true);
    let g = gate(&ks);
    let order = intent();
    assert!(g.check(order.clone(), &ctx()).is_ok());

    ks.observe_lease(false);
    assert!(matches!(
        g.check(order, &ctx()),
        Err(RiskRejection::MonitorUnavailable)
    ));
}

#[test]
fn the_lease_arm_is_one_way_and_there_is_no_code_that_undoes_it() {
    // Two halves, because either one alone is easy to defeat.
    //
    // Behavioural: nothing in the public API returns an armed switch to
    // "not required".
    let ks = KillSwitch::new(false);
    ks.require_monitor_lease();
    ks.observe_lease(true);
    ks.observe_lease(false);
    ks.require_monitor_lease();
    ks.reset(); // the global reset must not touch the lease
    ks.trip();
    ks.reset();
    assert!(ks.lease_required(), "the lease gate must stay armed");
    assert!(!ks.monitor_available());

    // Structural: no method exists that could. A future edit adding
    // `disarm_monitor_lease` would pass every behavioural test above and quietly
    // reintroduce the flag the spec says must not exist, so the source is scanned
    // for one.
    let src = include_str!("../src/kill_switch.rs");
    let code: String = src
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "fn disarm",
        "fn unrequire",
        "fn clear_lease",
        "fn set_lease",
        "fn disable_lease",
        "fn unarm",
    ] {
        assert!(
            !code.contains(forbidden),
            "kill_switch.rs must contain no way to disarm the lease; found {forbidden:?}"
        );
    }
    // Exactly one place puts the gate in the not-required state: the constructor.
    assert_eq!(
        code.matches("AtomicU8::new(LEASE_NOT_REQUIRED)").count(),
        1,
        "only the constructor may put the lease gate in the not-required state"
    );
    // The lease moves only by compare-exchange, so there is no plain store that
    // could write an arbitrary value into it.
    assert!(
        !code.contains("self.lease.store("),
        "the lease must only move by compare-exchange, never by a plain store"
    );
    // And the only two values `observe_lease` can write are VALID and MISSING.
    assert!(
        code.contains("let next = if valid { LEASE_VALID } else { LEASE_MISSING };"),
        "observe_lease must be unable to write anything but VALID or MISSING"
    );
}

#[test]
fn a_drill_trip_refuses_no_real_order() {
    // COMP-007 §4.2: the rehearsal asks the gate and the gate answers. If a drill
    // could refuse real flow, nobody would ever run one on a live book — which is
    // the situation drills exist to fix.
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    ks.observe_lease(true);
    let g = gate(&ks);

    ks.trip_scope(&Scope::drill("global"));
    assert!(
        ks.is_active_scope(&Scope::drill("global")),
        "the drill state must be observable, or the rehearsal proves nothing"
    );
    assert!(
        g.check(intent(), &ctx()).is_ok(),
        "a drill must refuse nothing real"
    );
}

#[test]
fn a_venue_halt_stops_that_venue_and_leaves_the_others_trading() {
    // COMP-007 §8.3, at the gate.
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    ks.observe_lease(true);
    let g = gate(&ks);

    ks.trip_scope(&Scope::venue("kraken"));
    let on_kraken = ctx().on_venue("kraken");
    let on_alpaca = ctx().on_venue("alpaca");
    assert!(matches!(
        g.check(intent(), &on_kraken),
        Err(RiskRejection::KillSwitchActive)
    ));
    assert!(g.check(intent(), &on_alpaca).is_ok());
}

#[test]
fn a_strategy_halt_stops_that_strategy_and_leaves_the_others_trading() {
    let ks = Arc::new(KillSwitch::new(false));
    ks.require_monitor_lease();
    ks.observe_lease(true);
    let g = gate(&ks);

    ks.trip_scope(&Scope::strategy("s-1"));
    assert!(matches!(
        g.check(intent(), &ctx().from_strategy("s-1")),
        Err(RiskRejection::KillSwitchActive)
    ));
    assert!(g.check(intent(), &ctx().from_strategy("s-2")).is_ok());
    assert!(
        g.check(intent(), &ctx()).is_ok(),
        "a manual order is not a strategy"
    );
}

#[test]
fn the_lease_is_checked_before_the_halt_so_the_two_are_distinguishable() {
    // An operator staring at a refusal needs to know whether somebody stopped the
    // platform or whether the monitor is missing. If both reported
    // KillSwitchActive, the reset they reach for would be the wrong one.
    let ks = Arc::new(KillSwitch::new(true)); // globally halted
    ks.require_monitor_lease(); // and no monitor
    let err = gate(&ks).check(intent(), &ctx());
    assert!(
        matches!(err, Err(RiskRejection::MonitorUnavailable)),
        "the missing monitor is the more fundamental fact and is reported first"
    );
}
