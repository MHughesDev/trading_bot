//! The one registry of strategies running live in this process.
//!
//! Before this module there were two ways to ask the platform to trade and
//! neither worked:
//!
//! * `POST /api/strategies/:id/start` inserted an instance into the API's
//!   `InstanceManager`, which the hot path never read — stage 3 held
//!   `let mut strategy: Option<StrategyInstance> = None;` — so the call
//!   returned `200` and then produced no intents, for ever, silently.
//! * `POST /api/automations/:id/arm` set a database flag.  `armed_automations`
//!   had no caller, so nothing ever evaluated the spec.
//!
//! Both are now the same path (ADR-0038).  A door *declares* what should run
//! here; stage 3 of the in-process pipeline is the single thing that *runs* it.
//! There is deliberately no second evaluator: two silent paths is how a total
//! outage came to report success twice.
//!
//! ## Why a generation counter rather than a lock
//!
//! Stage 3 sees one `WorldEvent` per trade tick and must not lock or allocate
//! per event (Set E locked decision 12).  It owns its `StrategyInstance`s —
//! they carry mutable per-instrument state, so they cannot be shared — and
//! re-reads this registry only when [`LiveStrategyRegistry::generation`]
//! changes.  Per event that is a single relaxed atomic load.  Registration and
//! removal are cold paths and may take the lock.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use domain::strategy_def::StrategyDefinition;
use thiserror::Error;

/// Which door asked for this strategy to run.  Recorded so the UI, the audit
/// trail and a rejection message can name the reason an order exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LiveSource {
    /// `POST /api/strategies/:id/start`.
    Strategy { strategy_id: String },
    /// An armed automation (`POST /api/automations/:id/arm`).
    Automation { automation_id: String },
}

impl LiveSource {
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            LiveSource::Strategy { strategy_id } => strategy_id,
            LiveSource::Automation { automation_id } => automation_id,
        }
    }
}

/// One strategy bound to one instrument for one user, declared live.
#[derive(Clone, Debug)]
pub struct LiveStrategySpec {
    pub user_id: String,
    pub instrument_id: String,
    pub definition: StrategyDefinition,
    pub source: LiveSource,
}

impl LiveStrategySpec {
    /// Identity of a running binding: one strategy per (user, instrument,
    /// strategy id).  The same user may run two different strategies on one
    /// instrument; the same strategy twice on one instrument is a duplicate.
    #[must_use]
    pub fn key(&self) -> LiveKey {
        LiveKey {
            user_id: self.user_id.clone(),
            instrument_id: self.instrument_id.clone(),
            strategy_id: self.definition.strategy_id.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LiveKey {
    pub user_id: String,
    pub instrument_id: String,
    pub strategy_id: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LiveRegistryError {
    #[error("strategy '{strategy_id}' is already running for user '{user_id}' on '{instrument_id}'")]
    AlreadyRunning {
        user_id: String,
        instrument_id: String,
        strategy_id: String,
    },
    #[error("live strategy registry lock poisoned")]
    Poisoned,
}

/// The registry itself.
#[derive(Debug, Default)]
pub struct LiveStrategyRegistry {
    generation: AtomicU64,
    by_instrument: Mutex<HashMap<String, Vec<Arc<LiveStrategySpec>>>>,
}

impl LiveStrategyRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bumps on every change.  Stage 3 compares this to the generation it last
    /// synced at; equal means its instance set is current.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Everything declared live for `instrument_id`.  Cold path — called only
    /// when the generation moved.
    pub fn snapshot_for(&self, instrument_id: &str) -> Vec<Arc<LiveStrategySpec>> {
        match self.by_instrument.lock() {
            Ok(map) => map.get(instrument_id).cloned().unwrap_or_default(),
            Err(_) => {
                tracing::error!(
                    instrument_id,
                    "live strategy registry lock poisoned — treating as empty (fail closed: \
                     no strategy evaluates rather than an unknown set evaluating)"
                );
                Vec::new()
            }
        }
    }

    /// Every instrument that currently has at least one live strategy.
    pub fn instruments(&self) -> Vec<String> {
        match self.by_instrument.lock() {
            Ok(map) => map.keys().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Declare a strategy live.  Rejects a duplicate binding rather than
    /// running the same strategy twice on one instrument.
    pub fn register(&self, spec: LiveStrategySpec) -> Result<(), LiveRegistryError> {
        let key = spec.key();
        let mut map = self
            .by_instrument
            .lock()
            .map_err(|_| LiveRegistryError::Poisoned)?;
        let entry = map.entry(spec.instrument_id.clone()).or_default();
        if entry.iter().any(|s| s.key() == key) {
            return Err(LiveRegistryError::AlreadyRunning {
                user_id: key.user_id,
                instrument_id: key.instrument_id,
                strategy_id: key.strategy_id,
            });
        }
        entry.push(Arc::new(spec));
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Remove one binding.  Returns whether anything was removed.
    pub fn deregister(&self, key: &LiveKey) -> bool {
        let Ok(mut map) = self.by_instrument.lock() else {
            return false;
        };
        let Some(entry) = map.get_mut(&key.instrument_id) else {
            return false;
        };
        let before = entry.len();
        entry.retain(|s| &s.key() != key);
        let removed = entry.len() != before;
        if entry.is_empty() {
            map.remove(&key.instrument_id);
        }
        if removed {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    /// Remove every binding for `(user_id, instrument_id)` regardless of which
    /// strategy it is — what `POST /api/strategies/:id/stop` means today.
    /// Returns how many were removed.
    pub fn deregister_all_for(&self, user_id: &str, instrument_id: &str) -> usize {
        let Ok(mut map) = self.by_instrument.lock() else {
            return 0;
        };
        let Some(entry) = map.get_mut(instrument_id) else {
            return 0;
        };
        let before = entry.len();
        entry.retain(|s| s.user_id != user_id);
        let removed = before - entry.len();
        if entry.is_empty() {
            map.remove(instrument_id);
        }
        if removed > 0 {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    /// Remove every binding declared by one automation, across instruments.
    pub fn deregister_automation(&self, automation_id: &str) -> usize {
        let Ok(mut map) = self.by_instrument.lock() else {
            return 0;
        };
        let mut removed = 0;
        map.retain(|_, entry| {
            let before = entry.len();
            entry.retain(|s| s.source != LiveSource::Automation {
                automation_id: automation_id.to_owned(),
            });
            removed += before - entry.len();
            !entry.is_empty()
        });
        if removed > 0 {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    /// Total live bindings, for diagnostics.
    pub fn len(&self) -> usize {
        match self.by_instrument.lock() {
            Ok(map) => map.values().map(Vec::len).sum(),
            Err(_) => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

static GLOBAL: OnceLock<Arc<LiveStrategyRegistry>> = OnceLock::new();

/// The process-wide registry.
///
/// The API and the in-process pipelines run in one binary and must see one
/// registry.  Threading a handle from `main` is not available to this track —
/// `apps/platform/src/main.rs` belongs to the user — and a second registry
/// would reproduce exactly the bug this module exists to remove: a door that
/// reports success while nothing evaluates.
#[must_use]
pub fn global() -> Arc<LiveStrategyRegistry> {
    GLOBAL
        .get_or_init(|| Arc::new(LiveStrategyRegistry::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(id: &str) -> StrategyDefinition {
        serde_json::from_value(serde_json::json!({
            "strategy_id": id,
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "min_trust_tier": "centralized_exchange",
            "inputs": [],
            "nodes": [],
            "actions": []
        }))
        .expect("definition")
    }

    fn spec(user: &str, instrument: &str, strategy: &str) -> LiveStrategySpec {
        LiveStrategySpec {
            user_id: user.to_owned(),
            instrument_id: instrument.to_owned(),
            definition: def(strategy),
            source: LiveSource::Strategy {
                strategy_id: strategy.to_owned(),
            },
        }
    }

    #[test]
    fn registration_bumps_the_generation_and_is_visible() {
        let r = LiveStrategyRegistry::new();
        let g0 = r.generation();
        r.register(spec("u1", "BTC-USD", "ema")).unwrap();
        assert!(r.generation() > g0, "a change must be observable");
        assert_eq!(r.snapshot_for("BTC-USD").len(), 1);
        assert_eq!(r.snapshot_for("ETH-USD").len(), 0);
    }

    #[test]
    fn duplicate_binding_is_rejected() {
        let r = LiveStrategyRegistry::new();
        r.register(spec("u1", "BTC-USD", "ema")).unwrap();
        let err = r.register(spec("u1", "BTC-USD", "ema")).unwrap_err();
        assert!(matches!(err, LiveRegistryError::AlreadyRunning { .. }));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn two_strategies_on_one_instrument_coexist() {
        let r = LiveStrategyRegistry::new();
        r.register(spec("u1", "BTC-USD", "ema")).unwrap();
        r.register(spec("u1", "BTC-USD", "rsi")).unwrap();
        assert_eq!(r.snapshot_for("BTC-USD").len(), 2);
    }

    #[test]
    fn deregister_removes_and_bumps() {
        let r = LiveStrategyRegistry::new();
        let s = spec("u1", "BTC-USD", "ema");
        let key = s.key();
        r.register(s).unwrap();
        let g = r.generation();
        assert!(r.deregister(&key));
        assert!(r.generation() > g);
        assert!(r.snapshot_for("BTC-USD").is_empty());
        assert!(!r.deregister(&key), "second removal is a no-op");
    }

    #[test]
    fn stop_removes_every_strategy_for_that_user_and_instrument() {
        let r = LiveStrategyRegistry::new();
        r.register(spec("u1", "BTC-USD", "ema")).unwrap();
        r.register(spec("u1", "BTC-USD", "rsi")).unwrap();
        r.register(spec("u2", "BTC-USD", "ema")).unwrap();
        assert_eq!(r.deregister_all_for("u1", "BTC-USD"), 2);
        assert_eq!(r.snapshot_for("BTC-USD").len(), 1, "u2 keeps running");
    }

    #[test]
    fn disarming_an_automation_removes_its_bindings_across_instruments() {
        let r = LiveStrategyRegistry::new();
        for instrument in ["BTC-USD", "ETH-USD"] {
            r.register(LiveStrategySpec {
                user_id: "u1".into(),
                instrument_id: instrument.into(),
                definition: def("ema"),
                source: LiveSource::Automation {
                    automation_id: "a1".into(),
                },
            })
            .unwrap();
        }
        r.register(spec("u1", "BTC-USD", "manual_started")).unwrap();
        assert_eq!(r.deregister_automation("a1"), 2);
        assert_eq!(r.len(), 1, "a hand-started strategy is untouched");
    }
}
