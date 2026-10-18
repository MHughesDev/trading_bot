//! Set P P-3.6 in a backtest: a strategy gated on a macro series trades only
//! once the vintage it needs was knowable, the same rows built into a paper
//! runner's genesis rebuild the same engine, and a run handed no rows reads the
//! series as missing — it does not trade on a zero.

use std::sync::Arc;

use backtest::replay::Genesis;
use backtest::runner::{self, V3Run};
use backtest::store::LoadedBar;
use chrono::{NaiveDate, TimeZone, Utc};
use dataplane::quality::QualityFlags;
use dataplane::vintage::MacroObservation;
use domain::instrument::AssetClass;
use domain::strategy_graph::StrategyGraph;
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::env::NoModels;
use strategy_engine::{
    Engine, Event, ExternalData, InstrumentSpec, PortfolioView, Step, VenueCaps,
};

const HOUR: i64 = 3_600_000_000_000;

fn t0_ns() -> i64 {
    Utc.with_ymd_and_hms(2025, 6, 1, 0, 0, 0)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap()
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2025, 6, d).unwrap()
}

/// DGS10 for 9 June: 4.0, published on the 10th.
fn dgs10() -> ExternalData {
    ExternalData {
        macro_rows: vec![MacroObservation::derived(
            "DGS10",
            day(9),
            day(10),
            Some(dec!(4.0)),
        )],
        facts: vec![],
    }
}

fn hourly(n: usize) -> Vec<LoadedBar> {
    (0..n)
        .map(|i| {
            let open_ns = t0_ns() + i as i64 * HOUR;
            LoadedBar {
                ts_ns: open_ns + HOUR,
                open_ns,
                knowledge_ns: open_ns + HOUR,
                open: dec!(100),
                high: dec!(100.5),
                low: dec!(99.5),
                close: dec!(100),
                volume: dec!(1000),
                trade_count: 10,
                quality_flags: QualityFlags::default(),
            }
        })
        .collect()
}

/// Buy one unit while the 10-year yield is above 3.5%.
fn graph() -> StrategyGraph {
    serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "rates_gate",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"rate","op":"feature","args":{"name":{"str":"macro_dgs10"}}},
        {"id":"high","op":"gt","args":{"a":"rate","b":3.5}},
        {"id":"go","op":"and","args":{"a":"high","b":"state.flat"}}
      ],
      "decide": {"rules": [{"when":"go","then":{"kind":"target","size":{"mode":"units","x":1}}}]}
    }))
    .unwrap()
}

fn spec() -> InstrumentSpec {
    InstrumentSpec {
        instrument: "BTC-USD".into(),
        lot_size: dec!(0.0001),
        tick_size: dec!(0.01),
        min_qty: dec!(0.0001),
        caps: VenueCaps::all(),
    }
}

fn run(external: ExternalData) -> runner::V3Outcome {
    let plan = runner::compile_for_run(
        &graph(),
        &backtest::run::ParamMap::new(),
        &strategy_engine::StdRegistries::new(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    runner::run(
        V3Run {
            plan: Arc::new(plan),
            instrument: "BTC-USD".into(),
            asset_class: AssetClass::CryptoSpotCex,
            bars: vec![hourly(24 * 20)],
            start_ns: t0_ns() + 48 * HOUR,
            spec: spec(),
            instrument_limits: InstrumentLimits {
                tick_size: dec!(0.01),
                lot_size: Decimal::ZERO,
            },
            limits: GlobalRiskLimits::default(),
            initial_balance: dec!(10_000),
            fill_policy: FillPolicy::NextBarOpen,
            strategy_id: "rates_gate".into(),
            sims: SimulatorSet::realistic(),
            external,
        },
        &mut NoModels,
    )
    .unwrap()
}

#[test]
fn a_backtest_trades_on_a_macro_series_only_once_its_vintage_was_knowable() {
    let out = run(dgs10());
    let known = dgs10().macro_rows[0]
        .knowledge
        .time()
        .timestamp_nanos_opt()
        .unwrap();
    let entries: Vec<i64> = out
        .traces
        .iter()
        .filter(|t| !t.actions.is_empty())
        .map(|t| t.at_ns)
        .collect();
    assert!(
        !entries.is_empty(),
        "the gate opened once 4.0 was published"
    );
    assert!(
        entries.iter().all(|at| *at >= known),
        "no decision before the vintage was knowable acted: {entries:?} vs {known}"
    );
    let first_decision = out.traces.first().unwrap().at_ns;
    assert!(
        first_decision < known,
        "the window starts before it, so the check bites"
    );

    // No rows: the series is missing, and the strategy holds.
    let none = run(ExternalData::default());
    assert!(none.traces.iter().all(|t| t.actions.is_empty()));
}

#[test]
fn a_genesis_with_external_rows_rebuilds_the_engine_a_runner_had() {
    let plan = Arc::new(
        runner::compile_for_run(
            &graph(),
            &backtest::run::ParamMap::new(),
            &strategy_engine::StdRegistries::new(),
        )
        .unwrap(),
    );
    let bars: Vec<strategy_engine::Bar> = hourly(24 * 20)
        .iter()
        .map(|b| strategy_engine::Bar {
            close_ns: b.ts_ns,
            open: 100.0,
            high: 100.5,
            low: 99.5,
            close: 100.0,
            volume: 1000.0,
        })
        .collect();
    let warm = 24 * 9;
    let genesis = Genesis {
        checkpoint: None,
        history: bars[..warm].iter().map(|b| (0, *b)).collect(),
        external: Some(dgs10()),
    };
    // Through JSON, as the journal keeps it.
    let genesis: Genesis = serde_json::from_value(serde_json::to_value(&genesis).unwrap()).unwrap();
    let mut rebuilt = genesis.build(Arc::clone(&plan), &spec()).unwrap();

    let mut direct = Engine::new(Arc::clone(&plan)).unwrap();
    let view = PortfolioView::default();
    direct
        .on_event(Event::External(dgs10()), &view, &spec())
        .unwrap();
    for b in &bars[..warm] {
        direct
            .on_event(
                Event::Bar {
                    input: 0,
                    bar: *b,
                    warmup: true,
                },
                &view,
                &spec(),
            )
            .unwrap();
    }
    let decide = |e: &mut Engine, b: strategy_engine::Bar| match e
        .on_event(
            Event::Bar {
                input: 0,
                bar: b,
                warmup: false,
            },
            &view,
            &spec(),
        )
        .unwrap()
    {
        Step::Decided(t) => serde_json::to_string(&*t).unwrap(),
        other => panic!("{other:?}"),
    };
    for b in &bars[warm..] {
        assert_eq!(decide(&mut rebuilt, *b), decide(&mut direct, *b));
    }
}
