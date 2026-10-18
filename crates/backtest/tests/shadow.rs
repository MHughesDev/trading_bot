//! Set T T-0.4: the shadow run beside a paper deployment is the backtest.
//!
//! The expected series forward evidence compares against has to be what the
//! backtest would have done over the live bars. So a shadow fed the bars one
//! at a time, as a paper runner hears them, must match a backtest run over the
//! same bars decision for decision and equity for equity — or the "expected"
//! side of every forward observation is a different model.

// Exact float equality is the claim: the shadow is the backtest, not an
// approximation of it.
#![allow(clippy::float_cmp)]

use std::sync::Arc;

use backtest::replay::Genesis;
use backtest::run::ParamMap;
use backtest::runner::{self, V3Run};
use backtest::shadow::{Shadow, ShadowSpec};
use backtest::store::LoadedBar;
use chrono::{DateTime, TimeZone, Utc};
use dataplane::quality::QualityFlags;
use domain::instrument::AssetClass;
use domain::strategy_graph::StrategyGraph;
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::env::NoModels;
use strategy_engine::{Bar, InstrumentSpec, VenueCaps};

const HOUR: i64 = 3_600_000_000_000;

fn t0_ns() -> i64 {
    Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0)
        .unwrap()
        .timestamp_nanos_opt()
        .unwrap()
}

fn d(x: f64) -> Decimal {
    Decimal::from_f64_retain((x * 100.0).round() / 100.0)
        .unwrap()
        .round_dp(2)
}

fn bars(closes: &[f64]) -> Vec<LoadedBar> {
    let mut prev = closes[0];
    closes
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let open_ns = t0_ns() + i as i64 * HOUR;
            let (o, h, l) = (prev, prev.max(c) + 0.3, prev.min(c) - 0.3);
            prev = c;
            LoadedBar {
                ts_ns: open_ns + HOUR,
                open_ns,
                knowledge_ns: open_ns + HOUR,
                open: d(o),
                high: d(h),
                low: d(l),
                close: d(c),
                volume: dec!(1000),
                trade_count: 10,
                quality_flags: QualityFlags::default(),
            }
        })
        .collect()
}

fn engine_bar(b: &LoadedBar) -> Bar {
    let f = |x: Decimal| x.to_f64().unwrap();
    Bar {
        close_ns: b.ts_ns,
        open: f(b.open),
        high: f(b.high),
        low: f(b.low),
        close: f(b.close),
        volume: f(b.volume),
    }
}

fn wave(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| {
            let x = i as f64;
            100.0 + 8.0 * (x / 12.0).sin() + 3.0 * (x / 5.0).cos()
        })
        .collect()
}

fn ema_cross() -> StrategyGraph {
    serde_json::from_value(serde_json::json!({
      "format": "tbot.strategy/3", "kind": "trading", "name": "ema_cross",
      "asset_class": "crypto_spot_cex", "timeframe": "1h",
      "nodes": [
        {"id":"f",    "op":"ema",           "args":{"x":"close", "n":5}},
        {"id":"s",    "op":"ema",           "args":{"x":"close", "n":20}},
        {"id":"up",   "op":"crossed_above", "args":{"a":"f", "b":"s"}},
        {"id":"entry","op":"and",           "args":{"a":"up", "b":"state.flat"}},
        {"id":"down", "op":"crossed_below", "args":{"a":"f", "b":"s"}},
        {"id":"leave","op":"and",           "args":{"a":"down", "b":"state.long"}}
      ],
      "decide": {"rules": [
        {"when":"entry","then":{"kind":"target","size":{"mode":"units","x":10}}},
        {"when":"leave","then":{"kind":"close"}}
      ]}
    }))
    .unwrap()
}

fn spec() -> InstrumentSpec {
    InstrumentSpec {
        instrument: "ETH-USD".into(),
        lot_size: dec!(0.0001),
        tick_size: dec!(0.01),
        min_qty: dec!(0.0001),
        caps: VenueCaps::all(),
    }
}

fn limits() -> InstrumentLimits {
    InstrumentLimits {
        tick_size: dec!(0.01),
        lot_size: Decimal::ZERO,
    }
}

#[test]
fn a_shadow_fed_bar_by_bar_is_the_backtest_over_the_same_bars() {
    const WARM: usize = 30;
    let all = bars(&wave(WARM + 240));
    let g = ema_cross();
    let plan = Arc::new(
        runner::compile_for_run(&g, &ParamMap::new(), &strategy_engine::StdRegistries::new())
            .unwrap(),
    );
    let start_ns = all[WARM - 1].ts_ns;
    let outcome = runner::run(
        V3Run {
            plan: Arc::clone(&plan),
            instrument: "ETH-USD".into(),
            asset_class: AssetClass::CryptoSpotCex,
            bars: vec![all.clone()],
            start_ns,
            spec: spec(),
            instrument_limits: limits(),
            limits: GlobalRiskLimits::default(),
            initial_balance: dec!(10_000),
            fill_policy: FillPolicy::NextBarOpen,
            strategy_id: "ema_cross".into(),
            sims: SimulatorSet::realistic(),
            external: strategy_engine::ExternalData::default(),
        },
        &mut NoModels,
    )
    .unwrap();
    assert!(
        outcome.trades.len() >= 2,
        "the fixture trades: {} round trips",
        outcome.trades.len()
    );

    let genesis = Genesis {
        checkpoint: None,
        history: all[..WARM].iter().map(|b| (0, engine_bar(b))).collect(),
        external: None,
    };
    let mut shadow = Shadow::new(
        ShadowSpec {
            plan,
            spec: spec(),
            asset_class: AssetClass::CryptoSpotCex,
            instrument_limits: limits(),
            limits: GlobalRiskLimits::default(),
            cash: dec!(10_000),
            start: DateTime::from_timestamp_nanos(all[WARM].open_ns),
            strategy_id: "ema_cross".into(),
        },
        &genesis,
    )
    .unwrap();
    let mut decided = Vec::new();
    let mut previous = 10_000.0_f64;
    for (i, b) in all[WARM..].iter().enumerate() {
        let step = shadow
            .on_bar(0, engine_bar(b), &[])
            .unwrap()
            .expect("a primary bar is a step");
        let (at, equity) = outcome.equity[i];
        assert_eq!(step.bar, at, "bar {i}");
        assert_eq!(
            shadow.equity().to_f64().unwrap(),
            equity,
            "equity at bar {i}"
        );
        // The expected return is the backtest's own equity chain.
        assert!(
            (step.expected_return - (equity / previous - 1.0)).abs() < 1e-15,
            "return at bar {i}"
        );
        previous = equity;
        decided.extend(step.decided);
    }
    let digests = |t: &[strategy_engine::DecisionTrace]| -> Vec<String> {
        t.iter()
            .map(strategy_engine::DecisionTrace::digest)
            .collect()
    };
    assert_eq!(digests(&decided), digests(&outcome.traces));
}

#[test]
fn a_shadow_fill_reports_its_turnover_and_slippage_against_the_decision_price() {
    let all = bars(&wave(200));
    let g = ema_cross();
    let plan = Arc::new(
        runner::compile_for_run(&g, &ParamMap::new(), &strategy_engine::StdRegistries::new())
            .unwrap(),
    );
    let mut shadow = Shadow::new(
        ShadowSpec {
            plan,
            spec: spec(),
            asset_class: AssetClass::CryptoSpotCex,
            instrument_limits: limits(),
            limits: GlobalRiskLimits::default(),
            cash: dec!(10_000),
            start: DateTime::from_timestamp_nanos(all[0].open_ns),
            strategy_id: "ema_cross".into(),
        },
        &Genesis {
            checkpoint: None,
            history: Vec::new(),
            external: None,
        },
    )
    .unwrap();
    let mut traded = false;
    for b in &all {
        let step = shadow.on_bar(0, engine_bar(b), &[]).unwrap().unwrap();
        if step.turnover.is_zero() {
            assert_eq!(step.slippage_bps, 0.0);
        } else {
            traded = true;
            assert!(step.turnover > Decimal::ZERO);
            assert!(step.slippage_bps.is_finite() && step.slippage_bps >= 0.0);
        }
        assert!(step.expected_return.is_finite());
    }
    assert!(traded, "the fixture fills at least one order");
}
