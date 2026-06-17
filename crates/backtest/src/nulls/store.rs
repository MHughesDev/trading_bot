//! Null registry persistence (spec §2.1). Nulls are reusable across Experiments
//! and immutable once stored (a new params set is a new `null_id`). The schema
//! lives in `migrations/0029_backtest_nulls.sql`; this is the trait + in-memory
//! reference, plus the recorded user *choice* (chosen null + any override reason),
//! which is what makes "the null was selected, not defaulted" auditable (J-3.7).

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{Null, NullId, NullKind};

/// A logged decision: which null a user chose for an Experiment, whether it was
/// the recommended one, and (if not) the override reason.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NullChoice {
    pub experiment_id: String,
    pub chosen: NullId,
    pub recommended: NullKind,
    pub was_override: bool,
    /// Required when `was_override` is true (an override carries a logged reason).
    pub override_reason: Option<String>,
    pub chosen_at: DateTime<Utc>,
}

impl NullChoice {
    /// Record a choice, deriving `was_override` from whether the chosen kind
    /// matches the recommendation. An override without a reason is rejected.
    ///
    /// # Errors
    /// Returns `Err` if the choice overrides the recommendation without a reason.
    pub fn record(
        experiment_id: impl Into<String>,
        chosen: &Null,
        recommended: NullKind,
        override_reason: Option<String>,
    ) -> Result<Self, &'static str> {
        let was_override = chosen.kind != recommended;
        if was_override && override_reason.as_ref().is_none_or(|r| r.trim().is_empty()) {
            return Err("overriding the recommended null requires a logged reason");
        }
        Ok(Self {
            experiment_id: experiment_id.into(),
            chosen: chosen.null_id.clone(),
            recommended,
            was_override,
            override_reason,
            chosen_at: Utc::now(),
        })
    }
}

/// Persistence for null definitions and the per-Experiment choice.
pub trait NullStore: Send + Sync {
    /// Idempotently store a null definition (immutable; same id is a no-op).
    fn put(&self, null: Null);
    fn get(&self, null_id: &NullId) -> Option<Null>;
    /// Log a user's null choice for an Experiment.
    fn record_choice(&self, choice: NullChoice);
    fn choice_for(&self, experiment_id: &str) -> Option<NullChoice>;
}

/// In-memory reference store.
#[derive(Default)]
pub struct InMemoryNullStore {
    nulls: Mutex<HashMap<NullId, Null>>,
    choices: Mutex<HashMap<String, NullChoice>>,
}

impl InMemoryNullStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl NullStore for InMemoryNullStore {
    fn put(&self, null: Null) {
        let mut map = self.nulls.lock().expect("null store poisoned");
        map.entry(null.null_id.clone()).or_insert(null);
    }
    fn get(&self, null_id: &NullId) -> Option<Null> {
        self.nulls
            .lock()
            .expect("null store poisoned")
            .get(null_id)
            .cloned()
    }
    fn record_choice(&self, choice: NullChoice) {
        self.choices
            .lock()
            .expect("null store poisoned")
            .insert(choice.experiment_id.clone(), choice);
    }
    fn choice_for(&self, experiment_id: &str) -> Option<NullChoice> {
        self.choices
            .lock()
            .expect("null store poisoned")
            .get(experiment_id)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nulls::{generators::recommend_null, NullParams};

    #[test]
    fn put_is_idempotent() {
        let store = InMemoryNullStore::new();
        let n = Null::new(NullKind::BlockPermutation, NullParams::default()).unwrap();
        store.put(n.clone());
        store.put(n.clone());
        assert_eq!(store.get(&n.null_id), Some(n));
    }

    #[test]
    fn accepting_the_recommendation_is_not_an_override() {
        let recommended = recommend_null("intraday_mean_reversion"); // BlockPermutation
        let chosen = Null::new(recommended, NullParams::default()).unwrap();
        let choice = NullChoice::record("exp-1", &chosen, recommended, None).unwrap();
        assert!(!choice.was_override);
    }

    #[test]
    fn override_requires_a_reason() {
        let recommended = recommend_null("intraday_mean_reversion"); // BlockPermutation
        let chosen = Null::new(NullKind::SignalReturnDecouple, NullParams::default()).unwrap();
        // No reason → rejected.
        assert!(NullChoice::record("exp-1", &chosen, recommended, None).is_err());
        // With a reason → recorded as an override.
        let choice = NullChoice::record(
            "exp-1",
            &chosen,
            recommended,
            Some("signal is the hypothesis".into()),
        )
        .unwrap();
        assert!(choice.was_override);
        assert!(choice.override_reason.is_some());
    }

    #[test]
    fn choice_is_logged_per_experiment() {
        let store = InMemoryNullStore::new();
        let recommended = recommend_null("general");
        let chosen = Null::new(recommended, NullParams::default()).unwrap();
        let choice = NullChoice::record("exp-7", &chosen, recommended, None).unwrap();
        store.record_choice(choice.clone());
        assert_eq!(store.choice_for("exp-7"), Some(choice));
    }
}
