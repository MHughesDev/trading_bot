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
pub mod result;
pub mod store;

pub use config::{
    Construction, DataSlice, EvalResolution, FillModel, ParamMap, RunConfig, RunConfigBuilder,
    UnsafeFlags,
};
pub use executor::{ClosureExecutor, RunExecutor};
pub use id::RunId;
pub use metrics::{MetricInputs, MetricKind, MetricSet};
pub use result::{ComputeCost, Flag, RunResult, RunStatus, Side, Trade};
pub use store::{InMemoryRunStore, PutOutcome, RunStore};

/// Engine version stamped on every `RunResult.produced_by`. Two runs from the
/// same engine share this; it changes when the simulator SDK rev or this crate's
/// version changes, so a result's provenance is always legible.
pub const ENGINE_VERSION: &str = concat!("backtest@", env!("CARGO_PKG_VERSION"), "+sim-sdk");

/// The cache-aware Run entry point. Studies (Phase 1) and gates (Phase 4) call
/// only this — never the executor or store directly.
pub struct Backtest<S: RunStore, E: RunExecutor> {
    store: S,
    executor: E,
}

/// Whether a [`Backtest::run`] call executed or served a cached result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOrigin {
    /// The config was not cached; it executed and was stored.
    Executed,
    /// An identical `run_id` was already stored; served from cache.
    CacheHit,
}

impl<S: RunStore, E: RunExecutor> Backtest<S, E> {
    /// Wire a cache-aware Run engine over a store and an executor.
    pub fn new(store: S, executor: E) -> Self {
        Self { store, executor }
    }

    /// The immutable store backing this engine.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Run a config, returning the result and whether it was executed or a cache
    /// hit. Identical configs collide on `run_id` and execute exactly once; the
    /// `RunOrigin` lets callers (and tests) prove the cache short-circuit.
    pub fn run_traced(&self, cfg: &RunConfig) -> (RunResult, RunOrigin) {
        if let Some(cached) = self.store.get(&cfg.run_id) {
            return (cached, RunOrigin::CacheHit);
        }
        let result = self.executor.execute(cfg);
        // The executor is contracted to echo the config's id; never trust it to
        // mutate identity.
        debug_assert_eq!(result.run_id, cfg.run_id, "executor must echo run_id");
        self.store.put(result.clone());
        (result, RunOrigin::Executed)
    }

    /// Run a config and return just the result (cache-aware).
    pub fn run(&self, cfg: &RunConfig) -> RunResult {
        self.run_traced(cfg).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::executor::{daily_curve, map_sim_result};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

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

    #[test]
    fn identical_config_executes_once_then_caches() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = Backtest::new(InMemoryRunStore::new(), counting_executor(counter.clone()));
        let c = cfg(1);
        let (_, o1) = bt.run_traced(&c);
        let (_, o2) = bt.run_traced(&c);
        assert_eq!(o1, RunOrigin::Executed);
        assert_eq!(o2, RunOrigin::CacheHit);
        assert_eq!(counter.load(Ordering::SeqCst), 1, "executed exactly once");
    }

    #[test]
    fn distinct_configs_execute_independently() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = Backtest::new(InMemoryRunStore::new(), counting_executor(counter.clone()));
        bt.run(&cfg(1));
        bt.run(&cfg(2));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        assert_eq!(bt.store().len(), 2);
    }

    #[test]
    fn produced_by_is_stamped() {
        let counter = Arc::new(AtomicUsize::new(0));
        let bt = Backtest::new(InMemoryRunStore::new(), counting_executor(counter));
        let r = bt.run(&cfg(3));
        assert_eq!(r.produced_by, ENGINE_VERSION);
    }
}
