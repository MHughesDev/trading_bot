//! Experiment persistence (spec §1.3). The relational schema lives in
//! `migrations/0028_backtest_experiments.sql`; this module defines the trait and
//! an in-memory reference implementation. The vault access log is append-only —
//! the store never offers a way to clear it or to un-spend the vault, which is
//! the structural guarantee behind J-2.9 (trial count cannot be laundered).

use std::collections::HashMap;
use std::sync::Mutex;

use super::Experiment;

/// Persistence for Experiments.
pub trait ExperimentStore: Send + Sync {
    /// Insert or update an Experiment by id (the in-process aggregate is the
    /// source of truth; this persists snapshots).
    fn upsert(&self, experiment: Experiment);
    /// Fetch an Experiment by id.
    fn get(&self, experiment_id: &str) -> Option<Experiment>;
}

/// In-memory reference store.
#[derive(Default)]
pub struct InMemoryExperimentStore {
    inner: Mutex<HashMap<String, Experiment>>,
}

impl InMemoryExperimentStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ExperimentStore for InMemoryExperimentStore {
    fn upsert(&self, experiment: Experiment) {
        self.inner
            .lock()
            .expect("experiment store poisoned")
            .insert(experiment.experiment_id.clone(), experiment);
    }

    fn get(&self, experiment_id: &str) -> Option<Experiment> {
        self.inner
            .lock()
            .expect("experiment store poisoned")
            .get(experiment_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{DataSlice, EvalResolution};
    use chrono::{TimeZone, Utc};

    #[test]
    fn upsert_round_trips() {
        let store = InMemoryExperimentStore::new();
        let slice = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        let e = Experiment::new("exp-1", "fam", slice, "null:x");
        store.upsert(e.clone());
        assert_eq!(store.get("exp-1").unwrap(), e);
    }
}
