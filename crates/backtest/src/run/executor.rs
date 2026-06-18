//! The Run executor (spec §1.1, ADR-001): a pure mapping `RunConfig → RunResult`.
//!
//! The executor is intentionally the *only* place that knows how to turn a
//! config into a simulated outcome; it knows nothing about Studies, nulls, or
//! trial counting (that lives one level up). The heavy `market_simulator`
//! integration is assembled from a [`RunExecutor`] implementation; the pure
//! result-mapping core ([`map_sim_result`]) is unit-testable without the engine
//! or live data.

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;

use super::config::RunConfig;
use super::metrics::{MetricInputs, MetricSet};
use super::result::{ComputeCost, RunResult, RunStatus};

/// Translates one `RunConfig` into a `RunResult`. An erroring execution must
/// return a `Failed` `RunResult` (never an `Err` that drops the run).
pub trait RunExecutor: Send + Sync {
    fn execute(&self, cfg: &RunConfig) -> RunResult;
}

/// A `RunExecutor` backed by an arbitrary closure — used for composition and in
/// tests as a stand-in for the simulator-backed executor.
pub struct ClosureExecutor<F>(pub F)
where
    F: Fn(&RunConfig) -> RunResult + Send + Sync;

impl<F> RunExecutor for ClosureExecutor<F>
where
    F: Fn(&RunConfig) -> RunResult + Send + Sync,
{
    fn execute(&self, cfg: &RunConfig) -> RunResult {
        (self.0)(cfg)
    }
}

/// The annualization factor implied by a config's eval resolution.
#[must_use]
pub fn periods_per_year(cfg: &RunConfig) -> f64 {
    use super::config::EvalResolution as E;
    match cfg.data_slice.eval_resolution {
        E::Min1 => 525_600.0,
        E::Min5 => 105_120.0,
        E::Min10 => 52_560.0,
        E::Min15 => 35_040.0,
        E::Min30 => 17_520.0,
        E::Hour1 => 8_760.0,
        E::Day1 => 252.0,
    }
}

/// Map a simulator result document into a [`RunResult`].
///
/// `equity` is the `(timestamp, equity_value)` curve the engine produced;
/// `net_exposure` is the per-bar net exposure fraction; `trades` are the
/// round-trips. Metrics are derived here so every Run — regardless of engine —
/// produces the standardized [`MetricSet`] shape.
#[must_use]
pub fn map_sim_result(
    cfg: &RunConfig,
    equity: Vec<(DateTime<Utc>, f64)>,
    net_exposure: Vec<(DateTime<Utc>, f64)>,
    trades: Vec<super::result::Trade>,
    compute_cost: ComputeCost,
    produced_by: impl Into<String>,
) -> RunResult {
    let returns = returns_from_equity(&equity);
    let exposure: Vec<f64> = net_exposure.iter().map(|(_, v)| *v).collect();
    let metrics = MetricSet::compute(&MetricInputs {
        equity_returns: &returns,
        trades: &trades,
        benchmark_returns: None,
        net_exposure: if exposure.is_empty() {
            None
        } else {
            Some(&exposure)
        },
        periods_per_year: periods_per_year(cfg),
    });
    RunResult {
        run_id: cfg.run_id.clone(),
        status: RunStatus::Ok,
        equity_curve: equity,
        net_exposure,
        trades,
        metrics,
        integrity_flags: Vec::new(),
        compute_cost,
        produced_at: Utc::now(),
        produced_by: produced_by.into(),
        unsafe_: cfg.unsafe_,
    }
}

/// Map a [`crate::sim::DetailedOutcome`] (real-engine trades + reconstructed
/// equity curve) into a [`RunResult`]. This is the suite executor's bridge from
/// the `market_simulator` to the standardized run shape — net exposure is absent
/// (the engine reports realized positions, not per-bar exposure), so exposure
/// metrics are simply unavailable, never wrong.
#[must_use]
pub fn map_detailed_result(
    cfg: &RunConfig,
    outcome: crate::sim::DetailedOutcome,
    compute_cost: ComputeCost,
    produced_by: impl Into<String>,
) -> RunResult {
    map_sim_result(
        cfg,
        outcome.equity,
        Vec::new(),
        outcome.trades,
        compute_cost,
        produced_by,
    )
}

/// Per-period simple returns from an equity curve.
#[must_use]
pub fn returns_from_equity(equity: &[(DateTime<Utc>, f64)]) -> Vec<f64> {
    equity
        .windows(2)
        .map(|w| {
            if w[0].1 == 0.0 {
                0.0
            } else {
                w[1].1 / w[0].1 - 1.0
            }
        })
        .collect()
}

/// Convenience for synthetic equity curves in tests: stamp values at a fixed
/// daily cadence from `2024-01-01`.
#[must_use]
pub fn daily_curve(values: &[f64]) -> Vec<(DateTime<Utc>, f64)> {
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let ts = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
                + chrono::Duration::days(i as i64);
            (ts, *v)
        })
        .collect()
}

/// A trivially-constructed trade (test helper).
#[must_use]
pub fn sample_trade(pnl: Decimal) -> super::result::Trade {
    super::result::Trade {
        symbol: "SYM".into(),
        side: super::result::Side::Long,
        entry_time: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        exit_time: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        entry_price: Decimal::ONE,
        exit_price: Decimal::ONE,
        qty: Decimal::ONE,
        mae: 0.0,
        mfe: 0.0,
        holding_period_secs: 86_400,
        costs_paid: Decimal::ZERO,
        pnl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::config::{DataSlice, EvalResolution, RunConfigBuilder};
    use rust_decimal_macros::dec;

    fn cfg() -> RunConfig {
        let s = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        RunConfigBuilder::new("s", "v", s, "c", "z", "snap").build()
    }

    #[test]
    fn maps_a_rising_curve_to_ok_with_positive_return() {
        let c = cfg();
        let equity = daily_curve(&[100.0, 101.0, 102.0, 103.0]);
        let r = map_sim_result(
            &c,
            equity,
            vec![],
            vec![sample_trade(dec!(3))],
            ComputeCost {
                wall_ms: 1,
                cpu_ms: 1,
            },
            "engine@test",
        );
        assert_eq!(r.status, RunStatus::Ok);
        assert_eq!(r.run_id, c.run_id);
        assert!(r.metrics.total_return > 0.0);
        assert_eq!(r.metrics.n_trades, 1);
        assert!((r.metrics.hit_rate - 1.0).abs() < 1e-12);
    }

    #[test]
    fn maps_detailed_outcome_to_metrics() {
        let c = cfg();
        let outcome = crate::sim::DetailedOutcome {
            cancelled: false,
            equity: daily_curve(&[100.0, 101.0, 102.0]),
            trades: vec![sample_trade(dec!(2))],
            stats: serde_json::Value::Null,
        };
        let r = map_detailed_result(
            &c,
            outcome,
            ComputeCost {
                wall_ms: 1,
                cpu_ms: 1,
            },
            "engine@test",
        );
        assert_eq!(r.status, RunStatus::Ok);
        assert_eq!(r.metrics.n_trades, 1);
        assert!(r.metrics.total_return > 0.0);
    }

    #[test]
    fn closure_executor_failing_config_yields_failed() {
        let exec = ClosureExecutor(|cfg: &RunConfig| RunResult::failed(cfg, "boom", "engine@test"));
        let r = exec.execute(&cfg());
        assert_eq!(r.status, RunStatus::Failed);
    }

    #[test]
    fn returns_from_equity_is_correct() {
        let curve = daily_curve(&[100.0, 110.0, 99.0]);
        let r = returns_from_equity(&curve);
        assert!((r[0] - 0.10).abs() < 1e-12);
        assert!((r[1] - (-0.10)).abs() < 1e-12);
    }
}
