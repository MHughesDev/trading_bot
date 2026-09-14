//! The **Run** — the smallest reproducible unit of the Backtest Suite (spec
//! §1.1). One strategy × params × data slice × cost model × seed → one equity
//! curve and its metrics. A Run is a pure, dumb, content-addressed function
//! `RunConfig → RunResult` (ADR-001): cacheable by `run_id`, immutable once
//! executed, and ignorant of everything above it (Studies, nulls, counters).
//!
//! [`Backtest`] is the single, cache-aware, funnel-facing entry point: it
//! computes the `run_id`, serves a cached result when present (no re-execution,
//! no re-count), otherwise executes and immutably stores the result.

pub mod config;
pub mod executor;
pub mod id;
pub mod metrics;
pub mod objective;
pub mod result;
pub mod store;

pub use config::{
    Construction, DataSlice, EvalResolution, FillModel, NullWorld, ParamMap, RunConfig,
    RunConfigBuilder, UnsafeFlags,
};
pub use executor::{ClosureExecutor, RunExecutor};
pub use id::RunId;
pub use metrics::{MetricInputs, MetricKind, MetricSet};
pub use objective::{Aggregate, Constraint, Objective};
pub use result::{ComputeCost, Flag, RunResult, RunStatus, Side, Trade};
pub use store::{InMemoryRunStore, PutOutcome, RunStore};

use ledger::{
    DispatchContext, LedgerError, OutcomeVector, Registration, TerminalReason, TrialEvent, TrialLedger, TrialState,
    TrialSubject,
};

/// Engine version stamped on every `RunResult.produced_by`. Two runs from the
/// same engine share this; it changes when the simulator SDK rev or this crate's
/// version changes, so a result's provenance is always legible.
pub const ENGINE_VERSION: &str = concat!("backtest@", env!("CARGO_PKG_VERSION"), "+sim-sdk");

/// What the ledger records a Run as.
#[must_use]
pub fn trial_subject(cfg: &RunConfig) -> TrialSubject {
    let slice_id = dataplane::content_hash(&(&cfg.data_slice, &cfg.data_snapshot))
        .map_or_else(|_| "slice:unhashable".to_string(), |h| format!("slice:{h}"));
    TrialSubject {
        config_hash: cfg.run_id.as_str().to_string(),
        config: serde_json::to_value(cfg).unwrap_or(serde_json::Value::Null),
        dataset_id: slice_id,
        split_spec_id: None,
        code_hash: format!("{}@{}", cfg.strategy_ref, cfg.strategy_version),
        image_digest: ENGINE_VERSION.to_string(),
        seed_set: vec![cfg.seed as i64],
        non_reproducible: cfg.unsafe_,
        overlapping_labels_unweighted: false,
        split_overrides: serde_json::json!([]),
        planned_steps: None,
    }
}

/// The typed outcome of a Run. Metrics the engine does not produce stay `None`.
#[must_use]
pub fn outcome_of(result: &RunResult) -> OutcomeVector {
    let m = &result.metrics;
    let finite = |x: f64| x.is_finite().then_some(x);
    OutcomeVector {
        sharpe_net: finite(m.sharpe),
        sortino_net: finite(m.sortino),
        calmar: finite(m.calmar),
        max_dd: finite(m.max_drawdown),
        turnover_annual: finite(m.turnover),
        ..OutcomeVector::default()
    }
}

/// The terminal ledger event for an executed Run. Every path sets censoring.
#[must_use]
pub fn terminal_event(result: &RunResult) -> TrialEvent {
    let detail = result.integrity_flags.first().map(|f| format!("{}: {}", f.code, f.detail)).unwrap_or_default();
    match result.status {
        RunStatus::Ok => TrialEvent::completed(result.run_id.as_str(), Some(outcome_of(result))),
        RunStatus::RejectedIntegrity => TrialEvent::failed(TerminalReason::IntegrityRejected, detail),
        // The reason is part of the status (ADR-P2-30). There is nothing to infer
        // here, and so nothing that a reworded message can change.
        RunStatus::Failed(reason) => TrialEvent::failed(reason, detail),
    }
}

/// One Run dispatch: provenance, config and the probability it was chosen.
#[derive(Clone, Copy, Debug)]
pub struct RunDispatch<'a> {
    pub ctx: &'a DispatchContext,
    pub cfg: &'a RunConfig,
    pub propensity: Option<f64>,
    pub exploration_flag: bool,
}

impl<'a> RunDispatch<'a> {
    #[must_use]
    pub fn new(ctx: &'a DispatchContext, cfg: &'a RunConfig, propensity: f64) -> Self {
        Self { ctx, cfg, propensity: Some(propensity), exploration_flag: false }
    }

    /// No propensity: accepted by the ledger only under the legacy marker.
    #[must_use]
    pub fn unlogged(ctx: &'a DispatchContext, cfg: &'a RunConfig) -> Self {
        Self { ctx, cfg, propensity: None, exploration_flag: false }
    }

    #[must_use]
    pub fn exploration(mut self) -> Self {
        self.exploration_flag = true;
        self
    }
}

/// The cache-aware Run entry point. Studies and gates call only this.
///
/// Register, then dispatch (INV-16). A cache hit still registers and settles as
/// `deduplicated` naming the prior trial, because trial accounting counts looks.
pub struct Backtest<S: RunStore, E: RunExecutor, L: TrialLedger> {
    store: S,
    executor: E,
    ledger: L,
}

/// Whether a [`Backtest::run`] call executed or served a cached result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOrigin {
    Executed,
    /// Served from the artifact cache. Still a counted look.
    CacheHit,
}

impl<S: RunStore, E: RunExecutor, L: TrialLedger> Backtest<S, E, L> {
    pub fn new(store: S, executor: E, ledger: L) -> Self {
        Self { store, executor, ledger }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn ledger(&self) -> &L {
        &self.ledger
    }

    /// # Errors
    /// A refused registration dispatches no compute, by construction.
    pub fn run_traced(&self, d: &RunDispatch<'_>) -> Result<(RunResult, RunOrigin), LedgerError> {
        self.run_traced_with_trial(d).map(|(r, o, _)| (r, o))
    }

    /// As [`Self::run_traced`], and also the id of the trial it registered.
    ///
    /// The id is what lets a caller attach statistics to the run afterwards
    /// (`TrialLedger::record_statistic`, ADR-P2-31). A gate that runs later, in a
    /// different process, reads those from the ledger — the alternative is
    /// passing them through a job manifest, which would make them numbers the
    /// submitter chose.
    ///
    /// # Errors
    /// Whatever the ledger refuses.
    pub fn run_traced_with_trial(
        &self,
        d: &RunDispatch<'_>,
    ) -> Result<(RunResult, RunOrigin, uuid::Uuid), LedgerError> {
        let cfg = d.cfg;
        let subject = trial_subject(cfg);
        let reg = Registration {
            exploration_flag: d.exploration_flag,
            propensity: d.propensity,
            ..Registration::new(d.ctx, &subject, 1.0)
        };
        let mut ticket = self.ledger.register(&reg)?;

        if let Some(cached) = self.store.get(&cfg.run_id) {
            // Dedup must name the trial whose artifact it reuses. If the ledger has no
            // such trial, lineage cannot be proven, so the run executes instead.
            if let Some(prior) = self.ledger.prior_trial(&d.ctx.tenant_id, cfg.run_id.as_str()) {
                let trial_id = ticket.trial_id();
                self.ledger.settle(ticket, TrialEvent::deduplicated(prior, cfg.run_id.as_str()))?;
                return Ok((cached, RunOrigin::CacheHit, trial_id));
            }
        }

        self.ledger.transition(&mut ticket, TrialEvent::to(TrialState::Running))?;
        let result = self.executor.execute(cfg, &ticket);
        debug_assert_eq!(result.run_id, cfg.run_id, "executor must echo run_id");
        self.store.put(result.clone());
        // INV-18: the return series is on record before the completion that reports on it.
        let mut event = terminal_event(&result);
        if event.state == Some(TrialState::Completed) {
            let series = ledger::neff::ReturnSeries::from_equity(&result.equity_curve);
            if series.is_empty() {
                // No series, no portfolio metric: a Sharpe without its returns cannot be
                // re-scored, clustered or verified.
                if let Some(o) = event.outcome.as_mut() {
                    o.sharpe_net = None;
                    o.sortino_net = None;
                    o.calmar = None;
                }
            } else {
                event = event.with_returns(self.ledger.persist_returns(&ticket, &series)?);
            }
        }
        let trial_id = ticket.trial_id();
        self.ledger.settle(ticket, event)?;
        Ok((result, RunOrigin::Executed, trial_id))
    }

    /// # Errors
    /// A refused registration.
    pub fn run(&self, d: &RunDispatch<'_>) -> Result<RunResult, LedgerError> {
        Ok(self.run_traced(d)?.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger::{DispatchContext, InMemoryLedger};
    use crate::run::executor::{daily_curve, map_sim_result};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn ctx() -> DispatchContext {
        DispatchContext::human("tenant-a", "tester", 0.1).with_experiment("exp")
    }

    fn cfg(seed: u64) -> RunConfig {
        use chrono::{TimeZone, Utc};
        let s = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        RunConfigBuilder::new("s", "v", s, "c", "z", "snap")
            .seed(seed)
            .build()
    }

    /// An executor that counts how many times it actually ran.
    fn counting_executor(counter: Arc<AtomicUsize>) -> impl RunExecutor {
        ClosureExecutor(move |c: &RunConfig| {
            counter.fetch_add(1, Ordering::SeqCst);
            map_sim_result(
                c,
                daily_curve(&[100.0, 101.0, 102.0]),
                vec![],
                vec![],
                ComputeCost::default(),
                ENGINE_VERSION,
            )
        })
    }

    fn engine(
        counter: Arc<AtomicUsize>,
    ) -> Backtest<InMemoryRunStore, impl RunExecutor, InMemoryLedger> {
        Backtest::new(
            InMemoryRunStore::new(),
            counting_executor(counter),
            InMemoryLedger::new(),
        )
    }

    #[test]
    fn identical_config_executes_once_then_caches() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = engine(counter.clone());
        let ctx = ctx();
        let c = cfg(1);
        let (_, o1) = bt.run_traced(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
        let (_, o2) = bt.run_traced(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
        assert_eq!(o1, RunOrigin::Executed);
        assert_eq!(o2, RunOrigin::CacheHit);
        assert_eq!(counter.load(Ordering::SeqCst), 1, "executed exactly once");
    }

    #[test]
    fn a_cache_hit_is_still_a_counted_look() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = engine(counter);
        let ctx = ctx();
        let c = cfg(1);
        bt.run(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
        bt.run(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
        assert_eq!(
            bt.ledger().trial_count("tenant-a"),
            2,
            "dedup saves compute, not trial count"
        );
        assert_eq!(bt.store().len(), 1, "one artifact");
    }

    #[test]
    fn a_refused_registration_dispatches_no_compute() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = engine(counter.clone());
        let ctx = ctx();
        let c = cfg(1);
        // No propensity under a non-legacy policy: the ledger refuses.
        let reg = RunDispatch::unlogged(&ctx, &c);
        assert!(bt.run(&reg).is_err());
        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "INV-16: no compute without a registered trial"
        );
        assert_eq!(bt.store().len(), 0);
    }

    #[test]
    fn distinct_configs_execute_independently() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = engine(counter.clone());
        let ctx = ctx();
        let (c1, c2) = (cfg(1), cfg(2));
        bt.run(&RunDispatch::new(&ctx, &c1, 0.5)).unwrap();
        bt.run(&RunDispatch::new(&ctx, &c2, 0.5)).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert_eq!(bt.store().len(), 2);
    }

    #[test]
    fn produced_by_is_stamped() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = engine(counter);
        let ctx = ctx();
        let c = cfg(3);
        let r = bt.run(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
        assert_eq!(r.produced_by, ENGINE_VERSION);
    }

    #[test]
    fn failed_runs_are_settled_as_censored() {
        let ledger = InMemoryLedger::new();
        let bt = Backtest::new(
            InMemoryRunStore::new(),
            ClosureExecutor(|c: &RunConfig| RunResult::failed(c, TerminalReason::DependencyFailure, "boom", ENGINE_VERSION)),
            ledger,
        );
        let ctx = ctx();
        let c = cfg(9);
        bt.run(&RunDispatch::new(&ctx, &c, 0.5)).unwrap();
        let rows = bt.ledger().rows("tenant-a");
        assert_eq!(rows.len(), 1, "a failure is still a recorded trial");
        let (state, cens) = bt.ledger().state_of(rows[0].trial_id).unwrap();
        assert_eq!(state, ledger::TrialState::Failed);
        assert_eq!(
            cens, ledger::Censoring::Failed,
            "INV-17: every terminal transition sets censoring"
        );
    }
}
