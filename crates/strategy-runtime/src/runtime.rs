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
use crate::exits::{bar_seconds, BarAggregator, ExitTracker, NetEffect, NetPosition, Triggered};
use crate::history::{HistoryRegisters, Unknown, Unresolved, Verdict};
use crate::ids::NodeId;
use crate::intents::{build_intents_for_signals, SizedIntents, SizingContext};
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
    /// The last exit this instance fired, kept so that the rule and the level
    /// it rested at stay observable after the flattening order has gone.
    ///
    /// Live has no `conservative_exit_fill`: the simulator needs it because it
    /// cannot rest an order and must reconcile the level with the close it
    /// actually got, whereas a live fill is simply what happened. Keeping the
    /// level is what makes the gap between the two measurable rather than
    /// assumed (ADR-P6-15).
    last_exit: Option<Triggered>,
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
    /// Previous-bar values for every `crossed_above`, `crossed_below`,
    /// `rising` and `falling` site in this definition's conditions.
    ///
    /// One file for the whole instance, so every condition rolls on the same
    /// bar and two crosses in one definition cannot drift apart.
    history: HistoryRegisters,
    /// NodeId → the base of that program's run of history sites.
    /// Built once at init, in node order so two runs of the same definition
    /// assign the same sites.
    program_site_base: HashMap<NodeId, usize>,
    /// Seconds per bar of the timeframe adopted at the first bar event, which
    /// is what the history registers bucket by.
    ///
    /// The live feed publishes one `WorldEvent::Bar` per *trade tick* — a
    /// degenerate bar whose open, high, low and close are all the trade price
    /// — so counting events would make a cross a comparison between two
    /// trades. Bucketing by the declared timeframe is the same correction
    /// [`BarAggregator`] makes for the holding count, and for the same reason:
    /// a bar is a unit of the strategy's clock, not of the feed's.
    ///
    /// `None` until the first bar arrives. Before then there is no bar clock
    /// and every history site reads unknown, which is the honest answer.
    history_bucket_secs: Option<i64>,
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
    /// Account equity + mark for equity-relative sizing (`percent_of_balance`).
    /// Refreshed by the caller before `process_event` **only** when
    /// [`Self::needs_sizing_context`] is true, so a fixed-size strategy costs
    /// no account read on the hot path.
    sizing: Option<SizingContext>,
    /// Whether any action in this definition uses an equity-relative size mode.
    /// Computed once at init.
    needs_sizing_context: bool,
    /// Signals that were already firing at the previous event.
    ///
    /// Orders are emitted on a signal's **rising edge** only: a signal must
    /// clear and re-fire before it places another order.  `crates/backtest`'s
    /// simulator has always done this, and its comment claimed the rule "is the
    /// crossover semantics live strategies get from event-driven dispatch" —
    /// which was not true.  The live runtime emitted on *every* event where the
    /// condition held, so `ema_7 > ema_21` would have placed an order per tick
    /// for as long as the cross lasted, against one order per crossing in
    /// replay.  Nobody noticed because hot-path stage 3 produced no intents at
    /// all; wiring it is what made the divergence observable.
    ///
    /// The two must agree or ADR-0008 does not hold and Gate 15 cannot
    /// reconcile a forward fill stream against the backtest that justified it.
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

        // Build the feature registry, the per-program slot maps and the
        // history register file at init time.  After this point the registry is
        // read-only during the hot loop.
        //
        // The walk is over `definition.nodes` in order and **not** over
        // `compiled_conditions`, which is a `HashMap`: iterating that handed
        // out feature slots — and would hand out history sites — in whatever
        // order the hasher produced, so two runs of the same definition could
        // number them differently.  Replay determinism (ADR-0008) needs the
        // numbering to be a function of the definition alone.
        let mut registry = FeatureRegistry::new();
        let mut program_slots: HashMap<NodeId, Vec<u16>> = HashMap::new();
        let mut history = HistoryRegisters::new();
        let mut program_site_base: HashMap<NodeId, usize> = HashMap::new();
        for idx in 0..definition.nodes.len() {
            let node_id = NodeId(idx as u32);
            let Some(program) = compiled_conditions.get(&node_id) else {
                continue;
            };
            program_slots.insert(
                node_id,
                bytecode::resolve_program_slots(program, &mut registry),
            );
            program_site_base.insert(node_id, program.install_sites(&mut history));
        }

        // Pre-register feature slots for every Inference / Combine output binding
        // declared in the definition.  This must happen before WorldState is
        // allocated so the capacity covers all slots.
        let inference_slot_map = Self::register_inference_slots(&definition.nodes, &mut registry);

        // Pre-allocate the slot array with NAN sentinels.
        let state = WorldState::with_capacity(instrument_id.clone(), start_time, registry.len());

        let needs_sizing_context = definition.actions.iter().any(|a| {
            let domain::strategy_def::actions::ActionKind::PlaceOrder { order } = &a.kind;
            matches!(
                order.size_mode,
                domain::strategy_def::actions::SizeMode::PercentOfBalance
                    | domain::strategy_def::actions::SizeMode::RiskUnit
            )
        });

        Self {
            user_id: user_id.into(),
            instrument_id,
            definition,
            state,
            compiled_conditions,
            registry,
            program_slots,
            history,
            program_site_base,
            history_bucket_secs: None,
            _node_id_to_str: node_id_to_str,
            model_forecast_results: HashMap::new(),
            inference_outputs: HashMap::new(),
            inference_slot_map,
            sizing: None,
            needs_sizing_context,
            exits: None,
            active_signals: HashSet::new(),
        }
    }

    /// True when this definition declares an equity-relative size mode and
    /// therefore needs [`Self::set_sizing_context`] before it can size an order.
    #[must_use]
    pub fn needs_sizing_context(&self) -> bool {
        self.needs_sizing_context
    }

    /// Supply account equity and the current mark for equity-relative sizing.
    ///
    /// Live and replay both call this with the same figure — total account
    /// equity (locked decision 10) — so a `percent_of_balance` strategy sizes
    /// identically on both sides (ADR-0008).
    pub fn set_sizing_context(&mut self, sizing: Option<SizingContext>) {
        self.sizing = sizing;
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
    ///    uses, so a bar that both stops a position out and signals a new one
    ///    produces the same two orders in the same sequence either side.
    /// 5. Collect and return any order intents — the caller routes them through the risk gate.
    ///
    /// Returns [`SizedIntents`]: the intents produced **and** any action that
    /// asked for a size and could not get one.  The caller must surface the
    /// errors; dropping them is how `percent_of_balance` came to mean
    /// "silently no order".
    pub fn process_event(&mut self, event: &WorldEvent) -> SizedIntents {
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

        // Tell the history registers which bar this event belongs to, before
        // anything reads them.
        self.advance_history_clock(event);

        // Feature slot array passed directly — no allocation, no string clone.
        let (fired, unresolved) = self.run_signals();

        // ── the declared exit, before this event's entries ───────────────────
        //
        // Ahead of every early return below, and deliberately: a stop must fire
        // on a bar where *nothing* signalled. Behind the "nothing fired and
        // nothing was firing" fast path, a position would only ever be checked
        // against its stop on bars that happened to produce a signal, which is
        // the same as having no stop.
        //
        // The tracker is passed the **full** emitted set, not the rising edge:
        // an `OnSignal` exit asks whether the signal is present, not whether it
        // just became present — the same split `sim.rs` makes.
        let emitted: Vec<String> = fired.iter().cloned().collect();
        let mut exit_intents = Vec::new();
        if let Some(exit) = self.drive_exits(event, &emitted) {
            exit_intents.push(exit);
        }

        // Nothing firing and nothing was firing: the overwhelmingly common
        // case, and it allocates nothing beyond what `run_signals` already did.
        // Safe to take with unresolved signals present — carrying an unknown
        // signal's previous state forward is a no-op when there is no previous
        // state to carry.
        if fired.is_empty() && self.active_signals.is_empty() {
            return SizedIntents {
                intents: exit_intents,
                errors: Vec::new(),
                unresolved,
            };
        }

        // Rising edge only — see `active_signals`.  A signal that stays true
        // places one order, not one per event.
        let rising: HashSet<String> = fired.difference(&self.active_signals).cloned().collect();

        // A signal nobody could evaluate keeps whatever state it had.  Letting
        // it drop out of `active_signals` would make the next event where it
        // *is* true look like a fresh rising edge, so one unreadable bar in the
        // middle of a held signal would place a second order into a position
        // that is already open.
        let mut next_active = fired;
        for u in &unresolved {
            if self.active_signals.contains(&u.signal) {
                next_active.insert(u.signal.clone());
            }
        }
        self.active_signals = next_active;

        if rising.is_empty() {
            return SizedIntents {
                intents: exit_intents,
                errors: Vec::new(),
                unresolved,
            };
        }

        // Resolve Sizing-node fractions for any SizeMode::Model actions.
        let size_overrides = self.size_overrides();

        let mut out = build_intents_for_signals(
            &self.definition.actions,
            &rising,
            &self.instrument_id,
            &self.definition.strategy_id,
            &size_overrides,
            self.sizing.as_ref(),
        );
        self.note_entry_fills(&out.intents);
        // The exit leads. A bar that both stops a position out and signals a new
        // one must produce flatten-then-enter in that order, or the two paths
        // net differently from the same bar.
        out.intents.splice(0..0, exit_intents);
        out.unresolved = unresolved;
        out
    }

    /// Tell the history registers which bar this event belongs to.
    ///
    /// The bar clock is the *declared timeframe's* bucket of the event's
    /// `available_time`, adopted from the first bar seen and fixed thereafter.
    /// Bars of any other timeframe do not move it, matching what [`LiveExits`]
    /// does with the holding count: a strategy's history is counted in one
    /// unit or it is not counted at all.
    ///
    /// Only bar events move the clock. Feature values for a bar arrive after
    /// it, so they update the bar that is already open — which is what lets a
    /// cross be evaluated from the features of the bar it belongs to rather
    /// than the bar before.
    fn advance_history_clock(&mut self, event: &WorldEvent) {
        let WorldEvent::Bar {
            timeframe,
            available_time,
            ..
        } = event
        else {
            return;
        };
        let secs = bar_seconds(*timeframe);
        let adopted = *self.history_bucket_secs.get_or_insert(secs);
        if secs != adopted {
            return;
        }
        self.history
            .open_bar(available_time.timestamp().div_euclid(adopted));
    }

    /// Bars of history this instance needs before every condition can be
    /// answered, **on top of** its indicators' own warm-up.
    ///
    /// Zero for a definition written against the frozen v1 grammar; one for a
    /// `crossed_above`; `n` for a `rising(x, n)`.
    #[must_use]
    pub fn history_warmup_bars(&self) -> u16 {
        self.history.warmup_bars()
    }

    /// Whether every history site has accumulated the bars it needs.
    ///
    /// False while the instance is still warming — during which conditions
    /// reading history report unresolved rather than false.
    #[must_use]
    pub fn history_is_warm(&self) -> bool {
        (0..self.history.site_count()).all(|s| self.history.readiness(s).is_ok())
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
                last_exit: None,
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
        exits.last_exit = Some(triggered.clone());
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

    /// The last declared exit this instance fired: which rule, the level it
    /// rested at, and how many bars the position was held.
    ///
    /// The level is what the policy said, not what the venue paid. The gap
    /// between the two is live slippage, and it is only measurable because
    /// this is kept (ADR-P6-15).
    #[must_use]
    pub fn last_declared_exit(&self) -> Option<&Triggered> {
        self.exits.as_ref().and_then(|e| e.last_exit.as_ref())
    }

    /// Signals currently held true, for diagnostics and tests.
    #[must_use]
    pub fn active_signals(&self) -> &HashSet<String> {
        &self.active_signals
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

    /// Evaluate every condition and map the results onto signals.
    ///
    /// Returns the signals that fired **and** the signals whose conditions
    /// could not be evaluated. The second list is the point: an empty first
    /// list used to mean both "the strategy declined to trade" and "nothing
    /// could ask it", and a backtest that cannot tell those apart under-counts
    /// entries at the start of every window it slices.
    fn run_signals(&mut self) -> (HashSet<String>, Vec<Unresolved>) {
        use domain::strategy_def::nodes::NodeKind;

        // NodeId → verdict: keyed by the node's string id so Signal nodes can
        // look it up.
        let mut conditions: HashMap<&str, Verdict> = HashMap::new();
        for (idx, node) in self.definition.nodes.iter().enumerate() {
            match &node.kind {
                NodeKind::Condition { expr } => {
                    let node_id = NodeId(idx as u32);
                    let verdict = if let (Some(program), Some(local_to_global), Some(site_base)) = (
                        self.compiled_conditions.get(&node_id),
                        self.program_slots.get(&node_id),
                        self.program_site_base.get(&node_id).copied(),
                    ) {
                        bytecode::run_slots(
                            program,
                            local_to_global,
                            &self.state.feature_slots,
                            &self.state.bars,
                            &mut self.history,
                            site_base,
                        )
                    } else {
                        // Fallback for conditions that failed to compile — build a
                        // temporary name→value map from the slot array for the
                        // tree-walking interpreter.
                        //
                        // This path cannot answer a history-bearing function:
                        // the interpreter refuses those by name. It is reached
                        // only when `bytecode::compile` failed, which for such
                        // an expression means it was malformed anyway.
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
                        Verdict::from_bool(evaluate_condition(expr, &features, &self.state.bars))
                    };
                    conditions.insert(&node.id, verdict);
                }
                NodeKind::ModelForecast { .. } => {
                    // A forecast that has not been fetched is not a forecast of
                    // "no": it is the absence of one.
                    let verdict = self
                        .model_forecast_results
                        .get(&node.id)
                        .copied()
                        .map_or(Verdict::Unknown(Unknown::FeatureNotReady), Verdict::from_bool);
                    conditions.insert(&node.id, verdict);
                }
                NodeKind::Inference { condition, .. } => {
                    // Only evaluates as a condition when MatchSpec is present.
                    let verdict = if let Some(spec) = condition {
                        self.inference_outputs.get(&node.id).map_or(
                            Verdict::Unknown(Unknown::FeatureNotReady),
                            |o| {
                                Verdict::from_bool(
                                    o.direction_matches(&spec.direction)
                                        && o.confidence >= spec.min_confidence,
                                )
                            },
                        )
                    } else {
                        // No match spec: this node is not a condition at all.
                        Verdict::False
                    };
                    conditions.insert(&node.id, verdict);
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

        let mut signals: HashSet<String> = HashSet::new();
        let mut unresolved: Vec<Unresolved> = Vec::new();
        for node in &self.definition.nodes {
            let NodeKind::Signal { when, emit } = &node.kind else {
                continue;
            };
            // A `when` naming a node that is not a condition is a broken
            // definition, which is a thing to report rather than a quiet "no".
            let verdict = conditions
                .get(when.as_str())
                .copied()
                .unwrap_or(Verdict::Unknown(Unknown::Malformed));
            match verdict {
                Verdict::True => {
                    signals.insert(emit.clone());
                }
                Verdict::False => {}
                Verdict::Unknown(reason) => unresolved.push(Unresolved {
                    signal: emit.clone(),
                    node: when.clone(),
                    reason,
                }),
            }
        }
        signals.extend(decision_signals);
        // A signal that some other node emitted for certain is settled, even if
        // one condition feeding a same-named signal could not be read.
        unresolved.retain(|u| !signals.contains(&u.signal));
        (signals, unresolved)
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
    /// Returns `(user_id, outcome)` pairs — one entry per instance that emitted
    /// at least one intent or one sizing error.
    ///
    /// **Not the live evaluation path.** Live evaluation happens in exactly one
    /// place — stage 3 of the in-process pipeline, driven by
    /// [`crate::live::LiveStrategyRegistry`] (ADR-0038).  This method drives
    /// instances held by this manager, which is what replay harnesses and tests
    /// want; adding a production caller here would recreate the two-evaluator
    /// split that let a started strategy report success and evaluate nothing.
    pub fn dispatch(
        &mut self,
        instrument_id: &str,
        event: WorldEvent,
    ) -> Vec<(Arc<str>, SizedIntents)> {
        let iid = domain::intern_instrument(instrument_id);
        let mut results = Vec::new();
        if let Some(keys) = self.by_instrument.get(&iid) {
            // Arc::clone is an atomic ref-count increment — no heap allocation (#19).
            let keys: Vec<InstanceKey> = keys.clone();
            for key in &keys {
                if let Some(instance) = self.instances.get_mut(key) {
                    let outcome = instance.process_event(&event);
                    if !outcome.is_empty() {
                        results.push((Arc::clone(&key.0), outcome));
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
        // This test used to assert `results.is_empty()` — true before
        // `SizedIntents::is_empty()` was defined to count `unresolved` too.
        // It no longer holds, and should not: a bar arriving before either
        // feature has a value is exactly the case `unresolved` exists to
        // report (`intents.rs`'s own doc comment names it, "a cross on the
        // first bar of a window"). Dropping that silently would be an absent
        // measurement reported as a zero, on the same day this session found
        // and fixed that mistake in three other crates. So the assertion is
        // now the honest one: no *order* intent was placed, and the manager
        // surfaced exactly one unresolved signal rather than nothing at all.
        assert_eq!(results.len(), 1, "the unresolved condition must be surfaced");
        let (_, outcome) = &results[0];
        assert!(
            outcome.intents.is_empty(),
            "no order intent expected without feature values"
        );
        assert!(
            !outcome.unresolved.is_empty(),
            "the missing features should be reported as unresolved, not silently dropped"
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
    /// Orders fire on the rising edge of a signal, not on every event where
    /// the signal is true.  This is the rule `crates/backtest`'s simulator has
    /// always applied; before track 16 the live runtime did not, and the
    /// difference was invisible only because hot-path stage 3 produced no
    /// intents at all (ADR-0008 / Gate 15).
    fn intent_emitted_on_the_rising_edge_only() {
        let clock = Arc::new(WallClock) as Arc<dyn StrategyClock>;
        let mut manager = InstanceManager::new(registry());
        manager
            .initialize("user1", "BTC-USDT", ema_cross_def(), &clock)
            .unwrap();

        use chrono::Utc;
        use domain::money::{Price, Size};
        use domain::payloads::bar::{BarPayload, Timeframe};
        use std::str::FromStr;

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

        let mut intents = 0;
        // Returns rather than accumulating: a closure that captures `intents`
        // mutably holds that borrow for its whole lifetime, and every
        // `assert_eq!(intents, _)` below is a read of it.
        fn count(results: Vec<(Arc<str>, SizedIntents)>) -> usize {
            let mut n = 0;
            for (uid, outcome) in &results {
                assert_eq!(uid.as_ref(), "user1");
                assert!(outcome.errors.is_empty(), "no sizing errors expected");
                for intent in &outcome.intents {
                    assert_eq!(intent.instrument_id, "BTC-USDT");
                }
                n += outcome.intents.len();
            }
            n
        }

        // ema_7 arrives first; ema_21 is still unknown, so nothing fires.
        intents += count(manager.dispatch("BTC-USDT", feature_event("ema_7", 11.0)));
        assert_eq!(intents, 0);

        // ema_21 arrives and the cross is true — this is the rising edge.
        intents += count(manager.dispatch("BTC-USDT", feature_event("ema_21", 10.0)));
        assert_eq!(intents, 1, "the crossing places exactly one order");

        // The condition stays true across three more events.  A strategy that
        // placed an order on each of them would accumulate position on every
        // tick — and would not resemble the backtest, which has always fired
        // on the rising edge only.
        intents += count(manager.dispatch("BTC-USDT", bar_event()));
        intents += count(manager.dispatch("BTC-USDT", feature_event("ema_7", 12.0)));
        intents += count(manager.dispatch("BTC-USDT", bar_event()));
        assert_eq!(intents, 1, "a signal that stays true must not re-fire");

        // It clears …
        intents += count(manager.dispatch("BTC-USDT", feature_event("ema_7", 9.0)));
        assert_eq!(intents, 1);

        // … and crosses again: a second order, and only one.
        intents += count(manager.dispatch("BTC-USDT", feature_event("ema_7", 13.0)));
        assert_eq!(intents, 2, "a fresh crossing places a second order");
        intents += count(manager.dispatch("BTC-USDT", bar_event()));
        assert_eq!(intents, 2);
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
        // Same correction as `no_intent_without_features`: `SizedIntents::is_empty()`
        // counts `unresolved` too, and a ModelForecast node with no result yet is
        // exactly that case (`Verdict::Unknown(Unknown::FeatureNotReady)` in
        // `run_signals`) — reported, not dropped. The order-intent half of the
        // claim still holds and is what this test is actually named for.
        assert!(
            intents.intents.is_empty(),
            "should not fire without forecast results"
        );
        assert!(
            !intents.unresolved.is_empty(),
            "an unfetched forecast should be reported as unresolved, not silently dropped"
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
            intents2.intents.len(),
            1,
            "should fire when forecast result is true"
        );
    }
}
