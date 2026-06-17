//! The immutable, content-addressed run store (spec §1.1, "What the Run logs").
//!
//! Every executed Run — `Ok`, `Failed`, or `RejectedIntegrity` — is written
//! here keyed by `run_id`. Writes are **idempotent**: a second `put` of the same
//! id is a no-op, never an update. Nothing is ever deleted or mutated. The
//! Postgres + ClickHouse schemas live in `migrations/0026_backtest_runs.sql` and
//! `clickhouse/05_backtest_run_series.sql`; this module defines the trait and an
//! in-memory reference implementation used by the funnel and by tests.

use std::collections::HashMap;
use std::sync::Mutex;

use super::id::RunId;
use super::result::RunResult;

/// Outcome of a store write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutOutcome {
    /// The result was newly stored.
    Stored,
    /// An identical `run_id` already existed; the write was a no-op (cache).
    AlreadyPresent,
}

/// Immutable, content-addressed run storage.
pub trait RunStore: Send + Sync {
    /// Idempotently store a result. Returns whether it was new or already
    /// present. Must never overwrite an existing `run_id`.
    fn put(&self, result: RunResult) -> PutOutcome;

    /// Fetch a stored result by id (a cache hit when present).
    fn get(&self, run_id: &RunId) -> Option<RunResult>;

    /// True if `run_id` is stored.
    fn contains(&self, run_id: &RunId) -> bool {
        self.get(run_id).is_some()
    }

    /// Number of stored runs (across all statuses).
    fn len(&self) -> usize;

    /// True if the store holds no runs.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// In-memory reference store: idempotent, immutable, thread-safe.
#[derive(Default)]
pub struct InMemoryRunStore {
    inner: Mutex<HashMap<RunId, RunResult>>,
}

impl InMemoryRunStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl RunStore for InMemoryRunStore {
    fn put(&self, result: RunResult) -> PutOutcome {
        let mut map = self.inner.lock().expect("run store mutex poisoned");
        if map.contains_key(&result.run_id) {
            // Immutable: the first write wins, forever. A second put is a no-op.
            PutOutcome::AlreadyPresent
        } else {
            map.insert(result.run_id.clone(), result);
            PutOutcome::Stored
        }
    }

    fn get(&self, run_id: &RunId) -> Option<RunResult> {
        self.inner
            .lock()
            .expect("run store mutex poisoned")
            .get(run_id)
            .cloned()
    }

    fn len(&self) -> usize {
        self.inner.lock().expect("run store mutex poisoned").len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::config::{DataSlice, EvalResolution, RunConfigBuilder};
    use crate::run::result::{RunResult, RunStatus};
    use chrono::{TimeZone, Utc};

    fn result(seed: u64, status: RunStatus) -> RunResult {
        let s = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        let cfg = RunConfigBuilder::new("s", "v", s, "c", "z", "snap")
            .seed(seed)
            .build();
        let mut r = RunResult::failed(&cfg, "x", "engine@test");
        r.status = status;
        r
    }

    #[test]
    fn put_is_idempotent_and_immutable() {
        let store = InMemoryRunStore::new();
        let r = result(1, RunStatus::Ok);
        let id = r.run_id.clone();
        assert_eq!(store.put(r.clone()), PutOutcome::Stored);
        // A second put of the same id is a no-op, even with a mutated body.
        let mut mutated = r.clone();
        mutated.status = RunStatus::Failed;
        assert_eq!(store.put(mutated), PutOutcome::AlreadyPresent);
        assert_eq!(store.len(), 1);
        // The original (Ok) survived; the mutation did not take.
        assert_eq!(store.get(&id).unwrap().status, RunStatus::Ok);
    }

    #[test]
    fn failed_and_rejected_runs_are_stored() {
        let store = InMemoryRunStore::new();
        store.put(result(1, RunStatus::Failed));
        store.put(result(2, RunStatus::RejectedIntegrity));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn distinct_ids_coexist() {
        let store = InMemoryRunStore::new();
        store.put(result(1, RunStatus::Ok));
        store.put(result(2, RunStatus::Ok));
        assert_eq!(store.len(), 2);
    }
}
