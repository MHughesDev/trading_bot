//! Strategy instance and multi-instance manager.
//!
//! `StrategyInstance` binds a definition to one instrument for one user.
//! `InstanceManager` tracks all active instances and deduplicates pipeline demand.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use demand_manager::DemandRegistry;
use domain::lanes::Lane;
use domain::money::Size;
use domain::order::{OrderIntent, OrderType, Side};
use domain::payloads::bar::Timeframe;
use domain::strategy_def::exits::PositionSide;
use domain::strategy_def::StrategyDefinition;
use domain::InstrumentId;
use rust_decimal::Decimal;
use thiserror::Error;

use crate::bytecode;
use crate::clock::StrategyClock;
use crate::exits::{BarAggregator, ExitTracker, NetEffect, NetPosition};
use crate::ids::NodeId;
use crate::intents::build_intents_for_signals;
use crate::interpreter::evaluate_condition;
use crate::registry::FeatureRegistry;
use crate::world::{WorldEvent, WorldState};

pub use domain::strategy_def::InferenceOutput;

/// Errors produced by the instance manager.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("strategy already running for user '{user_id}' on '{instrument_id}'")]
    AlreadyRunning {
        user_id: String,
        instrument_id: String,
    },
}

/// The order side that flattens a position.
const fn flattening_side(side: PositionSide) -> Side {
    match side {
        PositionSide::Long => Side::Sell,
        PositionSide::Short => Side::Buy,
    }
}

/// The declared exit policy, driven live (ADR-P6-14, SC-3).
///
/// Held by the instance because the instance is the one place live that knows
/// the definition, sees every bar and emits every intent — which is exactly
/// what an exit needs. There is no second evaluation site: the decision is
/// [`ExitTracker`]'s, the same one the backtest simulator drives.
struct LiveExits {
    /// The one shared tracker. Never given an intrabar policy of its own:
    /// `WorstFirst` is the default and the only one usable for evidence.
    tracker: ExitTracker,
    /// Folds the live feed's granularity into the declared bar. The hot path
    /// publishes one event per trade tick, so without this the holding count
    /// would count trades where the strategy declared minutes.
    bars: BarAggregator,
    /// The timeframe adopted at the first bar. Bars of any other timeframe are
    /// ignored for exit purposes: a holding period is counted in one unit.
    timeframe: Timeframe,
    /// The netted position, assumed from this instance's own emitted intents
    /// and corrected by [`WorldEvent::PositionUpdate`].
    position: Option<NetPosition>,
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
    /// Pre-populated model forecast results for `ModelForecast` nodes (v1.0/v1.1 compat).
    /// Maps node_id → whether the forecast condition fired.
    model_forecast_results: HashMap<String, bool>,
    /// Full inference outputs for v1.1 `Inference` / `Decision` nodes.
    /// Written into feature slots before `run_signals()`, and checked for
    /// `Decision` class-to-signal mapping.
    inference_outputs: HashMap<String, InferenceOutput>,
    /// Slot indices for each bound output name, pre-computed at init from
    /// `Inference.outputs`.  Keyed by node ID, value is list of
    /// `(field_name, slot_index)` pairs.
    inference_slot_map: HashMap<String, Vec<(String, u16)>>,
    /// Declared exits, or `None` when the definition declares none that can
    /// close a position on its own. `None` covers two different claims that
    /// happen to behave the same here — a pre-1.3 document, and an explicit
    /// `RiskLayerOnly` — and neither is collapsed into the other.
    exits: Option<LiveExits>,
    /// Signals that were already emitted on the previous event.
    ///
    /// An action fires on a signal's **rising edge**: the signal must clear and
    /// re-fire before it places another order. The simulator has always done
    /// this and the live runtime never did, so a condition that stays true —
    /// `ema_fast > ema_slow` holds for as long as the cross lasts — placed one
    /// order per bar in replay and one per *trade tick* live.
    ///
    /// That is a divergence on its own (SC-3), and it is also what would make
    /// the declared exits below decorative: every re-entry averages into the
    /// position and moves the entry reference with the price, so a stop
    /// measured from entry can never be reached. See ADR-P6-14.
    active_signals: HashSet<String>,
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

        // Pre-register feature slots for every Inference / Combine output binding
        // declared in the definition.  This must happen before WorldState is
        // allocated so the capacity covers all slots.
        let inference_slot_map = Self::register_inference_slots(&definition.nodes, &mut registry);

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
            inference_outputs: HashMap::new(),
            inference_slot_map,
            exits: None,
            active_signals: HashSet::new(),
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

    /// Walk `Inference` and `Combine` nodes in the definition and pre-register
    /// a global feature slot for every output binding.
    ///
    /// Returns a map from node_id → Vec<(field_name, slot_index)> so
    /// `apply_inference_outputs` can write values without any string look-ups
    /// in the hot path.
    fn register_inference_slots(
        nodes: &[domain::strategy_def::nodes::Node],
        registry: &mut FeatureRegistry,
    ) -> HashMap<String, Vec<(String, u16)>> {
        use domain::strategy_def::nodes::NodeKind;
        let mut map: HashMap<String, Vec<(String, u16)>> = HashMap::new();
        for node in nodes {
            let bindings: Vec<&domain::strategy_def::nodes::OutputBinding> = match &node.kind {
                NodeKind::Inference { outputs, .. } => outputs.iter().collect(),
                NodeKind::Combine {
                    inputs: _,
                    op: _,
                    as_,
                } => {
                    // Combine writes a single synthetic slot; we create a
                    // pseudo-binding with field = "__combine__" here.
                    let slot = registry.get_or_assign(as_);
                    map.entry(node.id.clone())
                        .or_default()
                        .push(("__combine__".to_owned(), slot));
                    continue;
                }
                _ => continue,
            };
            let slots: Vec<(String, u16)> = bindings
                .into_iter()
                .map(|b| {
                    let slot = registry.get_or_assign(&b.as_);
                    (b.field.clone(), slot)
                })
                .collect();
            if !slots.is_empty() {
                map.insert(node.id.clone(), slots);
            }
        }
        map
    }

    /// Set pre-fetched model forecast results (for ModelForecast nodes).
    ///
    /// Call this before `process_event()` when the definition contains
    /// `ModelForecast` nodes. The results are consumed by `run_signals()`.
    pub fn set_model_forecast_results(&mut self, results: HashMap<String, bool>) {
        self.model_forecast_results = results;
    }

    /// Set full inference outputs for v1.1 `Inference` / `Decision` nodes.
    ///
    /// Call this before `process_event()` for definitions with these nodes.
    /// The runtime will write bound fields into feature slots and evaluate
    /// `Decision` class maps.
    pub fn set_inference_outputs(&mut self, outputs: HashMap<String, InferenceOutput>) {
        self.inference_outputs = outputs;
    }

    /// Returns true if any node in the definition is a ModelForecast node.
    pub fn has_model_forecasts(&self) -> bool {
        use domain::strategy_def::nodes::NodeKind;
        self.definition
            .nodes
            .iter()
            .any(|n| matches!(n.kind, NodeKind::ModelForecast { .. }))
    }

    /// Returns true if the definition contains any v1.1 AI nodes that produce
    /// outputs (`Inference`, `Decision`, `Sizing`, `LlmInference`, `Combine`).
    pub fn has_inference_nodes(&self) -> bool {
        use domain::strategy_def::nodes::NodeKind;
        self.definition.nodes.iter().any(|n| {
            matches!(
                n.kind,
                NodeKind::Inference { .. }
                    | NodeKind::Decision { .. }
                    | NodeKind::Sizing { .. }
                    | NodeKind::LlmInference { .. }
                    | NodeKind::Combine { .. }
            )
        })
    }

    /// Write the current `inference_outputs` into the feature slot array.
    ///
    /// Called at the start of `process_event` so conditions evaluated in the
    /// same tick can see fresh inference values.  Missing fields write NaN
    /// (consistent with the `Flat` abstain policy).
    fn apply_inference_outputs(&mut self) {
        for (node_id, bindings) in &self.inference_slot_map {
            if let Some(output) = self.inference_outputs.get(node_id) {
                for (field, slot) in bindings {
                    if field == "__combine__" {
                        // Combine nodes are handled separately.
                        continue;
                    }
                    let value = output.get_field(field).unwrap_or(f64::NAN);
                    self.state.set_feature(*slot, value);
                }
            }
            // If no output for this node, do nothing (HoldLast) or NaN was
            // already the slot's initial value (Flat).  The abstain policy
            // write is handled by the gateway before calling set_inference_outputs.
        }
    }

    /// Process a world event.
    ///
    /// 1. Update `WorldState` from the event.
    /// 2. If it is a Feature event, write the value into the slot array (no HashMap, no alloc).
    /// 3. Evaluate the definition node graph via compiled bytecode against the slot array.
    /// 4. Drive the declared exit policy and, if it fired, emit the flattening
    ///    order **before** this event's entries — the same order the simulator
    ///    uses, so that a bar which both stops a position out and signals a new
    ///    one produces the same two orders in the same sequence either side.
    /// 5. Collect and return any order intents — the caller routes them through the risk gate.
    pub fn process_event(&mut self, event: &WorldEvent) -> Vec<OrderIntent> {
        // If this is a feature event, resolve the slot and write it before applying state.
        if let WorldEvent::Feature { feature_value, .. } = event {
            if let Some(slot) = self.registry.get(&feature_value.name) {
                self.state.set_feature(slot, feature_value.value);
            }
        }

        self.state.apply_event(event);

        // Write v1.1 Inference outputs into feature slots before evaluating conditions.
        if !self.inference_slot_map.is_empty() {
            self.apply_inference_outputs();
        }

        // Feature slot array passed directly — no allocation, no string clone.
        let signals = self.run_signals();

        // Resolve Sizing-node fractions for any SizeMode::Model actions.
        let size_overrides = self.size_overrides();

        // ── the declared exit, before this event's entries ───────────────────
        let emitted: Vec<String> = signals.iter().cloned().collect();
        let mut intents = Vec::new();
        if let Some(exit) = self.drive_exits(event, &emitted) {
            intents.push(exit);
        }

        // Entries fire on a signal's rising edge, which is what the simulator
        // does and what event-driven dispatch was always meant to give live.
        // The exit tracker above is passed the *full* emitted set, because an
        // `OnSignal` exit asks whether the signal is present, not whether it
        // just became present — the same split `sim.rs` makes.
        let rising: HashSet<String> = signals.difference(&self.active_signals).cloned().collect();
        self.active_signals = signals;

        let entries = build_intents_for_signals(
            &self.definition.actions,
            &rising,
            &self.instrument_id,
            &self.definition.strategy_id,
            &size_overrides,
        );
        self.note_entry_fills(&entries);
        intents.extend(entries);
        intents
    }

    /// Advance the declared exit policy and return the flattening order if it
    /// fired.
    ///
    /// # Why the live path counts bars the way it does
    ///
    /// The simulator's entries fill at the close of the bar that emitted them,
    /// because the venue processes that bar's ticks before the handler runs.
    /// Live is the same clock arrived at differently: the hot path writes the
    /// tick's price to the paper engine's mark board *before* it publishes the
    /// event, and a market order fills at the mark — so a live entry also
    /// fills at the close of the bar that emitted it. Both paths open the
    /// position at the same reference price, and the bar that opened it is
    /// consumed in both.
    ///
    /// What differs is granularity. The simulator gets one event per bar; the
    /// live feed gets one per trade, and one per trade is not one per bar. So
    /// [`BarAggregator`] reassembles the declared bar, [`ExitTracker::bar_closed`]
    /// advances the holding count once per reassembled bar, and
    /// [`ExitTracker::on_bar_update`] evaluates prices as often as the feed
    /// allows. A stop therefore fires live on the tick that breaches it rather
    /// than at the end of the minute — earlier than replay, never later, which
    /// is the only direction a bound on loss may err in.
    ///
    /// The one consequence worth stating: `ExitTracker::opened_at_bar_close`
    /// is *not* called here, and that is not a divergence. It exists to add the
    /// holding bar the simulator's entry bar never supplies; here the roll out
    /// of the entry bar supplies it, and calling both would count it twice.
    /// `live_and_replay_fire_the_time_exit_on_the_same_bar` is the test that
    /// says so.
    fn drive_exits(&mut self, event: &WorldEvent, signals: &[String]) -> Option<OrderIntent> {
        // A fill or a reconciliation is the venue's word about the position and
        // outranks this instance's assumption about its own orders.
        if let WorldEvent::PositionUpdate { quantity, .. } = event {
            self.reconcile_position(*quantity);
            return None;
        }

        let WorldEvent::Bar {
            timeframe,
            bar,
            available_time,
            ..
        } = event
        else {
            return None;
        };

        // Built on the first bar, because the declared bar is what the feed
        // says it is. `RiskLayerOnly` and a pre-1.3 definition both leave this
        // `None`: this instance closes nothing on its own, and the two are
        // different claims that happen to agree here.
        if self.exits.is_none() {
            let policy = self.definition.exit_policy()?;
            if !policy.is_self_closing() {
                return None;
            }
            self.exits = Some(LiveExits {
                // No `with_intrabar`: `WorstFirst` is the default and the only
                // policy usable for evidence. A per-instance choice between a
                // pessimistic and an optimistic assumption is a knob, and a
                // static invariant fails the build on one.
                tracker: ExitTracker::new(policy.clone()),
                bars: BarAggregator::new(*timeframe),
                timeframe: *timeframe,
                position: None,
            });
        }

        let instrument_id = self.instrument_id.clone();
        let strategy_id = self.definition.strategy_id.clone();
        let exits = self.exits.as_mut()?;
        if *timeframe != exits.timeframe {
            // A holding period is counted in one unit. A second timeframe on
            // the same instance would make `max_holding_bars` mean whichever
            // lane happened to tick.
            return None;
        }

        let observed = exits.bars.observe(
            *available_time,
            bar.high.inner(),
            bar.low.inner(),
            bar.close.inner(),
        );
        if observed.opened_new_bar {
            exits.tracker.bar_closed();
        }

        let triggered = exits.tracker.on_bar_update(&observed.bar, signals)?;

        // The tracker reports; the caller fills. A tracker that closed itself
        // would mark a position flat that the venue still holds.
        let Some(open) = exits.position else {
            // The tracker believes it holds something this instance does not.
            // Re-synchronise rather than send an order for a position that is
            // not there.
            exits.tracker.closed();
            return None;
        };

        exits.position = None;
        exits.tracker.closed();
        tracing::info!(
            instrument_id = %instrument_id,
            strategy_id = %strategy_id,
            rule = %triggered.rule,
            level = %triggered.price,
            bars_held = triggered.bars_held,
            "declared exit fired; flattening"
        );

        // A market order, deliberately. The venue can rest a stop and the
        // simulator cannot, so resting one live would make the deployment
        // behave unlike the backtest that admitted it — see ADR-P6-15.
        Some(OrderIntent::new(
            instrument_id,
            flattening_side(open.side),
            OrderType::Market,
            Size::from_decimal(open.qty),
            None,
            Some(strategy_id),
        ))
    }

    /// Net this event's own entry intents into the tracked position.
    ///
    /// The instance assumes its orders fill at the price the bar closed at,
    /// which is what the simulator assumes and what the paper engine actually
    /// does — a market order fills at the mark, and the mark is this tick's
    /// price. The assumption is corrected by [`WorldEvent::PositionUpdate`]
    /// whenever the venue disagrees.
    fn note_entry_fills(&mut self, entries: &[OrderIntent]) {
        let Some(exits) = self.exits.as_mut() else {
            return;
        };
        for intent in entries {
            let Some(bar) = self.state.bars.get(&exits.timeframe) else {
                continue;
            };
            let fill = bar.close.inner();
            let side = match intent.side {
                Side::Sell => PositionSide::Short,
                Side::Buy => PositionSide::Long,
            };
            match NetPosition::apply(&mut exits.position, side, intent.size.inner(), fill) {
                NetEffect::Reopened { side, entry } => {
                    exits.tracker.opened(side, entry);
                    // The prices this bar traded at before the fill are prices
                    // the position was never exposed to.
                    exits.bars.restart_at(fill);
                }
                NetEffect::Flat => exits.tracker.closed(),
                NetEffect::Unchanged => {}
            }
        }
    }

    /// Take the venue's word for the position size.
    ///
    /// The only correction that can be made honestly is to flat: a rejected or
    /// unfilled entry leaves this instance holding an assumption the venue
    /// never agreed to, and a stop measured from an entry that never happened
    /// is worse than no stop. The opposite direction — the venue holding a
    /// position this instance does not know the entry price of — cannot be
    /// repaired here and is logged rather than guessed at.
    fn reconcile_position(&mut self, quantity: Decimal) {
        let Some(exits) = self.exits.as_mut() else {
            return;
        };
        if quantity.is_zero() {
            if exits.tracker.is_open() {
                tracing::debug!(
                    instrument_id = %self.instrument_id,
                    "venue reports flat; releasing the tracked position"
                );
            }
            exits.position = None;
            exits.tracker.closed();
        } else if !exits.tracker.is_open() {
            tracing::warn!(
                instrument_id = %self.instrument_id,
                quantity = %quantity,
                "venue holds a position this strategy did not open; its declared exits \
                 cannot bound it and the risk layer is the only thing that can"
            );
        }
    }

    /// Bars the declared exit policy has held the current position, or `None`
    /// when it holds nothing. Test and diagnostic surface.
    #[must_use]
    pub fn exit_bars_held(&self) -> Option<u32> {
        self.exits.as_ref().and_then(|e| e.tracker.bars_held())
    }

    /// Collect resolved size fractions from `Sizing` nodes that produced an
    /// `InferenceOutput.size_fraction` this tick.
    ///
    /// Maps `Sizing` node ID → decimal-string fraction, consumed by
    /// `SizeMode::Model` actions.  Nodes that abstained (no output, or an
    /// output with no fraction) are simply absent from the map.
    fn size_overrides(&self) -> HashMap<String, String> {
        use domain::strategy_def::nodes::NodeKind;
        let mut overrides = HashMap::new();
        for node in &self.definition.nodes {
            if matches!(node.kind, NodeKind::Sizing { .. }) {
                if let Some(fraction) = self
                    .inference_outputs
                    .get(&node.id)
                    .and_then(|o| o.size_fraction.clone())
                {
                    overrides.insert(node.id.clone(), fraction);
                }
            }
        }
        overrides
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
                NodeKind::Inference { condition, .. } => {
                    // Only evaluates as a condition when MatchSpec is present.
                    let fired = if let Some(spec) = condition {
                        self.inference_outputs
                            .get(&node.id)
                            .map(|o| {
                                o.direction_matches(&spec.direction)
                                    && o.confidence >= spec.min_confidence
                            })
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    conditions.insert(&node.id, fired);
                }
                _ => {}
            }
        }

        // Collect Decision node signals (class-map → signal name).
        let mut decision_signals: HashSet<String> = HashSet::new();
        for node in &self.definition.nodes {
            if let NodeKind::Decision {
                class_map,
                min_confidence,
                ..
            } = &node.kind
            {
                if let Some(output) = self.inference_outputs.get(&node.id) {
                    if output.confidence >= *min_confidence {
                        let class = output.action_class.as_deref().unwrap_or(&output.direction);
                        if let Some(signal) = class_map.get(class) {
                            decision_signals.insert(signal.clone());
                        }
                    }
                }
            }
        }

        let mut signals: HashSet<String> = self
            .definition
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
            .collect();
        signals.extend(decision_signals);
        signals
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
            exits: None,
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
            parameters: Default::default(),
            constraints: Vec::new(),
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

    /// An action fires once when its signal becomes true, and again only after
    /// the signal has cleared.
    ///
    /// This used to assert that the *bar* after the features produced the
    /// intent, which passed for the wrong reason: the condition became true on
    /// the second feature event, so the intent was emitted there and then
    /// re-emitted on the bar, and on every event after it. That is the
    /// divergence ADR-P6-14 closes — the simulator has always used the rising
    /// edge — and it is also what would make a declared stop unreachable, since
    /// every re-entry averages the entry reference towards the current price.
    #[test]
    fn an_action_fires_on_the_rising_edge_and_not_again_until_it_clears() {
        use chrono::Utc;
        use domain::money::{Price, Size};
        use domain::payloads::bar::{BarPayload, Timeframe};
        use std::str::FromStr;

        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();

        let bar_event = || {
            let bar = BarPayload::new(
                Timeframe::Minutes1,
                Price::from_str("100").unwrap(),
                Price::from_str("110").unwrap(),
                Price::from_str("95").unwrap(),
                Price::from_str("105").unwrap(),
                Size::from_str("500").unwrap(),
                200,
            );
            WorldEvent::Bar {
                instrument_id: "BTC-USDT".into(),
                timeframe: Timeframe::Minutes1,
                bar,
                available_time: Utc::now(),
            }
        };

        // ema_7 alone leaves the condition unevaluable; ema_21 completes it and
        // that is the edge.
        assert!(manager
            .dispatch("BTC-USDT", feature_event("ema_7", 11.0))
            .is_empty());
        let results = manager.dispatch("BTC-USDT", feature_event("ema_21", 10.0));
        assert_eq!(results.len(), 1, "the crossing emits exactly one intent");
        let (uid, intents) = &results[0];
        assert_eq!(uid.as_ref(), "user1");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].instrument_id, "BTC-USDT");

        // The condition is still true on the next two events. A live runtime
        // that re-ordered here would buy on every tick of a cross that lasts.
        assert!(
            manager.dispatch("BTC-USDT", bar_event()).is_empty(),
            "a signal that is merely still true is not a new order"
        );
        assert!(manager
            .dispatch("BTC-USDT", feature_event("ema_7", 12.0))
            .is_empty());

        // Cleared, then re-fired: that is a new edge and a new order.
        assert!(manager
            .dispatch("BTC-USDT", feature_event("ema_7", 9.0))
            .is_empty());
        let again = manager.dispatch("BTC-USDT", feature_event("ema_7", 11.0));
        assert_eq!(again.len(), 1, "a genuine re-cross orders again");
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
            exits: None,
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
            parameters: Default::default(),
            constraints: Vec::new(),
        };

        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let _demand = Arc::new(DemandRegistry::new(Arc::new(NoopPipelineFactory)));
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
