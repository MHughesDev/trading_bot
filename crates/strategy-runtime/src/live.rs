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
    #[error(
        "strategy '{strategy_id}' is already running for user '{user_id}' on '{instrument_id}'"
    )]
    AlreadyRunning {
        user_id: String,
        instrument_id: String,
        strategy_id: String,
    },
    #[error("live strategy registry lock poisoned")]
    Poisoned,
    #[error("'{name}' is already deployed for user '{user_id}' on '{instrument_id}' (deployment {deployment_id})")]
    AlreadyDeployed {
        user_id: String,
        instrument_id: String,
        name: String,
        deployment_id: uuid::Uuid,
    },
}

/// The registry itself.
/// A `tbot.strategy/3` deployment declared live (Set U U-4.6).
///
/// Run by the platform's `LiveRunner` — the one strategy engine on decision
/// ticks — and **never** by hot-path stage 3. The two are kept in separate maps
/// on purpose: stage 3 reads only [`LiveStrategyRegistry::snapshot_for`], which
/// cannot return one of these, so there is no code path by which a v3 strategy
/// is evaluated twice.
#[derive(Clone, Debug)]
pub struct V3Deployment {
    /// The deployment record's id (`strategy_deployment.deployment_id`).
    pub deployment_id: uuid::Uuid,
    pub user_id: String,
    /// The instrument the strategy is bound to.
    pub instrument_id: String,
    pub graph: domain::strategy_graph::StrategyGraph,
    /// The graph's content-addressed revision.
    pub revision: String,
    /// What the deployment was started with — the base its realised series is
    /// measured from.
    pub starting_capital: rust_decimal::Decimal,
    /// Set when this deployment is one member of a selection-bound deployment
    /// (U-6.4): the handle its allocator moves its capital and retires it
    /// through. `None` for a deployment bound to one instrument by hand.
    pub sleeve: Option<Arc<SleeveHandle>>,
}

/// What a selection-bound deployment's allocator shares with one member's
/// runner (Set U U-6.4).
///
/// The allocator decides; the runner acts. A leaver is not stopped from the
/// outside: the allocator writes how it leaves, and the runner — the only thing
/// that owns this member's orders — carries it out through its own gate and
/// journal on its next bar.
#[derive(Debug)]
pub struct SleeveHandle {
    /// The selection-bound deployment this member belongs to.
    pub parent: uuid::Uuid,
    capital: Mutex<rust_decimal::Decimal>,
    retire: Mutex<Option<domain::strategy_graph::deployment::OnRemoval>>,
    selection: Mutex<Option<MemberSelection>>,
}

/// What the selection said about a member at its latest rebalance (U-6.6).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemberSelection {
    /// The selection strategy's name.
    pub name: String,
    pub revision: String,
    pub attributes: std::collections::BTreeMap<String, rust_decimal::Decimal>,
}

impl SleeveHandle {
    #[must_use]
    pub fn new(parent: uuid::Uuid, capital: rust_decimal::Decimal) -> Self {
        Self {
            parent,
            capital: Mutex::new(capital),
            retire: Mutex::new(None),
            selection: Mutex::new(None),
        }
    }

    /// This member's share of the deployment's capital — what its strategy
    /// sizes against.
    #[must_use]
    pub fn capital(&self) -> rust_decimal::Decimal {
        self.capital
            .lock()
            .map_or(rust_decimal::Decimal::ZERO, |c| *c)
    }

    pub fn set_capital(&self, capital: rust_decimal::Decimal) {
        if let Ok(mut c) = self.capital.lock() {
            *c = capital;
        }
    }

    /// What the latest selection said about this member.
    #[must_use]
    pub fn selection(&self) -> Option<MemberSelection> {
        self.selection.lock().ok().and_then(|s| s.clone())
    }

    pub fn set_selection(&self, selection: MemberSelection) {
        if let Ok(mut s) = self.selection.lock() {
            *s = Some(selection);
        }
    }

    /// The member left the selection: how it leaves.
    pub fn retire(&self, how: domain::strategy_graph::deployment::OnRemoval) {
        if let Ok(mut r) = self.retire.lock() {
            *r = Some(how);
        }
    }

    /// Picked again before its exit finished: it stays a member.
    pub fn reinstate(&self) {
        if let Ok(mut r) = self.retire.lock() {
            *r = None;
        }
    }

    /// `Some` once the member has left the selection.
    #[must_use]
    pub fn retiring(&self) -> Option<domain::strategy_graph::deployment::OnRemoval> {
        self.retire.lock().ok().and_then(|r| *r)
    }
}

/// A trading strategy deployed on whatever a selection picks (Set U U-6.4).
///
/// Its members run as ordinary [`V3Deployment`]s carrying a [`SleeveHandle`];
/// this is what the platform's allocator for it reads.
#[derive(Clone, Debug)]
pub struct SelectionBinding {
    /// The parent deployment record.
    pub deployment_id: uuid::Uuid,
    pub user_id: String,
    /// The trading strategy each member runs.
    pub trading: domain::strategy_graph::StrategyGraph,
    pub trading_revision: String,
    /// The selection's revision.
    pub selection: String,
    pub top_n: u32,
    pub rebalance: domain::strategy_graph::deployment::Rebalance,
    pub on_removal: domain::strategy_graph::deployment::OnRemoval,
    pub weighting: domain::strategy_graph::deployment::Weighting,
    /// Weights are fractions of this.
    pub capital: rust_decimal::Decimal,
}

#[derive(Debug, Default)]
pub struct LiveStrategyRegistry {
    generation: AtomicU64,
    by_instrument: Mutex<HashMap<String, Vec<Arc<LiveStrategySpec>>>>,
    v3_generation: AtomicU64,
    v3: Mutex<std::collections::BTreeMap<uuid::Uuid, Arc<V3Deployment>>>,
    selection_generation: AtomicU64,
    selections: Mutex<std::collections::BTreeMap<uuid::Uuid, Arc<SelectionBinding>>>,
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
            entry.retain(|s| {
                s.source
                    != LiveSource::Automation {
                        automation_id: automation_id.to_owned(),
                    }
            });
            removed += before - entry.len();
            !entry.is_empty()
        });
        if removed > 0 {
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    // ── v3 deployments (Set U U-4.6) ─────────────────────────────────────────

    /// Moves whenever a v3 deployment is added or removed. The live runners'
    /// supervisor re-reads [`Self::v3_snapshot`] only when it changes.
    #[must_use]
    pub fn v3_generation(&self) -> u64 {
        self.v3_generation.load(Ordering::Acquire)
    }

    /// Declare a v3 deployment. One strategy name per (user, instrument).
    ///
    /// # Errors
    /// The same graph already deployed for this user on this instrument.
    pub fn register_v3(&self, d: V3Deployment) -> Result<(), LiveRegistryError> {
        let mut map = self.v3.lock().map_err(|_| LiveRegistryError::Poisoned)?;
        if let Some(existing) = map.values().find(|e| {
            e.user_id == d.user_id
                && e.instrument_id == d.instrument_id
                && e.graph.name == d.graph.name
        }) {
            return Err(LiveRegistryError::AlreadyDeployed {
                user_id: d.user_id,
                instrument_id: d.instrument_id,
                name: d.graph.name.clone(),
                deployment_id: existing.deployment_id,
            });
        }
        map.insert(d.deployment_id, Arc::new(d));
        self.v3_generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Withdraw a v3 deployment. Its runner stops at its next decision tick.
    pub fn deregister_v3(&self, deployment_id: uuid::Uuid) -> bool {
        let Ok(mut map) = self.v3.lock() else {
            return false;
        };
        let removed = map.remove(&deployment_id).is_some();
        if removed {
            self.v3_generation.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    /// Whether a v3 deployment is still declared — what a runner checks before
    /// every decision.
    #[must_use]
    pub fn is_v3_declared(&self, deployment_id: uuid::Uuid) -> bool {
        self.v3
            .lock()
            .is_ok_and(|map| map.contains_key(&deployment_id))
    }

    // ── selection-bound deployments (Set U U-6.4) ────────────────────────────

    /// Moves whenever a selection-bound deployment is added or removed.
    #[must_use]
    pub fn selection_generation(&self) -> u64 {
        self.selection_generation.load(Ordering::Acquire)
    }

    /// Declare a selection-bound deployment. One per (user, trading strategy,
    /// selection).
    ///
    /// # Errors
    /// The same trading strategy already deployed on the same selection.
    pub fn register_selection(&self, b: SelectionBinding) -> Result<(), LiveRegistryError> {
        let mut map = self
            .selections
            .lock()
            .map_err(|_| LiveRegistryError::Poisoned)?;
        if let Some(existing) = map.values().find(|e| {
            e.user_id == b.user_id && e.selection == b.selection && e.trading.name == b.trading.name
        }) {
            return Err(LiveRegistryError::AlreadyDeployed {
                user_id: b.user_id,
                instrument_id: format!("selection:{}", b.selection),
                name: b.trading.name.clone(),
                deployment_id: existing.deployment_id,
            });
        }
        map.insert(b.deployment_id, Arc::new(b));
        self.selection_generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Withdraw a selection-bound deployment: its allocator stops its members
    /// and stops.
    pub fn deregister_selection(&self, deployment_id: uuid::Uuid) -> bool {
        let Ok(mut map) = self.selections.lock() else {
            return false;
        };
        let removed = map.remove(&deployment_id).is_some();
        if removed {
            self.selection_generation.fetch_add(1, Ordering::AcqRel);
        }
        removed
    }

    #[must_use]
    pub fn is_selection_declared(&self, deployment_id: uuid::Uuid) -> bool {
        self.selections
            .lock()
            .is_ok_and(|map| map.contains_key(&deployment_id))
    }

    /// Every declared selection-bound deployment, in id order.
    #[must_use]
    pub fn selection_snapshot(&self) -> Vec<Arc<SelectionBinding>> {
        self.selections
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Every declared v3 deployment, in id order. Empty on a poisoned lock —
    /// fail closed: nothing runs rather than an unknown set.
    #[must_use]
    pub fn v3_snapshot(&self) -> Vec<Arc<V3Deployment>> {
        self.v3
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
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

    fn v3(user: &str, instrument: &str) -> V3Deployment {
        let graph: domain::strategy_graph::StrategyGraph =
            serde_json::from_value(serde_json::json!({
              "format": "tbot.strategy/3", "kind": "trading", "name": "ema_v3",
              "asset_class": "crypto_spot_cex", "timeframe": "1m",
              "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
              "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
            }))
            .unwrap();
        V3Deployment {
            deployment_id: uuid::Uuid::new_v4(),
            user_id: user.into(),
            instrument_id: instrument.into(),
            revision: graph.revision().unwrap().as_str().to_string(),
            graph,
            starting_capital: rust_decimal::Decimal::from(10_000),
            sleeve: None,
        }
    }

    /// Set U U-4.6: a v3 deployment is never also evaluated by stage 3. Stage 3
    /// reads `snapshot_for` and nothing else; a v3 deployment cannot appear in
    /// it, and declaring one does not even move the generation stage 3 watches.
    #[test]
    fn a_v3_deployment_is_invisible_to_stage_3() {
        let r = LiveStrategyRegistry::new();
        let g = r.generation();
        let d = v3("u1", "BTC-USD");
        let id = d.deployment_id;
        r.register_v3(d).unwrap();
        assert!(r.snapshot_for("BTC-USD").is_empty());
        assert_eq!(r.generation(), g, "stage 3 is not even woken");
        assert!(r.is_empty(), "the v1 set is untouched");
        assert_eq!(r.v3_snapshot().len(), 1);
        assert!(r.is_v3_declared(id));
        assert!(r.deregister_v3(id));
        assert!(!r.is_v3_declared(id));
        assert!(!r.deregister_v3(id), "withdrawing twice is a no-op");
    }

    #[test]
    fn the_same_graph_twice_on_one_instrument_is_refused() {
        let r = LiveStrategyRegistry::new();
        r.register_v3(v3("u1", "BTC-USD")).unwrap();
        assert!(matches!(
            r.register_v3(v3("u1", "BTC-USD")),
            Err(LiveRegistryError::AlreadyDeployed { .. })
        ));
        r.register_v3(v3("u2", "BTC-USD")).unwrap();
        r.register_v3(v3("u1", "ETH-USD")).unwrap();
        assert_eq!(r.v3_snapshot().len(), 3);
    }
}
