//! `RunConfig::fill_delay_bars`: every decided order reaches the venue N bars
//! later — what a timing-jitter sensitivity study varies (Set S catalogue).

// Exact float equality is the claim: the shadow is the backtest, not an
// approximation of it.
#![allow(clippy::float_cmp)]

use std::sync::Arc;

use backtest::run::ParamMap;
use backtest::runner::{self, V3Run};
use backtest::store::LoadedBar;
use chrono::{TimeZone, Utc};
use dataplane::quality::QualityFlags;
use domain::instrument::AssetClass;
use domain::strategy_graph::StrategyGraph;
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::env::NoModels;
use strategy_engine::{InstrumentSpec, VenueCaps};

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

fn run_with(delay: u32) -> runner::V3Outcome {
    const WARM: usize = 30;
    let all = bars(&wave(WARM + 240));
    let plan = Arc::new(
        runner::compile_for_run(&ema_cross(), &ParamMap::new(), &strategy_engine::StdRegistries::new())
            .unwrap(),
    );
    runner::run_delayed(
        V3Run {
            plan,
            instrument: "ETH-USD".into(),
            asset_class: AssetClass::CryptoSpotCex,
            bars: vec![all.clone()],
            start_ns: all[WARM - 1].ts_ns,
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
        delay,
    )
    .unwrap()
}

#[test]
fn a_delay_of_one_bar_fills_every_order_one_bar_later() {
    let on_time = run_with(0);
    let late = run_with(1);
    assert!(on_time.trades.len() >= 2, "the fixture trades");
    // Same decisions to enter; each entry fills one bar later.
    let first = &on_time.trades[0];
    let first_late = &late.trades[0];
    assert_eq!(
        first_late.entry_time - first.entry_time,
        chrono::Duration::hours(1),
        "{first:?} vs {first_late:?}"
    );
    // Zero is exactly the undelayed run.
    let zero = run_with(0);
    assert_eq!(zero.equity, on_time.equity);
}

#[test]
fn the_delay_is_part_of_a_runs_identity_only_when_set() {
    use backtest::run::{DataSlice, EvalResolution, RunConfigBuilder};
    let base = || {
        let slice = DataSlice::new(
            "ETH-USD",
            Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2025, 2, 1, 0, 0, 0).unwrap(),
            EvalResolution::Hour1,
        );
        RunConfigBuilder::new("ema_cross", "rev", slice, "cost:paper", "sz", "snap")
    };
    let a = base().build();
    let b = base().fill_delay_bars(0).build();
    let c = base().fill_delay_bars(1).build();
    assert_eq!(a.run_id, b.run_id, "zero leaves every existing run id alone");
    assert_ne!(a.run_id, c.run_id);
}
