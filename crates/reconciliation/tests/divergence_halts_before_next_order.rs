//! T-0.3, the drill: a planted divergence stops order submission **before** the
//! next order, counted at the venue adapter (`artifact.md` SC-6, FM-4).
//!
//! The halt is the deliverable. A report that says "we diverged" after the next
//! order went out is exactly the failure this exists to prevent, so the
//! assertion is not "a halt was recorded" — it is that the number of orders the
//! broker saw did not go up.
//!
//! Nothing here can cheat: `ApprovedOrder` is sealed inside `crates/risk` and
//! `ExecutionEngine::submit` takes one, so the only way an order reaches the
//! adapter in this test is through the gate that consults the halt registry.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::forward::ObservationSource;
use domain::money::{Price, Size};
use domain::numeraire::Numeraire;
use domain::order::{OrderIntent, OrderType, Side};
use domain::RiskRejection;
use execution::mock_broker::MockBroker;
use execution::ExecutionEngine;
use reconciliation::forward::{
    DeploymentSpec, DivergenceThresholds, ExpectedBar, PaperVenue,
};
use reconciliation::{DivergenceVerdict, ForwardRecorder};
use risk::{GateContext, GlobalRiskLimits, HumanActor, KillSwitch, RiskGate};
use rust_decimal::Decimal;
use uuid::Uuid;

const STRATEGY: &str = "sma-cross-1";
const BAR: i64 = 60;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn px(s: &str) -> Price {
    Price::from_str(s).unwrap()
}

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).unwrap()
}

fn spec() -> DeploymentSpec {
    DeploymentSpec {
        deployment_id: Uuid::new_v4(),
        strategy_id: STRATEGY.to_owned(),
        trial_id: Uuid::new_v4(),
        venue: PaperVenue::AlpacaPaper,
        numeraire: Numeraire::Usd,
        bar_seconds: BAR,
        thresholds: DivergenceThresholds {
            max_abs_return_divergence: 0.01,
            max_cum_return_divergence: 0.03,
            max_position_divergence: dec("0.000001"),
            min_signal_match: 0.99,
        },
    }
}

fn ctx() -> GateContext {
    GateContext::for_manual_order(
        Decimal::ZERO,
        Some(px("100")),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        true,
        0,
        0,
    )
}

fn intent() -> OrderIntent {
    OrderIntent::new(
        "AAPL",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        Some(STRATEGY.to_owned()),
    )
}

/// Everything a drill needs: the one gate, the one engine, and the adapter that
/// counts what actually reached a venue.
struct Rig {
    gate: RiskGate,
    engine: ExecutionEngine,
    broker: Arc<MockBroker>,
    kill_switch: Arc<KillSwitch>,
}

fn rig() -> Rig {
    let kill_switch = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), Arc::clone(&kill_switch));
    let broker = Arc::new(MockBroker::new());
    let engine = ExecutionEngine::new(Arc::clone(&broker) as Arc<dyn execution::broker::Broker>);
    Rig {
        gate,
        engine,
        broker,
        kill_switch,
    }
}

/// Submit through the one chokepoint. Returns whether the order reached the
/// adapter. There is no second path in this test, by construction.
async fn try_submit(rig: &Rig) -> Result<(), RiskRejection> {
    let approved = rig.gate.check(intent(), &ctx())?;
    rig.engine.submit(approved).await.expect("mock broker");
    Ok(())
}

#[tokio::test]
async fn a_planted_position_divergence_stops_the_next_order_at_the_gate() {
    let rig = rig();
    let recorder = ForwardRecorder::new(
        spec(),
        dec("100000"),
        Arc::clone(rig.gate.halt_registry()),
    )
    .unwrap();

    // One order goes out normally.
    try_submit(&rig).await.expect("the first order is fine");
    assert_eq!(rig.broker.submit_call_count(), 1);

    // Plant it: internally flat, the broker says we are long 2.
    let verdict = recorder.reconcile_position("AAPL", dec("2"), &rig.kill_switch);
    assert!(verdict.is_halted(), "a 2-lot disagreement must halt");

    // The next order does not reach the adapter.
    //
    // A *position* divergence trips the global kill switch as well as halting
    // the strategy — the disagreement is about the account, not only about this
    // strategy's simulator fidelity — and the kill switch is checked first, so
    // that is the rejection the caller sees. Either refusal is the deliverable;
    // what must not happen is an order.
    let err = try_submit(&rig).await.expect_err("the next order must be refused");
    assert!(
        matches!(
            err,
            RiskRejection::KillSwitchActive | RiskRejection::StrategyHalted { .. }
        ),
        "expected the order to be refused on divergence, got {err:?}"
    );
    assert_eq!(
        rig.broker.submit_call_count(),
        1,
        "no further order may reach the venue adapter after a divergence"
    );

    // And the per-strategy halt is in force independently, so clearing the
    // global switch alone does not resume a strategy whose book nobody has
    // reconciled.
    assert!(rig.gate.halt_registry().is_halted(STRATEGY));
    rig.kill_switch.reset();
    let err = try_submit(&rig)
        .await
        .expect_err("resetting the kill switch must not resume a halted strategy");
    assert!(matches!(err, RiskRejection::StrategyHalted { .. }), "{err:?}");
    assert_eq!(rig.broker.submit_call_count(), 1);
}

#[tokio::test]
async fn a_planted_return_divergence_stops_the_next_order_at_the_gate() {
    let rig = rig();
    let recorder = ForwardRecorder::new(
        spec(),
        dec("100000"),
        Arc::clone(rig.gate.halt_registry()),
    )
    .unwrap();

    try_submit(&rig).await.expect("the first order is fine");
    assert_eq!(rig.broker.submit_call_count(), 1);

    // Two bars. The first establishes the equity base; the second moves the
    // mark 5% against a backtest that expected nothing, which is five times the
    // declared per-bar threshold.
    recorder
        .close_bar(
            at(BAR),
            px("100"),
            ExpectedBar {
                expected_return: 0.0,
                expected_turnover: Decimal::ZERO,
                modelled_slippage_bps: 0.0,
                signal_matched: true,
            },
            ObservationSource::Observed,
        )
        .unwrap();

    // Put the book to work so the mark moves equity.
    recorder.on_submit(&intent(), px("100"));

    let (observation, verdict) = recorder
        .close_bar(
            at(2 * BAR),
            px("100"),
            ExpectedBar {
                // The backtest said +5% on this bar; the book returned ~0.
                expected_return: 0.05,
                expected_turnover: Decimal::ZERO,
                modelled_slippage_bps: 0.0,
                signal_matched: true,
            },
            ObservationSource::Observed,
        )
        .unwrap();

    assert_eq!(observation.source, ObservationSource::Observed);
    assert!(
        matches!(verdict, DivergenceVerdict::Halted(ref h) if h.reason.contains("divergence")),
        "a 5% gap against a 1% threshold must halt: {verdict:?}"
    );

    let err = try_submit(&rig).await.expect_err("the next order must be refused");
    assert!(matches!(err, RiskRejection::StrategyHalted { .. }));
    assert_eq!(rig.broker.submit_call_count(), 1);
}

#[tokio::test]
async fn a_halted_strategy_resumes_only_through_a_recorded_human_act() {
    let rig = rig();
    let recorder = ForwardRecorder::new(
        spec(),
        dec("100000"),
        Arc::clone(rig.gate.halt_registry()),
    )
    .unwrap();
    recorder.reconcile_position("AAPL", dec("2"), &rig.kill_switch);
    try_submit(&rig).await.expect_err("halted");

    let halts = rig.gate.halt_registry();

    // An empty note does not clear it.
    assert!(halts
        .clear(STRATEGY, HumanActor::new("mason").unwrap(), "")
        .is_err());
    try_submit(&rig).await.expect_err("still halted");

    // An anonymous actor cannot even be constructed.
    assert!(HumanActor::new("").is_err());

    // A named human with a stated reason clears it, and gets a record back to
    // persist. `HaltClearance` carries the original halt's id and reason, so the
    // record says what was overruled and not only that something was.
    let record = halts
        .clear(
            STRATEGY,
            HumanActor::new("mason").unwrap(),
            "broker restated an overnight sweep; positions agree",
        )
        .expect("a named human with a reason may clear it");
    assert_eq!(record.strategy_id, STRATEGY);
    assert!(record.halt_reason.contains("internal"));
    assert_eq!(record.cleared_by.as_str(), "mason");

    // The global kill switch tripped too, because a position disagreement is
    // about the account and not only this strategy. Both have to be lifted.
    rig.kill_switch.reset();
    try_submit(&rig).await.expect("resumed");
    assert_eq!(rig.broker.submit_call_count(), 1);
}

#[tokio::test]
async fn the_halt_is_scoped_to_the_diverging_strategy() {
    let rig = rig();
    let recorder = ForwardRecorder::new(
        spec(),
        dec("100000"),
        Arc::clone(rig.gate.halt_registry()),
    )
    .unwrap();

    // A return divergence halts only this strategy — no kill switch, because a
    // strategy whose realised returns stopped matching its backtest says
    // nothing about anyone else's book.
    recorder
        .close_bar(
            at(BAR),
            px("100"),
            ExpectedBar {
                expected_return: 0.0,
                expected_turnover: Decimal::ZERO,
                modelled_slippage_bps: 0.0,
                signal_matched: false,
            },
            ObservationSource::Observed,
        )
        .unwrap();

    assert!(
        rig.gate.halt_registry().is_halted(STRATEGY),
        "a 0% signal-match rate against a 99% floor must halt"
    );
    assert!(
        !rig.kill_switch.is_active(),
        "a return divergence must not stop the whole platform"
    );

    let other = OrderIntent::new(
        "MSFT",
        Side::Buy,
        OrderType::Market,
        Size::from_str("1").unwrap(),
        None,
        Some("some-other-strategy".to_owned()),
    );
    let approved = rig
        .gate
        .check(other, &ctx())
        .expect("another strategy is unaffected");
    rig.engine.submit(approved).await.unwrap();
    assert_eq!(rig.broker.submit_call_count(), 1);
}
