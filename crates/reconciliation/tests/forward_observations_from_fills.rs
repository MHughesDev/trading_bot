//! T-0.1: a paper fill produces a forward observation within one bar, and the
//! realised series is element-wise comparable with the ledger's stored backtest
//! series **without any per-call adjustment**.
//!
//! The fill stream here is injected through the real `ExecutionEngine` and its
//! real `FillProcessor` — the only stand-in is the venue adapter. That is
//! deliberate: the path from an applied fill to a `forward_observation` row is
//! exercised end to end, so the moment a real paper venue is wired in (track 16)
//! this pipeline works with no further change.

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use domain::forward::{Gate15Evidence, ObservationSource};
use domain::money::{Price, Size};
use domain::numeraire::Numeraire;
use domain::order::{OrderIntent, OrderType, Side};
use execution::broker::{Broker, BrokerOrderState};
use execution::mock_broker::MockBroker;
use execution::ExecutionEngine;
use ledger::neff::ReturnSeries;
use reconciliation::forward::{
    DeploymentSpec, DivergenceThresholds, ExpectedBar, PaperVenue, RealisedSeries,
};
use reconciliation::{ForwardRecorder, ForwardRecorders};
use risk::{GateContext, GlobalRiskLimits, KillSwitch, RiskGate, StrategyHaltRegistry};
use rust_decimal::Decimal;
use uuid::Uuid;

/// Install an order journal once per test binary.
///
/// `ExecutionEngine::submit` refuses without one — a trade the platform cannot
/// reconcile is worse than a trade it did not make — and these tests have no
/// database. `journal::install` is a `OnceLock`, so first call wins and the
/// rest are no-ops; calling it from every test is therefore safe and means no
/// test depends on another having run first.
fn ensure_journal() {
    execution::journal::install(std::sync::Arc::new(
        execution::journal::memory::MemoryOrderJournal::new(),
    ));
}


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

fn spec(numeraire: Numeraire) -> DeploymentSpec {
    DeploymentSpec {
        deployment_id: Uuid::new_v4(),
        strategy_id: STRATEGY.to_owned(),
        trial_id: Uuid::new_v4(),
        venue: PaperVenue::AlpacaPaper,
        numeraire,
        bar_seconds: BAR,
        thresholds: DivergenceThresholds {
            // Wide enough that this test measures the pipeline, not the halt —
            // the halt has its own drill.
            max_abs_return_divergence: 1.0,
            max_cum_return_divergence: 1.0,
            max_position_divergence: dec("0.000001"),
            min_signal_match: 0.5,
        },
    }
}

fn buy_intent() -> OrderIntent {
    OrderIntent::new(
        "AAPL",
        Side::Buy,
        OrderType::Market,
        Size::from_str("10").unwrap(),
        None,
        Some(STRATEGY.to_owned()),
    )
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

/// A bar the backtest expected nothing to happen on. The unit is the
/// deployment's, not the bar's — an `ExpectedBar` is dimensionless by design.
fn flat() -> ExpectedBar {
    ExpectedBar {
        expected_return: 0.0,
        expected_turnover: Decimal::ZERO,
        modelled_slippage_bps: 0.0,
        signal_matched: true,
    }
}

#[tokio::test]
async fn a_fill_reaches_the_forward_record_within_one_bar() {
    ensure_journal();
    let halts = Arc::new(StrategyHaltRegistry::new());
    // Through the registry, because that is the only way a recorder is ever
    // attached to the fill stream — including here. A test that wired itself
    // up by hand would be testing a path production does not have.
    let recorder = ForwardRecorders::install()
        .deploy(spec(Numeraire::Usd), dec("10000"), Arc::clone(&halts))
        .unwrap();

    let mut mock = MockBroker::new();
    mock.filled_qty = dec("10");
    mock.fill_price = Some(px("101"));
    mock.query_state = BrokerOrderState::Filled;
    let broker = Arc::new(mock);

    // One seam, not two: the engine publishes to the global observer registry
    // when a fill is both new to the processor AND newly inserted into `fills`.
    // A per-engine `with_fill_observer` would have been a second, weaker path —
    // it fires on `FillResult::Applied` alone, which a redelivery satisfies.
    // `ForwardRecorders::install` (above, inside `deploy`) is that one
    // subscription and it is idempotent, so the second test in this binary
    // joins it rather than doubling it.
    let engine = ExecutionEngine::new(Arc::clone(&broker) as Arc<dyn Broker>);

    let kill_switch = Arc::new(KillSwitch::new(false));
    let gate = RiskGate::new(GlobalRiskLimits::default(), kill_switch);

    // The opening mark establishes the equity base. It is deliberately not an
    // observation: a bar the strategy did not trade must not enter Gate 15's
    // count.
    recorder.open(at(BAR), px("100")).unwrap();

    // The strategy decides at 100 and submits.
    let intent = buy_intent();
    recorder.on_submit(&intent, px("100"));
    assert_eq!(
        recorder.position(),
        Decimal::ZERO,
        "an intent is not a fill: deciding to trade must not move the position"
    );

    let approved = gate.check(intent.clone(), &ctx()).expect("approved");
    let submitted = engine.submit(approved).await.expect("submitted");

    // `submit` queries the order back and records whatever the venue already
    // filled, so a market order this mock fills immediately reaches the
    // recorder here rather than on a later sync. That is the honest ordering —
    // the fill happened — and it is why the assertion above is placed before
    // the submit rather than after it.
    assert!(submitted.fill.is_some(), "the mock fills this market order at once");
    assert_eq!(
        recorder.position(),
        dec("10"),
        "the applied fill must reach the forward record"
    );

    // Re-syncing the same broker state is the redelivery case. The journal
    // recognises the fill, so nothing is applied twice — a position that moved
    // here would be a forward observation that never happened.
    engine
        .sync_order(
            intent.idempotency_key,
            &submitted.broker_order_id,
            Decimal::ZERO,
            Side::Buy,
        )
        .await
        .expect("sync");

    assert_eq!(
        recorder.position(),
        dec("10"),
        "a redelivered fill must not be counted twice"
    );

    // Bar 2 closes: the observation exists, within one bar of the fill.
    let (observation, verdict) = recorder
        .close_bar(
            at(2 * BAR),
            px("101"),
            flat(),
            ObservationSource::Observed,
        )
        .unwrap();

    assert!(!verdict.is_halted(), "{verdict:?}");
    assert_eq!(observation.strategy_id, STRATEGY);
    assert_eq!(observation.bar, at(2 * BAR));
    assert_eq!(observation.source, ObservationSource::Observed);
    assert_eq!(observation.numeraire, Numeraire::Usd);
    assert!(
        (observation.realised_slippage_bps - 100.0).abs() < 1e-6,
        "bought at 101 having decided at 100 — 100 bps of cost, got {}",
        observation.realised_slippage_bps
    );
    assert!(
        observation.realised_turnover > 0.0,
        "the bar turned over notional"
    );
}

/// A redelivered fill must not be counted twice. It would inflate exactly the
/// evidence that is hardest to acquire.
#[tokio::test]
async fn a_redelivered_fill_does_not_double_count() {
    ensure_journal();
    let halts = Arc::new(StrategyHaltRegistry::new());
    // Through the registry, because that is the only way a recorder is ever
    // attached to the fill stream — including here. A test that wired itself
    // up by hand would be testing a path production does not have.
    let recorder = ForwardRecorders::install()
        .deploy(spec(Numeraire::Usd), dec("10000"), Arc::clone(&halts))
        .unwrap();

    let mut mock = MockBroker::new();
    mock.filled_qty = dec("10");
    mock.fill_price = Some(px("101"));
    mock.query_state = BrokerOrderState::Filled;
    let broker = Arc::new(mock);

    // One seam, not two: the engine publishes to the global observer registry
    // when a fill is both new to the processor AND newly inserted into `fills`.
    // A per-engine `with_fill_observer` would have been a second, weaker path —
    // it fires on `FillResult::Applied` alone, which a redelivery satisfies.
    // `ForwardRecorders::install` (above, inside `deploy`) is that one
    // subscription and it is idempotent, so the second test in this binary
    // joins it rather than doubling it.
    let engine = ExecutionEngine::new(Arc::clone(&broker) as Arc<dyn Broker>);

    let intent = buy_intent();
    recorder.on_submit(&intent, px("100"));

    for _ in 0..3 {
        engine
            .sync_order(intent.idempotency_key, "mock-1", Decimal::ZERO, Side::Buy)
            .await
            .expect("sync");
    }

    assert_eq!(
        recorder.position(),
        dec("10"),
        "three syncs of one fill must move the position once"
    );
}

/// The point of T-0.1: the realised series drops straight into a comparison
/// with the ledger's stored backtest series. No resampling, no unit conversion,
/// no "close enough" join.
#[tokio::test]
async fn the_realised_series_compares_element_wise_with_a_stored_backtest_series() {
    ensure_journal();
    let halts = Arc::new(StrategyHaltRegistry::new());
    let recorder =
        ForwardRecorder::new(spec(Numeraire::Usd), dec("10000"), Arc::clone(&halts)).unwrap();

    recorder.open(at(BAR), px("100")).unwrap();
    for i in 2..=5 {
        recorder
            .close_bar(
                at(i * BAR),
                px("100"),
                flat(),
                ObservationSource::Observed,
            )
            .unwrap();
    }
    let realised = recorder.realised_series();

    // What the ledger stores for a backtest: `ReturnSeries::from_equity` over
    // the same bar grid. Built here exactly as `persist_returns` receives it.
    let curve: Vec<(DateTime<Utc>, f64)> =
        (1..=5).map(|i| (at(i * BAR), 10_000.0_f64)).collect();
    let backtest = RealisedSeries {
        numeraire: Numeraire::Usd,
        bar_seconds: BAR,
        series: ReturnSeries::from_equity(&curve),
    };

    let bars = realised
        .align(&backtest)
        .expect("the two series line up with no adjustment");
    assert_eq!(bars.len(), 4);
    for b in &bars {
        assert!(b.divergence().abs() < 1e-12, "{b:?}");
    }

    // And the realised series passes the ledger's own validation, because it is
    // the ledger's own type.
    realised.series.validate().expect("a valid return series");
}

/// A USDT-denominated deployment is not comparable with a USD backtest, and the
/// comparison refuses rather than summing them.
#[tokio::test]
async fn a_usdt_deployment_is_not_summed_as_usd() {
    ensure_journal();
    let halts = Arc::new(StrategyHaltRegistry::new());
    let recorder =
        ForwardRecorder::new(spec(Numeraire::of("USDT")), dec("10000"), Arc::clone(&halts)).unwrap();
    recorder.open(at(BAR), px("100")).unwrap();
    for i in 2..=3 {
        recorder
            .close_bar(
                at(i * BAR),
                px("100"),
                flat(),
                ObservationSource::Observed,
            )
            .unwrap();
    }

    let curve: Vec<(DateTime<Utc>, f64)> =
        (1..=3).map(|i| (at(i * BAR), 10_000.0_f64)).collect();
    let usd_backtest = RealisedSeries {
        numeraire: Numeraire::Usd,
        bar_seconds: BAR,
        series: ReturnSeries::from_equity(&curve),
    };

    recorder
        .realised_series()
        .align(&usd_backtest)
        .expect_err("USDT must not align against USD");
}

/// Observations accumulated from the fill stream become Gate 15 evidence, and a
/// backfilled row planted in the same window does not enter the count.
#[tokio::test]
async fn observations_become_gate_15_evidence_and_backfill_does_not() {
    ensure_journal();
    let halts = Arc::new(StrategyHaltRegistry::new());
    let recorder =
        ForwardRecorder::new(spec(Numeraire::Usd), dec("10000"), Arc::clone(&halts)).unwrap();

    let mut rows = Vec::new();
    recorder.open(at(0), px("100")).unwrap();
    for i in 1..=6 {
        let (o, _) = recorder
            .close_bar(
                at(i * 86_400),
                px("100"),
                flat(),
                ObservationSource::Observed,
            )
            .unwrap();
        rows.push(o);
    }

    // Planted inside the window, disagreeing with everything around it.
    let mut planted = rows[2].clone();
    planted.source = ObservationSource::Backfill;
    planted.signal_matched = false;
    planted.model_attributable_rejects = 99;
    rows.push(planted);

    let evidence = Gate15Evidence::from_observations(&rows).unwrap();
    assert_eq!(evidence.observations, 6, "the backfilled row is not evidence");
    assert_eq!(evidence.observed_days, 6);
    assert_eq!(evidence.model_attributable_rejects, 0);
    assert!((evidence.signal_match - 1.0).abs() < 1e-12);
}
