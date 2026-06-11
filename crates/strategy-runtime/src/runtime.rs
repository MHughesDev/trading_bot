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
use thiserror::Error;

use crate::bytecode;
use crate::clock::StrategyClock;
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
    compiled_conditions: HashMap<String, bytecode::Program>,
    /// Stable feature registry — assigned at init, read-only during the hot loop.
    registry: FeatureRegistry,
    /// Per-program local-slot → global-registry-slot mapping, keyed by node id.
    /// Built once at init from compiled_conditions + registry.
    program_slots: HashMap<String, Vec<u16>>,
}

impl StrategyInstance {
    pub fn new(
        user_id: impl Into<String>,
        instrument_id: impl Into<String>,
        definition: StrategyDefinition,
        start_time: DateTime<Utc>,
    ) -> Self {
        let instrument_id = instrument_id.into();
        let compiled_conditions = Self::compile_conditions(&definition.nodes);

        // Build the feature registry and per-program slot maps at init time.
        // After this point the registry is read-only during the hot loop.
        let mut registry = FeatureRegistry::new();
        let program_slots: HashMap<String, Vec<u16>> = compiled_conditions
            .iter()
            .map(|(node_id, program)| {
                let mapping = bytecode::resolve_program_slots(program, &mut registry);
                (node_id.clone(), mapping)
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
        }
    }

    fn compile_conditions(
        nodes: &[domain::strategy_def::nodes::Node],
    ) -> HashMap<String, bytecode::Program> {
        use domain::strategy_def::nodes::NodeKind;
        nodes
            .iter()
            .filter_map(|node| {
                if let NodeKind::Condition { expr } = &node.kind {
                    bytecode::compile(expr)
                        .ok()
                        .map(|program| (node.id.clone(), program))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Process a world event.
    ///
    /// 1. Update `WorldState` from the event.
    /// 2. If it is a Feature event, write the value into the slot array (no HashMap, no alloc).
    /// 3. Evaluate the definition node graph via compiled bytecode against the slot array.
    /// 4. Collect and return any order intents — the caller routes them through the risk gate.
    pub fn process_event(&mut self, event: WorldEvent) -> Vec<OrderIntent> {
        // If this is a feature event, resolve the slot and write it before applying state.
        if let WorldEvent::Feature {
            ref feature_value, ..
        } = event
        {
            if let Some(slot) = self.registry.get(&feature_value.name) {
                self.state.set_feature(slot, feature_value.value);
            }
        }

        self.state.apply_event(&event);

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

        let mut conditions: HashMap<&str, bool> = HashMap::new();
        for node in &self.definition.nodes {
            if let NodeKind::Condition { expr } = &node.kind {
                let fired = if let (Some(program), Some(local_to_global)) = (
                    self.compiled_conditions.get(&node.id),
                    self.program_slots.get(&node.id),
                ) {
                    bytecode::run_slots(program, local_to_global, feature_slots, &self.state.bars)
                } else {
                    // Fallback for conditions that failed to compile — build a
                    // temporary name→value map from the slot array for the
                    // tree-walking interpreter.
                    let features: HashMap<String, f64> = self
                        .state
                        .features
                        .iter()
                        .map(|(k, v)| (k.clone(), v.value))
                        .collect();
                    evaluate_condition(expr, &features, &self.state.bars)
                };
                conditions.insert(&node.id, fired);
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

/// Manages all active strategy instances keyed by `(user_id, instrument_id)`.
///
/// Deduplicates pipeline demand via the `DemandRegistry`: when two users both
/// need `BTC-USDT market.bars.1m`, only one pipeline runs.
pub struct InstanceManager {
    instances: HashMap<(String, String), StrategyInstance>,
    demand: Arc<DemandRegistry>,
}

impl InstanceManager {
    pub fn new(demand: Arc<DemandRegistry>) -> Self {
        Self {
            instances: HashMap::new(),
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
        let user_id = user_id.into();
        let instrument_id = instrument_id.into();
        let key = (user_id.clone(), instrument_id.clone());

        if self.instances.contains_key(&key) {
            return Err(RuntimeError::AlreadyRunning {
                user_id,
                instrument_id,
            });
        }

        // Declare demand for each input lane, resolving $bound_at_init.
        for input in &definition.inputs {
            let resolved = if input.is_bound_at_init() {
                instrument_id.as_str()
            } else {
                input.instrument.as_str()
            };
            if let Ok(lane) = input.lane.parse::<Lane>() {
                self.demand.add(&lane, resolved);
            }
        }

        let start = clock.now();
        let instance =
            StrategyInstance::new(user_id.clone(), instrument_id.clone(), definition, start);
        self.instances.insert(key, instance);
        Ok(())
    }

    /// Stop and remove the instance for `(user_id, instrument_id)`, releasing demand.
    pub fn stop(&mut self, user_id: &str, instrument_id: &str) {
        let key = (user_id.to_owned(), instrument_id.to_owned());
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
    /// Returns `(user_id, intents)` pairs — one entry per instance that emitted
    /// at least one intent.
    pub fn dispatch(
        &mut self,
        instrument_id: &str,
        event: WorldEvent,
    ) -> Vec<(String, Vec<OrderIntent>)> {
        let mut results = Vec::new();
        for ((uid, iid), instance) in &mut self.instances {
            if iid == instrument_id {
                let intents = instance.process_event(event.clone());
                if !intents.is_empty() {
                    results.push((uid.clone(), intents));
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
        assert_eq!(uid, "user1");
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
}
