//! [`RunResult`] — the full, immutable output of one Run (spec §1.1).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::config::RunConfig;
use super::id::RunId;
use super::metrics::MetricSet;

/// Terminal status of a Run. `RejectedIntegrity` is set by Gate 0 (Phase 4);
/// `Failed` is any execution error — both are stored and counted, never lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Ok,
    Failed,
    RejectedIntegrity,
}

/// Long or short.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Long,
    Short,
}

/// One round-trip trade. `costs_paid` and `pnl` are `Decimal` (money, ADR-0002);
/// `mae`/`mfe` are `f64` excursions in return units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub symbol: String,
    pub side: Side,
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub entry_price: Decimal,
    pub exit_price: Decimal,
    pub qty: Decimal,
    /// Maximum adverse excursion (fraction, ≤ 0).
    pub mae: f64,
    /// Maximum favorable excursion (fraction, ≥ 0).
    pub mfe: f64,
    /// Holding period in seconds.
    pub holding_period_secs: i64,
    /// Total costs paid on this trade (money).
    pub costs_paid: Decimal,
    /// Realized P&L net of costs (money).
    pub pnl: Decimal,
}

/// An integrity / sanity finding written by Gate 0 (Phase 4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Flag {
    /// Stable machine code, e.g. `"lookahead.higher_tf_open"`.
    pub code: String,
    /// Human-readable detail.
    pub detail: String,
}

/// Wall/CPU cost of producing a Run, for funnel budgeting (Phase 4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeCost {
    pub wall_ms: u64,
    pub cpu_ms: u64,
}

/// The full output of one Run (spec §1.1). Immutable once produced.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunResult {
    /// Matches the config that produced it.
    pub run_id: RunId,
    pub status: RunStatus,
    /// Equity curve: `(timestamp, equity_value)`.
    pub equity_curve: Vec<(DateTime<Utc>, f64)>,
    /// Per-period net exposure fraction (for exposure metrics / reconciliation).
    #[serde(default)]
    pub net_exposure: Vec<(DateTime<Utc>, f64)>,
    pub trades: Vec<Trade>,
    pub metrics: MetricSet,
    /// Leakage/lookahead/cost-sanity findings (Gate 0 populates).
    #[serde(default)]
    pub integrity_flags: Vec<Flag>,
    pub compute_cost: ComputeCost,
    pub produced_at: DateTime<Utc>,
    /// Engine version (see [`super::ENGINE_VERSION`]).
    pub produced_by: String,
    /// Carries the `unsafe` bit of the producing config (INV-1).
    #[serde(default)]
    pub unsafe_: bool,
}

impl RunResult {
    /// Build a `Failed` result for `cfg` with a reason flag — used so an
    /// erroring execution is still a recorded, counted Run (never dropped).
    #[must_use]
    pub fn failed(
        cfg: &RunConfig,
        reason: impl Into<String>,
        produced_by: impl Into<String>,
    ) -> Self {
        Self {
            run_id: cfg.run_id.clone(),
            status: RunStatus::Failed,
            equity_curve: Vec::new(),
            net_exposure: Vec::new(),
            trades: Vec::new(),
            metrics: MetricSet::empty(),
            integrity_flags: vec![Flag {
                code: "run.failed".into(),
                detail: reason.into(),
            }],
            compute_cost: ComputeCost::default(),
            produced_at: Utc::now(),
            produced_by: produced_by.into(),
            unsafe_: cfg.unsafe_,
        }
    }

    /// Build a `RejectedIntegrity` result for `cfg` with the failing flags.
    #[must_use]
    pub fn rejected_integrity(
        cfg: &RunConfig,
        flags: Vec<Flag>,
        produced_by: impl Into<String>,
    ) -> Self {
        Self {
            run_id: cfg.run_id.clone(),
            status: RunStatus::RejectedIntegrity,
            equity_curve: Vec::new(),
            net_exposure: Vec::new(),
            trades: Vec::new(),
            metrics: MetricSet::empty(),
            integrity_flags: flags,
            compute_cost: ComputeCost::default(),
            produced_at: Utc::now(),
            produced_by: produced_by.into(),
            unsafe_: cfg.unsafe_,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::config::{DataSlice, EvalResolution, RunConfigBuilder};
    use chrono::TimeZone;
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
    fn run_result_round_trips() {
        let c = cfg();
        let r = RunResult {
            run_id: c.run_id.clone(),
            status: RunStatus::Ok,
            equity_curve: vec![(Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(), 100.0)],
            net_exposure: vec![],
            trades: vec![Trade {
                symbol: "BTC".into(),
                side: Side::Long,
                entry_time: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                exit_time: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
                entry_price: dec!(100),
                exit_price: dec!(110),
                qty: dec!(1),
                mae: -0.02,
                mfe: 0.12,
                holding_period_secs: 86_400,
                costs_paid: dec!(0.5),
                pnl: dec!(9.5),
            }],
            metrics: MetricSet::empty(),
            integrity_flags: vec![],
            compute_cost: ComputeCost {
                wall_ms: 5,
                cpu_ms: 4,
            },
            produced_at: Utc::now(),
            produced_by: "engine@test".into(),
            unsafe_: false,
        };
        let back: RunResult = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn failed_carries_unsafe_and_a_reason() {
        let c = cfg();
        let r = RunResult::failed(&c, "boom", "engine@test");
        assert_eq!(r.status, RunStatus::Failed);
        assert_eq!(r.run_id, c.run_id);
        assert_eq!(r.integrity_flags.len(), 1);
    }
}
