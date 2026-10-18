//! The same `RunConfig` hash gives bit-identical trades, fills and equity (Set M M-2.8, BT-11;
//! FEAT-005 B5).
//!
//! The oracle test (`crates/backtest/tests/e2e.rs` and the gates that rely on it) holds the
//! engine to *correct*; this is the stricter claim that it is *reproducible*: two executions of
//! one config hash agree down to the bit of every float, not "within tolerance". A result whose
//! last ulp moves between runs is one whose hash cites a number nobody can re-derive.
//!
//! Run on the position-series executor, which the platform runs for Layer 1. The v3 runner has
//! `reproduce.rs` (a re-run under a reproduction ticket); this file is the byte-level
//! comparison for the executor written to this standard.

use std::sync::Arc;

use backtest::outputs_v2;
use backtest::position_run::{ArtifactBar, Layer1Artifact, PositionRunExecutor};
use backtest::position_series::{PositionPoint, PositionSeries, PositionUnits, SqrtImpact, TruncationVerdict};
use backtest::run::{Backtest, InMemoryRunStore, ParamMap, RunDispatch, RunResult};
use ledger::{DispatchContext, InMemoryLedger};

fn artifact(seed: u64) -> Layer1Artifact {
    let bars: Vec<ArtifactBar> = (0..400)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let p = 100.0 + (f64::from(i) * 0.21).sin() * 6.0 + (f64::from(i) * 0.013).cos() * 3.0;
            ArtifactBar { ts_ns: (i64::from(i) + 1) * 3_600_000_000_000, open: p, close: p * 1.0007 }
        })
        .collect();
    let points = bars
        .iter()
        .enumerate()
        .map(|(i, b)| {
            #[allow(clippy::cast_precision_loss)]
            let t = ((i as f64) * 0.11).sin();
            PositionPoint::new(b.ts_ns, if i % 7 == 0 { f64::NAN } else { t })
        })
        .collect();
    Layer1Artifact {
        code_sha256: "cd".repeat(32),
        instrument: "BTC-USD".into(),
        timeframe: "1h".into(),
        series: PositionSeries { points, position_units: PositionUnits::Fraction, allow_short: true },
        bars,
        initial_equity: 25_000.0,
        fee_bps: 10.0,
        half_spread_bps: 2.5,
        cost_model_ref: "cost:flat".into(),
        data_snapshot: "snap".into(),
        lot_size: None,
        impact: Some(SqrtImpact { y: 1.0, sigma: 0.012, volume_usd: 2e6 }),
        params: ParamMap::new(),
        seed,
    }
}

fn run(a: &Arc<Layer1Artifact>) -> (RunResult, backtest::run::RunConfig) {
    let cfg = a.run_config().unwrap();
    let bt = Backtest::new(
        InMemoryRunStore::new(),
        PositionRunExecutor::new(a.clone(), Some(TruncationVerdict::Pass { cuts_checked: 8 })),
        InMemoryLedger::new(),
    );
    let ctx = DispatchContext::human("tenant", "m-2.8", 0.1).with_experiment("exp");
    (bt.run_traced(&RunDispatch::new(&ctx, &cfg, 1.0)).unwrap().0, cfg)
}

/// Every float as its bits, so equality is bit equality and a NaN would show.
fn bits(r: &RunResult) -> (Vec<(i64, u64)>, Vec<(i64, u64)>, String, String) {
    (
        r.equity_curve.iter().map(|(t, e)| (t.timestamp_nanos_opt().unwrap(), e.to_bits())).collect(),
        r.net_exposure.iter().map(|(t, e)| (t.timestamp_nanos_opt().unwrap(), e.to_bits())).collect(),
        serde_json::to_string(&r.trades).unwrap(),
        // `produced_at` and `compute_cost` are wall-clock facts about *this execution*, not the result.
        serde_json::to_string(&(&r.metrics, &r.integrity_flags, &r.status, &r.run_id)).unwrap(),
    )
}

#[test]
fn two_runs_of_one_hash_agree_bit_for_bit_in_equity_exposure_trades_and_metrics() {
    let a = Arc::new(artifact(7));
    let (x, cx) = run(&a);
    let (y, cy) = run(&a);
    assert_eq!(cx.run_id, cy.run_id, "one config, one hash");
    assert_eq!(bits(&x), bits(&y));
    assert!(!x.trades.is_empty(), "the fixture trades, so equality is not vacuous");
}

#[test]
fn the_fills_and_the_rows_written_for_them_are_identical_across_runs() {
    let a = Arc::new(artifact(7));
    let one = PositionRunExecutor::new(a.clone(), None).evaluate().unwrap();
    let two = PositionRunExecutor::new(a.clone(), None).evaluate().unwrap();
    assert_eq!(one.fills.len(), two.fills.len());
    for (f, g) in one.fills.iter().zip(&two.fills) {
        assert_eq!((f.qty.to_bits(), f.price.to_bits(), f.fee.to_bits()), (g.qty.to_bits(), g.price.to_bits(), g.fee.to_bits()));
    }
    assert_eq!(outputs_v2::build("r", &a, &one), outputs_v2::build("r", &a, &two));
}

#[test]
fn mutating_the_seed_changes_the_hash_and_a_seeded_executor_refuses_a_mismatched_one() {
    let (_, base) = run(&Arc::new(artifact(7)));
    let (_, other) = run(&Arc::new(artifact(8)));
    assert_ne!(base.run_id, other.run_id, "the seed is part of the hash");
    // This executor has no probabilistic fills, so the seed cannot change the arithmetic; what it
    // may not do is accept a config whose seed disagrees with the series it was produced under.
    let a = Arc::new(artifact(7));
    let mut cfg = a.run_config().unwrap();
    cfg.seed = 8;
    let r = PositionRunExecutor::new(a, Some(TruncationVerdict::Pass { cuts_checked: 8 }));
    assert!(r.check_config(&cfg).is_err());
}

#[test]
fn a_change_to_the_series_changes_the_numbers_so_the_equality_above_is_not_a_constant() {
    let a = Arc::new(artifact(7));
    let mut b = artifact(7);
    b.series.points[50] = PositionPoint::new(b.series.points[50].ts_ns, 0.9);
    let (x, _) = run(&a);
    let (y, _) = run(&Arc::new(b));
    assert_ne!(bits(&x), bits(&y));
}
