//! `WorldState`, `WorldContext`, and `WorldEvent`.
//!
//! `WorldState` is the local materialized view maintained per strategy instance.
//! `WorldContext` is the read/write surface strategies call during `on_event`.
//! `WorldEvent` is the union of all event types the runtime dispatches.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use domain::money::{Price, Size};
use domain::order::{OrderIntent, OrderType, Side};
use domain::payloads::bar::{BarPayload, Timeframe};
use domain::payloads::orderbook::OrderBookPayload;
use features::FeatureValue;
use rust_decimal::Decimal;

/// All event types the strategy runtime dispatches to a strategy instance.
#[derive(Clone, Debug)]
pub enum WorldEvent {
    /// A new or revised OHLCV bar.
    Bar {
        instrument_id: String,
        timeframe: Timeframe,
        bar: BarPayload,
        /// Replay sort key — strategies must not use any time source other than this.
        available_time: DateTime<Utc>,
    },
    /// A newly computed feature value for an instrument.
    Feature {
        instrument_id: String,
        feature_value: FeatureValue,
    },
    /// Position update received from a fill or reconciliation event.
    PositionUpdate {
        instrument_id: String,
        /// Signed quantity (positive = long, negative = short).
        quantity: Decimal,
    },
}

impl WorldEvent {
    /// The `available_time` attached to this event, if any.
    pub fn available_time(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Bar { available_time, .. } => Some(*available_time),
            Self::Feature { feature_value, .. } => Some(feature_value.available_time),
            Self::PositionUpdate { .. } => None,
        }
    }
}

/// Per-instance materialized view — updated from bus events before `on_event`.
///
/// Never queried from a database during `on_event`; populated exclusively from
/// events so that live and replay produce identical `WorldState` transitions.
pub struct WorldState {
    instrument_id: String,
    /// Latest bar per timeframe.
    pub bars: HashMap<Timeframe, BarPayload>,
    /// Latest order-book snapshot (optional; not all instruments have L2 data).
    pub orderbook: Option<OrderBookPayload>,
    /// Latest computed feature values by name.
    pub features: HashMap<String, FeatureValue>,
    /// Feature slot array — indexed by u16 slot ID from FeatureRegistry.
    /// f64::NAN = feature not yet received.
    pub feature_slots: Vec<f64>,
    /// Current signed position (positive = long, negative = short).
    pub position: Decimal,
    /// The `available_time` of the most recently dispatched event.
    /// This is what `WorldContext::now()` returns.
    pub current_time: DateTime<Utc>,
}

impl WorldState {
    pub fn new(instrument_id: impl Into<String>, start_time: DateTime<Utc>) -> Self {
        Self {
            instrument_id: instrument_id.into(),
            bars: HashMap::new(),
            orderbook: None,
            features: HashMap::new(),
            feature_slots: Vec::new(),
            position: Decimal::ZERO,
            current_time: start_time,
        }
    }

    /// Create a `WorldState` pre-allocated for `num_slots` feature slots.
    pub fn with_capacity(
        instrument_id: impl Into<String>,
        start_time: DateTime<Utc>,
        num_slots: usize,
    ) -> Self {
        Self {
            instrument_id: instrument_id.into(),
            bars: HashMap::new(),
            orderbook: None,
            features: HashMap::new(),
            feature_slots: vec![f64::NAN; num_slots],
            position: Decimal::ZERO,
            current_time: start_time,
        }
    }

    /// Update state from an incoming event.
    pub fn apply_event(&mut self, event: &WorldEvent) {
        match event {
            WorldEvent::Bar {
                timeframe,
                bar,
                available_time,
                ..
            } => {
                self.bars.insert(*timeframe, bar.clone());
                self.current_time = *available_time;
            }
            WorldEvent::Feature { feature_value, .. } => {
                // Advance current_time so ctx.now() is correct for strategies
                // that fire on feature events before receiving a bar (M-12).
                self.current_time = feature_value.available_time;
                self.features
                    .insert(feature_value.name.clone(), feature_value.clone());
            }
            WorldEvent::PositionUpdate { quantity, .. } => {
                self.position = *quantity;
            }
        }
    }

    /// Write a feature value into the slot array by pre-resolved slot ID.
    /// No-op if the slot index is out of range (guards against races during registry growth).
    pub fn set_feature(&mut self, slot: u16, value: f64) {
        if let Some(v) = self.feature_slots.get_mut(slot as usize) {
            *v = value;
        }
    }

    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }
}

/// The mutable surface strategies call during `on_event`.
///
/// All reads are from the already-updated `WorldState`; no I/O occurs here.
pub struct WorldContext<'a> {
    state: &'a WorldState,
    strategy_id: String,
    instrument_id: String,
    pending_intents: Vec<OrderIntent>,
}

impl<'a> WorldContext<'a> {
    pub fn new(
        state: &'a WorldState,
        strategy_id: impl Into<String>,
        instrument_id: impl Into<String>,
    ) -> Self {
        Self {
            state,
            strategy_id: strategy_id.into(),
            instrument_id: instrument_id.into(),
            pending_intents: Vec::new(),
        }
    }

    /// The `available_time` of the most recently dispatched event.
    ///
    /// Strategies **must** call this instead of any OS clock function.
    pub fn now(&self) -> DateTime<Utc> {
        self.state.current_time
    }

    /// Latest bar for the given timeframe.
    pub fn latest_bar(&self, timeframe: Timeframe) -> Option<&BarPayload> {
        self.state.bars.get(&timeframe)
    }

    /// Current value of a named feature, or `None` if not yet computed.
    pub fn feature(&self, name: &str) -> Option<f64> {
        self.state.features.get(name).map(|f| f.value)
    }

    /// All current feature values — used by the interpreter to build the eval map.
    pub fn features(&self) -> &HashMap<String, FeatureValue> {
        &self.state.features
    }

    /// All current bars — used by the interpreter for `bar('field')` expressions.
    pub fn bars(&self) -> &HashMap<Timeframe, BarPayload> {
        &self.state.bars
    }

    /// Current signed position for the bound instrument.
    pub fn position(&self) -> Decimal {
        self.state.position
    }

    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    /// Submit an order intent.  The intent is collected here and routed through
    /// the risk gate by the runtime after `on_event` returns — the strategy has
    /// no direct broker path.
    pub fn place_order(
        &mut self,
        side: Side,
        order_type: OrderType,
        size: Size,
        limit_price: Option<Price>,
    ) {
        let intent = OrderIntent::new(
            self.instrument_id.as_str(),
            side,
            order_type,
            size,
            limit_price,
            Some(self.strategy_id.clone()),
        );
        self.pending_intents.push(intent);
    }

    /// Drain and return collected order intents.
    pub fn drain_intents(&mut self) -> Vec<OrderIntent> {
        std::mem::take(&mut self.pending_intents)
    }
}

/// The result of a single `Strategy::on_event` call.
#[derive(Debug)]
pub enum StrategyResult {
    Ok,
    Error(String),
}
