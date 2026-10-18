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
pub use result::{
    ComputeCost, EngineManifest, Flag, FoldUse, GateRejectionRecord, InstrumentStat, JournalEntry,
    ModelPin, RunResult, RunStatus, Side, Trade,
};
pub use store::{InMemoryRunStore, PutOutcome, RunStore};

use ledger::{
    DispatchContext, LedgerError, OutcomeVector, Registration, TerminalReason, TrialEvent,
    TrialLedger, TrialState, TrialSubject,
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
    let detail = result
        .integrity_flags
        .first()
        .map(|f| format!("{}: {}", f.code, f.detail))
        .unwrap_or_default();
    match result.status {
        RunStatus::Ok => TrialEvent::completed(result.run_id.as_str(), Some(outcome_of(result))),
        RunStatus::RejectedIntegrity => {
            TrialEvent::failed(TerminalReason::IntegrityRejected, detail)
        }
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
        Self {
            ctx,
            cfg,
            propensity: Some(propensity),
            exploration_flag: false,
        }
    }

    /// No propensity: accepted by the ledger only under the legacy marker.
    #[must_use]
    pub fn unlogged(ctx: &'a DispatchContext, cfg: &'a RunConfig) -> Self {
        Self {
            ctx,
            cfg,
            propensity: None,
            exploration_flag: false,
        }
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
    /// The config each executed result came from, and the receipt the executor
    /// gave for it. A `RunResult` does not carry its own config, so without these
    /// a stored result cannot be re-run (S-10.8). In memory like the store they
    /// sit beside: a result that has not survived a restart has no config to
    /// lose.
    provenance: std::sync::Mutex<std::collections::HashMap<RunId, Provenance>>,
}

/// What a reproduction needs to re-run a stored result.
#[derive(Clone)]
struct Provenance {
    cfg: RunConfig,
    receipt: Option<crate::receipt::ReproSpec>,
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
        Self {
            store,
            executor,
            ledger,
            provenance: std::sync::Mutex::default(),
        }
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
            if let Some(prior) = self
                .ledger
                .prior_trial(&d.ctx.tenant_id, cfg.run_id.as_str())
            {
                let trial_id = ticket.trial_id();
                self.ledger
                    .settle(ticket, TrialEvent::deduplicated(prior, cfg.run_id.as_str()))?;
                return Ok((cached, RunOrigin::CacheHit, trial_id));
            }
        }

        self.ledger
            .transition(&mut ticket, TrialEvent::to(TrialState::Running))?;
        let result = self.executor.execute(cfg, &ticket);
        debug_assert_eq!(result.run_id, cfg.run_id, "executor must echo run_id");
        if self.store.put(result.clone()) == PutOutcome::Stored {
            // First time this run id was executed: keep what a reproduction needs.
            self.provenance
                .lock()
                .expect("provenance lock poisoned")
                .insert(
                    cfg.run_id.clone(),
                    Provenance {
                        cfg: cfg.clone(),
                        receipt: self.executor.receipt(cfg),
                    },
                );
        }
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

    /// Re-run a stored result under its own config and report whether the platform
    /// still computes the same thing (AGENT-013 §6.9, §8.4; S-10.8).
    ///
    /// **A reproduction is not a look.** It runs under a reproduction ticket the
    /// ledger mints for the completed trial that produced the result
    /// ([`TrialLedger::begin_reproduction`], ADR-S10-18): no trial is registered, no
    /// counter moves, nothing enters the hash chain. It bypasses the artifact cache
    /// on purpose — a cache hit would return the stored result and prove nothing —
    /// and it never stores the re-run: the stored result is the thing under test.
    ///
    /// The finding is appended to `mlops.reproduction` through the same ticket.
    ///
    /// # Errors
    /// [`ReproduceError`].
    pub fn reproduce(
        &self,
        tenant_id: &str,
        run_id: &RunId,
        reproduced_by: &str,
    ) -> Result<ReproduceReport, ReproduceError> {
        let stored = self
            .store
            .get(run_id)
            .ok_or_else(|| ReproduceError::UnknownRun(run_id.as_str().to_string()))?;
        let prov = self
            .provenance
            .lock()
            .expect("provenance lock poisoned")
            .get(run_id)
            .cloned()
            .ok_or_else(|| ReproduceError::ConfigNotRetained(run_id.as_str().to_string()))?;
        if stored.status != RunStatus::Ok {
            return Err(ReproduceError::NotCompleted(run_id.as_str().to_string()));
        }
        let ticket = self.ledger.begin_reproduction(tenant_id, run_id.as_str())?;
        let rerun = self.executor.execute(&prov.cfg, &ticket);
        let report = compare_rerun(
            &stored,
            &rerun,
            prov.receipt.as_ref(),
            self.executor.receipt(&prov.cfg),
        );
        let record = report.record(run_id.as_str(), reproduced_by);
        self.ledger.record_reproduction(ticket, &record)?;
        Ok(report)
    }

    /// Every stored run id that has a config on record, i.e. every run this
    /// process executed. The nightly sample draws from this.
    #[must_use]
    pub fn reproducible_runs(&self) -> Vec<RunId> {
        let mut ids: Vec<RunId> = self
            .provenance
            .lock()
            .expect("provenance lock poisoned")
            .keys()
            .cloned()
            .collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids
    }
}

/// Why a reproduction could not be made.
#[derive(Debug, thiserror::Error)]
pub enum ReproduceError {
    #[error("no stored result for run `{0}`")]
    UnknownRun(String),
    #[error(
        "run `{0}` has no config on record in this process (it predates the last restart); a \
         result that cannot name the config it came from cannot be re-run"
    )]
    ConfigNotRetained(String),
    #[error("run `{0}` did not complete, so there is no number to reproduce")]
    NotCompleted(String),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

/// What a reproduction found.
#[derive(Debug, Clone, PartialEq)]
pub struct ReproduceReport {
    pub reproduction: crate::receipt::Reproduction,
    /// The original carried no receipt, so the outcome rests on the number alone.
    pub receipt_missing: bool,
    pub receipt_stored: Option<crate::receipt::ReproSpec>,
    pub receipt_rerun: Option<crate::receipt::ReproSpec>,
    pub was: f64,
    pub now: f64,
}

impl ReproduceReport {
    /// The row `mlops.reproduction` appends.
    #[must_use]
    pub fn record(
        &self,
        run_id: &str,
        reproduced_by: &str,
    ) -> ledger::reproduction::ReproductionRecord {
        use crate::receipt::Reproduction as R;
        let (differences, was, now) = match &self.reproduction {
            R::BitIdentical => (serde_json::json!([]), None, None),
            R::DriftedUnderTheSameReceipt { was, now } => {
                (serde_json::json!([]), Some(*was), Some(*now))
            }
            R::ReceiptChanged { differences } => (
                serde_json::to_value(differences).unwrap_or_default(),
                None,
                None,
            ),
        };
        ledger::reproduction::ReproductionRecord {
            run_id: run_id.to_string(),
            outcome: self.reproduction.as_str().to_string(),
            receipt_stored: self
                .receipt_stored
                .as_ref()
                .and_then(|r| serde_json::to_value(r).ok()),
            receipt_rerun: self
                .receipt_rerun
                .as_ref()
                .and_then(|r| serde_json::to_value(r).ok())
                .unwrap_or(serde_json::Value::Null),
            differences,
            was_value: was,
            now_value: now,
            reproduced_by: reproduced_by.to_string(),
        }
    }
}

/// A bit-level fingerprint of everything a result reports: the metrics, the
/// equity curve and the trades. Excludes `produced_at` and `compute_cost`, which
/// legitimately differ between two executions of the same computation.
#[must_use]
pub fn result_fingerprint(r: &RunResult) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let mut feed = |tag: &str, v: &dyn erased_json::Json| {
        h.update(tag.as_bytes());
        h.update(b"=");
        h.update(v.json().as_bytes());
        h.update(b"\x1f");
    };
    feed("metrics", &r.metrics);
    feed("equity", &r.equity_curve);
    feed("exposure", &r.net_exposure);
    feed("trades", &r.trades);
    feed("flags", &r.integrity_flags);
    format!("sha256:{}", hex::encode(h.finalize()))
}

mod erased_json {
    pub trait Json {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Json for T {
        fn json(&self) -> String {
            serde_json::to_string(self).unwrap_or_default()
        }
    }
}

/// Decide what a re-run found. Receipts first — a moved receipt explains a moved
/// number and is not a defect — then the bits of the result. There is no
/// tolerance (`receipt::reproduce`'s argument), and a result whose fingerprint
/// moved while its headline Sharpe did not is still drift.
#[must_use]
pub fn compare_rerun(
    stored: &RunResult,
    rerun: &RunResult,
    stored_receipt: Option<&crate::receipt::ReproSpec>,
    rerun_receipt: Option<crate::receipt::ReproSpec>,
) -> ReproduceReport {
    use crate::receipt::{reproduce, Reproduction};
    let was = stored.metrics.sharpe;
    let now = rerun.metrics.sharpe;
    // With no receipt on one side the two cannot be compared, so the outcome rests
    // on the number alone and says so: both sides are then given the same
    // placeholder receipt, which can only agree.
    let receipt_missing = stored_receipt.is_none() || rerun_receipt.is_none();
    let placeholder = crate::receipt::ReproSpec {
        dataset_hash: "unrecorded".into(),
        feature_registry_version: "unrecorded".into(),
        calendar_version: "unrecorded".into(),
        engine_version: stored.produced_by.clone(),
        strategy_version_hash: "unrecorded".into(),
        cost_model_version: "unrecorded".into(),
        seed_digest: "unrecorded".into(),
        harness_version: "unrecorded".into(),
    };
    let (a, b) = if receipt_missing {
        (placeholder.clone(), placeholder)
    } else {
        (
            stored_receipt.cloned().expect("checked"),
            rerun_receipt.clone().expect("checked"),
        )
    };
    let mut verdict = reproduce(&a, &b, was, now);
    if matches!(verdict, Reproduction::BitIdentical)
        && result_fingerprint(stored) != result_fingerprint(rerun)
    {
        verdict = Reproduction::DriftedUnderTheSameReceipt { was, now };
    }
    ReproduceReport {
        reproduction: verdict,
        receipt_missing,
        receipt_stored: stored_receipt.cloned(),
        receipt_rerun: rerun_receipt,
        was,
        now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::executor::{daily_curve, map_sim_result};
    use ledger::{DispatchContext, InMemoryLedger};
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
            ClosureExecutor(|c: &RunConfig| {
                RunResult::failed(c, TerminalReason::DependencyFailure, "boom", ENGINE_VERSION)
            }),
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
            cens,
            ledger::Censoring::Failed,
            "INV-17: every terminal transition sets censoring"
        );
    }
}
