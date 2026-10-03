//! Set U U-7.3: a v1 EMA cross, translated to v3, backtests on the one
//! strategy engine — entries from its signal, exits from its exit block.

use std::sync::Arc;

use backtest::runner::{self, V3Run};
use backtest::store::LoadedBar;
use dataplane::quality::QualityFlags;
use domain::instrument::AssetClass;
use domain::strategy_def::StrategyDefinition;
use execution::paper::SimulatorSet;
use execution::sim_venue::FillPolicy;
use risk::{GlobalRiskLimits, InstrumentLimits};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use strategy_engine::env::NoModels;
use strategy_engine::{InstrumentSpec, StdRegistries, VenueCaps};

const HOUR: i64 = 3_600_000_000_000;

#[allow(clippy::many_single_char_names)]
fn bars(n: usize) -> Vec<LoadedBar> {
    let t0 = 1_735_689_600_000_000_000_i64; // 2025-01-01T00:00Z
    let d = |x: f64| Decimal::from_f64_retain((x * 100.0).round() / 100.0).unwrap();
    let mut prev = 100.0;
    (0..n)
        .map(|i| {
            let x = i as f64;
            let c = 100.0 + 8.0 * (x / 12.0).sin() + 3.0 * (x / 5.0).cos();
            let open_ns = t0 + i as i64 * HOUR;
            let b = LoadedBar {
                ts_ns: open_ns + HOUR,
                open_ns,
                knowledge_ns: open_ns + HOUR,
                open: d(prev),
                high: d(prev.max(c) + 0.3),
                low: d(prev.min(c) - 0.3),
                close: d(c),
                volume: dec!(1000),
                trade_count: 10,
                quality_flags: QualityFlags::default(),
            };
            prev = c;
            b
        })
        .collect()
}

#[test]
fn a_translated_v1_ema_cross_backtests() {
    let v1: StrategyDefinition = serde_json::from_value(serde_json::json!({
      "strategy_id": "ema_cross_v1", "definition_version": "1.3", "asset_class": "crypto_spot_cex",
      "inputs": [],
      "nodes": [
        {"id": "up", "type": "condition", "expr": "feature('ema_5') > feature('ema_20')"},
        {"id": "dn", "type": "condition", "expr": "feature('ema_5') < feature('ema_20')"},
        {"id": "s1", "type": "signal", "when": "up", "emit": "long"},
        {"id": "s2", "type": "signal", "when": "dn", "emit": "flat"}
      ],
      "actions": [{"on_signal": "long", "type": "place_order",
                   "order": {"side": "buy", "size_mode": "fixed", "size": "10"}}],
      "exits": {"mode": "rules", "rules": [{"rule": "on_signal", "emit": "flat"}]}
    }))
    .unwrap();
    let t = strategy_lang::v3::translate_v1::translate(&v1, "1h").expect("translates");
    let plan = runner::compile_for_run(
        &t.graph,
        &backtest::run::ParamMap::new(),
        &StdRegistries::new(),
    )
    .unwrap_or_else(|e| panic!("{e}\n{}", t.text));
    let hourly = bars(400);
    let start = hourly[40].ts_ns;
    let out = runner::run(
        V3Run {
            plan: Arc::new(plan),
            instrument: "ETH-USD".into(),
            asset_class: AssetClass::CryptoSpotCex,
            bars: vec![hourly],
            start_ns: start,
            spec: InstrumentSpec {
                instrument: "ETH-USD".into(),
                lot_size: dec!(0.0001),
                tick_size: dec!(0.01),
                min_qty: dec!(0.0001),
                caps: VenueCaps::all(),
            },
            instrument_limits: InstrumentLimits {
                tick_size: dec!(0.01),
                lot_size: Decimal::ZERO,
            },
            limits: GlobalRiskLimits::default(),
            initial_balance: dec!(10_000),
            fill_policy: FillPolicy::NextBarOpen,
            strategy_id: "ema_cross_v1".into(),
            sims: SimulatorSet::realistic(),
        },
        &mut NoModels,
    )
    .expect("runs");
    assert!(
        out.trades.len() >= 3,
        "it entered and exited: {} round trips\n{}",
        out.trades.len(),
        t.text
    );
    for tr in &out.trades {
        assert!(tr.qty >= dec!(10), "the v1 size carried: {}", tr.qty);
    }
}
