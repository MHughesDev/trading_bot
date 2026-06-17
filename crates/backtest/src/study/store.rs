//! Study persistence (spec §1.2, "What the Study logs").
//!
//! A Study logs its `StudyConfig` (including the `question` and any `null_ref`)
//! at **creation time** — before results exist — and its `StudyResult`
//! distribution + `trial_delta` + member references when it completes. The
//! relational schema lives in `migrations/0027_backtest_studies.sql`; this
//! module defines the trait and an in-memory reference implementation. The
//! `created_at` of the config is always earlier than the result, which is the
//! audit property that makes post-hoc reinterpretation detectable.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};

use super::config::StudyConfig;
use super::result::StudyResult;

/// A logged study record: config (with creation time) + optional result.
#[derive(Clone, Debug)]
pub struct StudyRecord {
    pub config: StudyConfig,
    pub question_logged_at: DateTime<Utc>,
    pub result: Option<StudyResult>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Persistence for Study configs and results.
pub trait StudyStore: Send + Sync {
    /// Log a Study's config (and `question`) at creation, before it runs.
    fn create(&self, config: StudyConfig) -> DateTime<Utc>;
    /// Attach the completed result to a previously-created Study.
    fn complete(&self, study_id: &str, result: StudyResult);
    /// Fetch the full record.
    fn get(&self, study_id: &str) -> Option<StudyRecord>;
}

/// In-memory reference store.
#[derive(Default)]
pub struct InMemoryStudyStore {
    inner: Mutex<HashMap<String, StudyRecord>>,
}

impl InMemoryStudyStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl StudyStore for InMemoryStudyStore {
    fn create(&self, config: StudyConfig) -> DateTime<Utc> {
        let now = Utc::now();
        let id = config.study_id.clone();
        self.inner.lock().expect("study store poisoned").insert(
            id,
            StudyRecord {
                config,
                question_logged_at: now,
                result: None,
                completed_at: None,
            },
        );
        now
    }

    fn complete(&self, study_id: &str, result: StudyResult) {
        let mut map = self.inner.lock().expect("study store poisoned");
        if let Some(rec) = map.get_mut(study_id) {
            rec.result = Some(result);
            rec.completed_at = Some(Utc::now());
        }
    }

    fn get(&self, study_id: &str) -> Option<StudyRecord> {
        self.inner.lock().expect("study store poisoned").get(study_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::{DataSlice, EvalResolution, MetricKind, RunConfigBuilder};
    use crate::study::config::{SelectionRule, StudyBudget, StudyKind, VarySpec};
    use crate::study::result::{Distribution, StudyVerdict};
    use chrono::TimeZone;

    fn study() -> StudyConfig {
        let s = DataSlice::new(
            "u",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2024, 6, 1, 0, 0, 0).unwrap(),
            EvalResolution::Day1,
        );
        StudyConfig {
            study_id: "s1".into(),
            kind: StudyKind::ParameterSweep,
            base_config: RunConfigBuilder::new("s", "v", s, "c", "z", "snap").build(),
            vary: VarySpec::Params { grid: vec![] },
            metric: MetricKind::Sharpe,
            null_ref: None,
            budget: StudyBudget::default(),
            question: "logged up front".into(),
            selection_rule: SelectionRule::None,
        }
    }

    #[test]
    fn question_is_logged_before_the_result() {
        let store = InMemoryStudyStore::new();
        let logged = store.create(study());
        std::thread::sleep(std::time::Duration::from_millis(2));
        let res = StudyResult::new(
            "s1".into(),
            vec![],
            Distribution::from_values(MetricKind::Sharpe, vec![1.0]),
            StudyVerdict {
                summary: "ok".into(),
                positive_median: true,
                survivable_worst5: true,
                plateau: None,
            },
            0,
            None,
            false,
        );
        store.complete("s1", res);
        let rec = store.get("s1").unwrap();
        assert!(rec.question_logged_at == logged);
        assert!(rec.completed_at.unwrap() > rec.question_logged_at);
        assert!(rec.result.is_some());
    }
}
