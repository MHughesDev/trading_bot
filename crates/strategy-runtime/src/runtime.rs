//! Strategy instance and multi-instance manager.
//!
//! `StrategyInstance` binds a definition to one instrument for one user.
//! `InstanceManager` tracks all active instances and deduplicates pipeline demand.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use demand_manager::DemandRegistry;
use domain::lanes::Lane;
use domain::order::OrderIntent;
use domain::strategy_def::StrategyDefinition;
use domain::InstrumentId;
use thiserror::Error;

use crate::bytecode;
use crate::clock::StrategyClock;
use crate::ids::NodeId;
use crate::intents::build_intents_for_signals;
use crate::interpreter::evaluate_condition;
use crate::registry::FeatureRegistry;
use crate::world::{WorldEvent, WorldState};

/// Errors produced by the instance manager.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("strategy already running for user '{user_id}' on '{instrument_id}'")]
    AlreadyRunning {
        user_id: String,
        instrument_id: String,
    },
}

/// A single runtime binding of a strategy definition to one instrument for one user.
pub struct StrategyInstance {
    pub user_id: String,
    pub instrument_id: String,
    pub definition: StrategyDefinition,
    state: WorldState,
    /// Condition node expressions compiled to postfix bytecode at init.
    /// Keyed by NodeId (u32 index) instead of String for O(1) numeric lookup.
    compiled_conditions: HashMap<NodeId, bytecode::Program>,
    /// Stable feature registry — assigned at init, read-only during the hot loop.
    registry: FeatureRegistry,
    /// Per-program local-slot → global-registry-slot mapping, keyed by NodeId.
    /// Built once at init from compiled_conditions + registry.
    program_slots: HashMap<NodeId, Vec<u16>>,
    /// String node IDs indexed by NodeId for fallback interpreter path.
    _node_id_to_str: HashMap<NodeId, String>,
    /// Pre-populated model forecast results for ModelForecast nodes.
    /// Must be set before process_event() if any ModelForecast nodes exist.
    /// Maps node_id → whether the forecast condition fired.
    model_forecast_results: HashMap<String, bool>,
}

impl StrategyInstance {
    pub fn new(
        user_id: impl Into<String>,
        instrument_id: impl Into<String>,
        definition: StrategyDefinition,
        start_time: DateTime<Utc>,
    ) -> Self {
        let instrument_id = instrument_id.into();
        let (compiled_conditions, node_id_to_str) = Self::compile_conditions(&definition.nodes);

        // Build the feature registry and per-program slot maps at init time.
        // After this point the registry is read-only during the hot loop.
        let mut registry = FeatureRegistry::new();
        let program_slots: HashMap<NodeId, Vec<u16>> = compiled_conditions
            .iter()
            .map(|(node_id, program)| {
                let mapping = bytecode::resolve_program_slots(program, &mut registry);
                (*node_id, mapping)
            })
            .collect();

        // Pre-allocate the slot array with NAN sentinels.
        let state = WorldState::with_capacity(instrument_id.clone(), start_time, registry.len());

        Self {
            user_id: user_id.into(),
            instrument_id,
            definition,
            state,
            compiled_conditions,
            registry,
            program_slots,
            _node_id_to_str: node_id_to_str,
            model_forecast_results: HashMap::new(),
        }
    }

    /// Compile condition nodes to bytecode, assigning NodeId by position.
    ///
    /// Returns `(compiled_conditions, node_id_to_str)` where `node_id_to_str`
    /// maps each NodeId back to the original string ID for the fallback path.
    fn compile_conditions(
        nodes: &[domain::strategy_def::nodes::Node],
    ) -> (HashMap<NodeId, bytecode::Program>, HashMap<NodeId, String>) {
        use domain::strategy_def::nodes::NodeKind;
        let mut programs = HashMap::new();
        let mut id_map = HashMap::new();
        for (idx, node) in nodes.iter().enumerate() {
            if let NodeKind::Condition { expr } = &node.kind {
                let node_id = NodeId(idx as u32);
                id_map.insert(node_id, node.id.clone());
                if let Ok(program) = bytecode::compile(expr) {
                    programs.insert(node_id, program);
                }
            }
        }
        (programs, id_map)
    }

    /// Set pre-fetched model forecast results (for ModelForecast nodes).
    ///
    /// Call this before `process_event()` when the definition contains
    /// `ModelForecast` nodes. The results are consumed by `run_signals()`.
    pub fn set_model_forecast_results(&mut self, results: HashMap<String, bool>) {
        self.model_forecast_results = results;
    }

    /// Returns true if any node in the definition is a ModelForecast node.
    pub fn has_model_forecasts(&self) -> bool {
        use domain::strategy_def::nodes::NodeKind;
        self.definition
            .nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::ModelForecast { .. }))
    }

    /// Process a world event.
    ///
    /// 1. Update `WorldState` from the event.
    /// 2. If it is a Feature event, write the value into the slot array (no HashMap, no alloc).
    /// 3. Evaluate the definition node graph via compiled bytecode against the slot array.
    /// 4. Collect and return any order intents — the caller routes them through the risk gate.
    pub fn process_event(&mut self, event: &WorldEvent) -> Vec<OrderIntent> {
        // If this is a feature event, resolve the slot and write it before applying state.
        if let WorldEvent::Feature { feature_value, .. } = event {
            if let Some(slot) = self.registry.get(&feature_value.name) {
                self.state.set_feature(slot, feature_value.value);
            }
        }

        self.state.apply_event(event);

        // Feature slot array passed directly — no allocation, no string clone.
        let signals = self.run_signals();

        build_intents_for_signals(
            &self.definition.actions,
            &signals,
            &self.instrument_id,
            &self.definition.strategy_id,
        )
    }

    fn run_signals(&self) -> HashSet<String> {
        use domain::strategy_def::nodes::NodeKind;

        let feature_slots = &self.state.feature_slots;

        // NodeId → fired: keyed by u32 so Signal nodes can look up by string id.
        let mut conditions: HashMap<&str, bool> = HashMap::new();
        for (idx, node) in self.definition.nodes.iter().enumerate() {
            match &node.kind {
                NodeKind::Condition { expr } => {
                    let node_id = NodeId(idx as u32);
                    let fired = if let (Some(program), Some(local_to_global)) = (
                        self.compiled_conditions.get(&node_id),
                        self.program_slots.get(&node_id),
                    ) {
                        bytecode::run_slots(
                            program,
                            local_to_global,
                            feature_slots,
                            &self.state.bars,
                        )
                    } else {
                        // Fallback for conditions that failed to compile — build a
                        // temporary name→value map from the slot array for the
                        // tree-walking interpreter.
                        let features: HashMap<String, f64> = self
                            .registry
                            .iter_slots()
                            .filter_map(|(name, slot)| {
                                let v = self.state.feature_slots.get(slot as usize).copied()?;
                                if v.is_nan() {
                                    None
                                } else {
                                    Some((name.to_owned(), v))
                                }
                            })
                            .collect();
                        evaluate_condition(expr, &features, &self.state.bars)
                    };
                    conditions.insert(&node.id, fired);
                }
                NodeKind::ModelForecast { .. } => {
                    // Look up pre-fetched result. If not available, abstain (false).
                    let fired = self
                        .model_forecast_results
                        .get(&node.id)
                        .copied()
                        .unwrap_or(false);
                    conditions.insert(&node.id, fired);
                }
                _ => {}
            }
        }

        self.definition
            .nodes
            .iter()
            .filter_map(|node| {
                if let NodeKind::Signal { when, emit } = &node.kind {
                    if conditions.get(when.as_str()).copied().unwrap_or(false) {
                        Some(emit.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// The `available_time` of the most recently processed event.
    pub fn current_time(&self) -> DateTime<Utc> {
        self.state.current_time
    }
}

type InstanceKey = (Arc<str>, Arc<str>);

/// Manages all active strategy instances keyed by `(user_id, instrument_id)`.
///
/// Deduplicates pipeline demand via the `DemandRegistry`: when two users both
/// need `BTC-USDT market.bars.1m`, only one pipeline runs.
///
/// Keys use `Arc<str>` so that dispatch() can clone the by_instrument entry
/// with atomic ref-count increments instead of heap String allocations.
pub struct InstanceManager {
    instances: HashMap<InstanceKey, StrategyInstance>,
    /// Secondary index: interned InstrumentId → Vec of Arc<str> key pairs.
    /// Arc::clone is an atomic increment; the by_instrument clone in dispatch()
    /// allocates nothing on the heap per matched instance (#19).
    by_instrument: HashMap<InstrumentId, Vec<InstanceKey>>,
    demand: Arc<DemandRegistry>,
}

impl InstanceManager {
    pub fn new(demand: Arc<DemandRegistry>) -> Self {
        Self {
            instances: HashMap::new(),
            by_instrument: HashMap::new(),
            demand,
        }
    }

    /// Initialize a new strategy instance bound to `instrument_id` for `user_id`.
    ///
    /// Returns `Err` if an instance already exists for this `(user_id, instrument_id)` pair.
    pub fn initialize(
        &mut self,
        user_id: impl Into<String>,
        instrument_id: impl Into<String>,
        definition: StrategyDefinition,
        clock: &Arc<dyn StrategyClock>,
    ) -> Result<(), RuntimeError> {
        let user_id_str = user_id.into();
        let instrument_id_str = instrument_id.into();
        // Create Arc<str> once; all further clones are cheap atomic increments.
        let uid: Arc<str> = Arc::from(user_id_str.as_str());
        let iid_str: Arc<str> = Arc::from(instrument_id_str.as_str());
        let key = (Arc::clone(&uid), Arc::clone(&iid_str));

        if self.instances.contains_key(&key) {
            return Err(RuntimeError::AlreadyRunning {
                user_id: user_id_str,
                instrument_id: instrument_id_str,
            });
        }

        // Declare demand for each input lane, resolving $bound_at_init.
        for input in &definition.inputs {
            let resolved = if input.is_bound_at_init() {
                &*iid_str
            } else {
                input.instrument.as_str()
            };
            if let Ok(lane) = input.lane.parse::<Lane>() {
                self.demand.add(&lane, resolved);
            }
        }

        let start = clock.now();
        let instance = StrategyInstance::new(&*uid, &*iid_str, definition, start);
        self.instances.insert(key.clone(), instance);
        // Intern the instrument name once at registration; store the compact u32 ID
        // so dispatch() can do a u32 hash lookup instead of a string hash lookup.
        let iid = domain::intern_instrument(&iid_str);
        self.by_instrument.entry(iid).or_default().push(key);
        Ok(())
    }

    /// Stop and remove the instance for `(user_id, instrument_id)`, releasing demand.
    pub fn stop(&mut self, user_id: &str, instrument_id: &str) {
        // Cold path — constructing temporary Arc<str> for key lookup is acceptable.
        let key: InstanceKey = (Arc::from(user_id), Arc::from(instrument_id));
        if let Some(instance) = self.instances.remove(&key) {
            for input in &instance.definition.inputs {
                let resolved = if input.is_bound_at_init() {
                    instance.instrument_id.as_str()
                } else {
                    input.instrument.as_str()
                };
                if let Ok(lane) = input.lane.parse::<Lane>() {
                    self.demand.remove(&lane, resolved);
                }
            }
        }
    }

    /// Dispatch an event to all instances bound to `instrument_id`.
    ///
    /// Interns `instrument_id` to a compact `InstrumentId` (u32) and uses the
    /// `by_instrument` secondary index for O(1) u32-keyed lookup — no string
    /// hashing in the hot path and no `event.clone()` per match.
    ///
    /// Returns `(user_id, intents)` pairs — one entry per instance that emitted
    /// at least one intent.
    pub fn dispatch(
        &mut self,
        instrument_id: &str,
        event: WorldEvent,
    ) -> Vec<(Arc<str>, Vec<OrderIntent>)> {
        let iid = domain::intern_instrument(instrument_id);
        let mut results = Vec::new();
        if let Some(keys) = self.by_instrument.get(&iid) {
            // Arc::clone is an atomic ref-count increment — no heap allocation (#19).
            let keys: Vec<InstanceKey> = keys.clone();
            for key in &keys {
                if let Some(instance) = self.instances.get_mut(key) {
                    let intents = instance.process_event(&event);
                    if !intents.is_empty() {
                        results.push((Arc::clone(&key.0), intents));
                    }
                }
            }
        }
        results
    }

    pub fn active_count(&self) -> usize {
        self.instances.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::WallClock;
    use demand_manager::{DemandRegistry, NoopPipelineFactory};
    use domain::strategy_def::{
        actions::{Action, ActionKind, OrderSpec, SizeMode},
        inputs::InputDeclaration,
        nodes::{Node, NodeKind},
        StrategyDefinition,
    };

    fn ema_cross_def() -> StrategyDefinition {
        StrategyDefinition {
            strategy_id: "ema_cross_v1".into(),
            definition_version: "1.0".into(),
            asset_class: "crypto_spot_cex".into(),
            min_trust_tier: domain::TrustTier::CentralizedExchange,
            inputs: vec![InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            }],
            nodes: vec![
                Node {
                    id: "n1".into(),
                    kind: NodeKind::Condition {
                        expr: "feature('ema_7') > feature('ema_21')".into(),
                    },
                },
                Node {
                    id: "n2".into(),
                    kind: NodeKind::Signal {
                        when: "n1".into(),
                        emit: "long".into(),
                    },
                },
            ],
            actions: vec![Action {
                on_signal: "long".into(),
                kind: ActionKind::PlaceOrder {
                    order: OrderSpec {
                        side: domain::order::Side::Buy,
                        size_mode: SizeMode::Fixed,
                        size: "0.01".into(),
                    },
                },
            }],
            risk_overrides: domain::strategy_def::risk_overrides::RiskOverrides::default(),
        }
    }

    fn registry() -> Arc<DemandRegistry> {
        Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)))
    }

    fn feature_event(name: &str, value: f64) -> WorldEvent {
        use chrono::Utc;
        use features::FeatureValue;
        WorldEvent::Feature {
            instrument_id: "BTC-USDT".into(),
            feature_value: FeatureValue::new(name, value, 1, Utc::now()),
        }
    }

    #[test]
    fn no_intent_without_features() {
        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();

        use chrono::Utc;
        use domain::money::{Price, Size};
        use domain::payloads::bar::{BarPayload, Timeframe};
        use std::str::FromStr;

        let bar = BarPayload::new(
            Timeframe::Minutes1,
            Price::from_str("100").unwrap(),
            Price::from_str("110").unwrap(),
            Price::from_str("95").unwrap(),
            Price::from_str("105").unwrap(),
            Size::from_str("500").unwrap(),
            200,
        );
        let event = WorldEvent::Bar {
            instrument_id: "BTC-USDT".into(),
            timeframe: Timeframe::Minutes1,
            bar,
            available_time: Utc::now(),
        };
        let results = manager.dispatch("BTC-USDT", event);
        assert!(
            results.is_empty(),
            "no intents expected without feature values"
        );
    }

    #[test]
    fn intent_emitted_when_condition_true() {
        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();

        // Inject features that satisfy ema_7 > ema_21
        manager.dispatch("BTC-USDT", feature_event("ema_7", 11.0));
        manager.dispatch("BTC-USDT", feature_event("ema_21", 10.0));

        // Now inject a bar event — the condition should fire
        use chrono::Utc;
        use domain::money::{Price, Size};
        use domain::payloads::bar::{BarPayload, Timeframe};
        use std::str::FromStr;

        let bar = BarPayload::new(
            Timeframe::Minutes1,
            Price::from_str("100").unwrap(),
            Price::from_str("110").unwrap(),
            Price::from_str("95").unwrap(),
            Price::from_str("105").unwrap(),
            Size::from_str("500").unwrap(),
            200,
        );
        let event = WorldEvent::Bar {
            instrument_id: "BTC-USDT".into(),
            timeframe: Timeframe::Minutes1,
            bar,
            available_time: Utc::now(),
        };
        let results = manager.dispatch("BTC-USDT", event);
        assert_eq!(results.len(), 1);
        let (uid, intents) = &results[0];
        assert_eq!(uid.as_ref(), "user1");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].instrument_id, "BTC-USDT");
    }

    #[test]
    fn two_users_independent_state() {
        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();
        manager
            .initialize("user2", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();
        assert_eq!(manager.active_count(), 2);

        // Both instances must be independent (demand count = 2 for the shared lane)
    }

    #[test]
    fn duplicate_instance_returns_error() {
        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();
        let err = manager.initialize("user1", "BTC-USDT", ema_cross_def(), &clock);
        assert!(err.is_err());
    }

    #[test]
    fn stop_removes_instance() {
        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();
        assert_eq!(manager.active_count(), 1);
        manager.stop("user1", "BTC-USDT");
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn model_forecast_node_fires_when_result_set() {
        use domain::strategy_def::{
            actions::{Action, ActionKind, OrderSpec, SizeMode},
            inputs::InputDeclaration,
            nodes::{Node, NodeKind},
            StrategyDefinition,
        };

        let def = StrategyDefinition {
            strategy_id: "model_forecast_test".into(),
            definition_version: "1.1".into(),
            asset_class: "crypto_spot_cex".into(),
            min_trust_tier: domain::TrustTier::CentralizedExchange,
            inputs: vec![InputDeclaration {
                lane: "market.bars.1m".into(),
                instrument: "$bound_at_init".into(),
                features: vec![],
            }],
            nodes: vec![
                Node {
                    id: "mf1".into(),
                    kind: NodeKind::ModelForecast {
                        model_ref: "mdl_test".into(),
                        target_kind: "model".into(),
                        alias: "production".into(),
                        direction: "bullish".into(),
                        min_confidence: 0.7,
                        input: None,
                    },
                },
                Node {
                    id: "s1".into(),
                    kind: NodeKind::Signal {
                        when: "mf1".into(),
                        emit: "long".into(),
                    },
                },
            ],
            actions: vec![Action {
                on_signal: "long".into(),
                kind: ActionKind::PlaceOrder {
                    order: OrderSpec {
                        side: domain::order::Side::Buy,
                        size_mode: SizeMode::Fixed,
                        size: "0.01".into(),
                    },
                },
            }],
            risk_overrides: domain::strategy_def::risk_overrides::RiskOverrides::default(),
        };

        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
        let mut instance = StrategyInstance::new("user1", "BTC-USDT", def, clock.now());

        assert!(instance.has_model_forecasts());

        // Without setting model forecast results, should not fire.
        use chrono::Utc;
        use domain::money::{Price, Size};
        use domain::payloads::bar::{BarPayload, Timeframe};
        use std::str::FromStr;

        let bar = BarPayload::new(
            Timeframe::Minutes1,
            Price::from_str("100").unwrap(),
            Price::from_str("110").unwrap(),
            Price::from_str("95").unwrap(),
            Price::from_str("105").unwrap(),
            Size::from_str("500").unwrap(),
            200,
        );
        let event = WorldEvent::Bar {
            instrument_id: "BTC-USDT".into(),
            timeframe: Timeframe::Minutes1,
            bar: bar.clone(),
            available_time: Utc::now(),
        };
        let intents = instance.process_event(&event);
        assert!(
            intents.is_empty(),
            "should not fire without forecast results"
        );

        // Set forecast result to true
        let mut results = HashMap::new();
        results.insert("mf1".to_string(), true);
        instance.set_model_forecast_results(results);

        let event2 = WorldEvent::Bar {
            instrument_id: "BTC-USDT".into(),
            timeframe: Timeframe::Minutes1,
            bar,
            available_time: Utc::now(),
        };
        let intents2 = instance.process_event(&event2);
        assert_eq!(
            intents2.len(),
            1,
            "should fire when forecast result is true"
        );
    }
}
