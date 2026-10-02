//! [`RunResult`] — the full, immutable output of one Run (spec §1.1).

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use ledger::TerminalReason;

use super::config::RunConfig;
use super::id::RunId;
use super::metrics::MetricSet;

/// Terminal status of a Run. `RejectedIntegrity` is set by Gate 0 (Phase 4);
/// `Failed` is any execution error — both are stored and counted, never lost.
///
/// `Failed` carries the [`TerminalReason`] the ledger will record, because a
/// failure whose reason has to be reconstructed later is a failure whose reason
/// is guessed: the settlement path used to substring-match the message and call
/// everything it did not recognise `dependency_failure` (ADR-P2-30). Making the
/// reason part of the status means a Run cannot fail without saying how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Ok,
    Failed(TerminalReason),
    RejectedIntegrity,
}

impl RunStatus {
    /// Whether this status means the Run produced usable metrics.
    #[must_use]
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }
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

/// What the executor measured about one instrument **from the run's own bars,
/// over the run's own window** (SPEC §12.3 Gate 4).
///
/// It lives on the result rather than being looked up later on purpose. ADV and
/// volatility read at gate time would be *today's* liquidity judged against a
/// backtest that ran in 2023 — a non-PIT read, and the kind that looks harmless
/// because the number is real. Measured here, it is a fact about the same data
/// the Run saw.
///
/// Every field but the name is `Option`, and absence is honest: the synthetic
/// executor has no bars and declares none, which leaves Gate 4 inconclusive
/// rather than passing on a guess.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InstrumentStat {
    pub instrument: String,
    /// The venue's quote currency — the unit this instrument's P&L is in. Gate
    /// 12's concentration share refuses to sum across two of them.
    #[serde(default)]
    pub numeraire: Option<String>,
    /// 20-day rolling dollar ADV over the run's window.
    #[serde(default)]
    pub adv_usd: Option<f64>,
    /// Daily close-to-close return volatility over the same window.
    #[serde(default)]
    pub daily_vol: Option<f64>,
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
    /// Per-instrument liquidity measured from this Run's own bars (Gate 4).
    /// Empty when the executor had no bars to measure.
    #[serde(default)]
    pub instrument_stats: Vec<InstrumentStat>,
    pub compute_cost: ComputeCost,
    pub produced_at: DateTime<Utc>,
    /// Engine version (see [`super::ENGINE_VERSION`]).
    pub produced_by: String,
    /// Carries the `unsafe` bit of the producing config (INV-1).
    #[serde(default)]
    pub unsafe_: bool,
    /// How a run on the one strategy engine ran (Set U U-3.3). `None` for runs
    /// from the older executors, which recorded none of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<EngineManifest>,
}

/// What a v3 run records about how it ran: the facts a reader needs to know
/// what its numbers rest on (Set U U-3.3, FEAT-007 §9.4).
///
/// The numbers alone cannot say whether a quiet strategy was quiet because its
/// signal never fired, because a model never answered, or because the risk gate
/// refused it; this can.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineManifest {
    /// `strategy-engine/v3`.
    pub engine: String,
    /// The graph revision hash the run compiled.
    pub revision: String,
    /// `next_bar_open` or `current_close`.
    pub fill_policy: String,
    /// The fill policy traded at prices nobody could have traded at.
    pub optimistic_fills: bool,
    /// Where orders executed.
    pub venue: String,
    /// The risk limits every order passed, by name.
    pub risk_limits: std::collections::BTreeMap<String, String>,
    /// Every model the plan calls, at the version it was pinned to.
    pub model_pins: Vec<ModelPin>,
    /// Walk-forward folds used to answer model requests point-in-time. Empty
    /// when the plan calls no model.
    pub folds: Vec<FoldUse>,
    /// Decision ticks taken inside the evaluation window.
    pub decisions: u64,
    /// Matched rules that held because a value their intent needed was
    /// `missing`, by reason.
    pub holds_by_reason: std::collections::BTreeMap<String, u64>,
    /// Rules whose condition was `missing` and so did not match, by reason.
    pub unknown_by_reason: std::collections::BTreeMap<String, u64>,
    /// Intents the planner could not carry out, by reason.
    pub planner_refusals: std::collections::BTreeMap<String, u64>,
    /// Every order the risk gate refused.
    pub gate_rejections: Vec<GateRejectionRecord>,
    /// SHA-256 over every decision trace's digest, in order — one value that
    /// changes if any decision changes (ADR-0047).
    pub journal_digest: String,
    /// The decisions that did something: placed or cancelled an order, held, or
    /// had an intent refused.
    pub journal: Vec<JournalEntry>,
}

/// A model the plan calls, pinned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPin {
    pub node: String,
    pub model: String,
    pub version: u32,
}

/// One walk-forward fold's use in a run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldUse {
    pub model: String,
    pub version: u32,
    pub fold: String,
    /// Requests this fold answered.
    pub answered: u64,
}

/// An order the risk gate refused.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateRejectionRecord {
    pub at: DateTime<Utc>,
    pub instrument: String,
    /// The planner's tag for the order.
    pub tag: String,
    /// The check that failed, e.g. `PositionLimitExceeded`.
    pub check: String,
    pub detail: String,
}

/// One decision in the journal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub at_ns: i64,
    pub tick: u64,
    /// What caused it: `bar_close`, `fill`, `order_closed`, `schedule`.
    pub cause: String,
    /// The decision trace's digest.
    pub digest: String,
    pub actions: u32,
    pub refusals: u32,
    pub holds: u32,
}

impl RunResult {
    /// Build a `Failed` result for `cfg` — used so an erroring execution is still
    /// a recorded, counted Run (never dropped).
    ///
    /// `terminal` is what §9 records and therefore which censoring INV-17
    /// applies; `detail` is for a human reading the flag.
    #[must_use]
    pub fn failed(
        cfg: &RunConfig,
        terminal: TerminalReason,
        reason: impl Into<String>,
        produced_by: impl Into<String>,
    ) -> Self {
        Self {
            run_id: cfg.run_id.clone(),
            status: RunStatus::Failed(terminal),
            equity_curve: Vec::new(),
            net_exposure: Vec::new(),
            trades: Vec::new(),
            metrics: MetricSet::empty(),
            integrity_flags: vec![Flag {
                code: "run.failed".into(),
                detail: reason.into(),
            }],
            instrument_stats: Vec::new(),
            compute_cost: ComputeCost::default(),
            produced_at: Utc::now(),
            produced_by: produced_by.into(),
            unsafe_: cfg.unsafe_,
            manifest: None,
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
            instrument_stats: Vec::new(),
            compute_cost: ComputeCost::default(),
            produced_at: Utc::now(),
            produced_by: produced_by.into(),
            unsafe_: cfg.unsafe_,
            manifest: None,
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
            instrument_stats: Vec::new(),
            compute_cost: ComputeCost {
                wall_ms: 5,
                cpu_ms: 4,
            },
            produced_at: Utc::now(),
            produced_by: "engine@test".into(),
            unsafe_: false,
            manifest: None,
        };
        let back: RunResult = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn failed_carries_unsafe_and_a_reason() {
        let c = cfg();
        let r = RunResult::failed(&c, TerminalReason::NanDivergence, "boom", "engine@test");
        assert_eq!(r.status, RunStatus::Failed(TerminalReason::NanDivergence));
        assert_eq!(r.run_id, c.run_id);
        assert_eq!(r.integrity_flags.len(), 1);
    }
}
