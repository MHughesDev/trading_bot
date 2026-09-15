//! The wire between the paper fill stream and the forward record (T-0.1).
//!
//! `crates/reconciliation` existed and nothing called it. This is the piece
//! that calls it: a [`FillObserver`] the execution engine notifies on every
//! newly-applied fill, holding one [`RealisedSeriesBuilder`] and one
//! [`DivergenceMonitor`] per deployed strategy.
//!
//! ## Why intents are registered before they fill
//!
//! A [`FillEvent`] carries the quantity, the price and the fee, and not the
//! side or the price the strategy decided at. Realised slippage measured
//! against the fill is always zero, which is the most flattering possible
//! number and the least informative. So the recorder is told about the intent
//! when it is submitted — [`ForwardRecorder::on_submit`] — and pairs the fill
//! with it by idempotency key.
//!
//! ## What happens when the two disagree
//!
//! The monitor halts the strategy through the risk gate's registry, which the
//! gate consults on the very next order. The halt is in force before
//! [`ForwardRecorder::close_bar`] returns; nothing the caller does with the
//! returned verdict can let an order out in between.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use domain::forward::{ForwardObservation, ObservationSource};
use domain::money::Price;
use domain::order::{OrderIntent, Side};
use execution::fills::{FillEvent, FillObserver};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::divergence::{DivergenceMonitor, DivergenceVerdict};
use crate::forward::{
    AlignedBar, BarClose, DeploymentSpec, ExpectedBar, ForwardError, RealisedSeries,
    RealisedSeriesBuilder,
};

/// How many submitted-but-unfilled intents to remember. Beyond this the oldest
/// are dropped: an intent that has not filled after this many submissions is
/// not going to, and an unbounded map on the fill path is a slow leak in a
/// process that is meant to run for months.
const PENDING_INTENT_CAPACITY: usize = 10_000;

/// The decision context a fill needs and does not carry.
#[derive(Clone, Debug)]
struct PendingIntent {
    side: Side,
    /// The mark the strategy saw when it decided. Realised slippage is measured
    /// against this.
    decision_price: Price,
}

#[derive(Debug)]
struct RecorderState {
    builder: RealisedSeriesBuilder,
    monitor: DivergenceMonitor,
    pending: HashMap<Uuid, PendingIntent>,
    order: Vec<Uuid>,
}

/// One deployment's forward recorder.
#[derive(Debug)]
pub struct ForwardRecorder {
    state: Mutex<RecorderState>,
}

impl ForwardRecorder {
    /// # Errors
    /// An invalid [`DeploymentSpec`] or a non-positive opening balance.
    pub fn new(
        spec: DeploymentSpec,
        opening_balance: Decimal,
        halts: Arc<risk::StrategyHaltRegistry>,
    ) -> Result<Self, ForwardError> {
        let monitor = DivergenceMonitor::new(spec.clone(), halts)?;
        let builder = RealisedSeriesBuilder::new(spec, opening_balance)?;
        Ok(Self {
            state: Mutex::new(RecorderState {
                builder,
                monitor,
                pending: HashMap::new(),
                order: Vec::new(),
            }),
        })
    }

    /// Register an intent this deployment submitted, with the mark it decided at.
    ///
    /// Intents belonging to another strategy are ignored, so one recorder per
    /// deployment can be attached to a shared execution engine without
    /// filtering at the call site.
    pub fn on_submit(&self, intent: &OrderIntent, decision_price: Price) {
        let mut st = self.state.lock().expect("recorder lock");
        if intent.strategy_id.as_deref() != Some(st.builder.spec().strategy_id.as_str()) {
            return;
        }
        if st.order.len() >= PENDING_INTENT_CAPACITY {
            let oldest = st.order.remove(0);
            st.pending.remove(&oldest);
        }
        st.order.push(intent.idempotency_key);
        st.pending.insert(
            intent.idempotency_key,
            PendingIntent {
                side: intent.side,
                decision_price,
            },
        );
    }

    /// Note a broker reject attributable to the model's own output (§12.6).
    pub fn on_model_reject(&self) {
        self.state
            .lock()
            .expect("recorder lock")
            .builder
            .on_model_reject();
    }

    /// The internal signed position, for reconciliation against the broker.
    #[must_use]
    pub fn position(&self) -> Decimal {
        self.state.lock().expect("recorder lock").builder.position()
    }

    /// The realised series so far, in the ledger's own type.
    #[must_use]
    pub fn realised_series(&self) -> RealisedSeries {
        self.state
            .lock()
            .expect("recorder lock")
            .builder
            .realised_series()
    }

    /// Close a bar, judge it against the backtest's expectation, and return the
    /// row plus the verdict.
    ///
    /// The halt, if any, is already in force when this returns.
    ///
    /// # Errors
    /// [`ForwardError`] when the bar is off the deployment's grid, does not
    /// advance, or leaves equity non-positive.
    pub fn close_bar(
        &self,
        bar: chrono::DateTime<chrono::Utc>,
        mark: Price,
        expected: ExpectedBar,
        source: ObservationSource,
    ) -> Result<(ForwardObservation, DivergenceVerdict), ForwardError> {
        let mut st = self.state.lock().expect("recorder lock");
        let close: BarClose = st.builder.close_bar(bar, mark)?;
        let observation = st.builder.observation(&close, expected, source);
        let aligned = AlignedBar {
            bar: close.bar,
            realised: close.realised_return,
            expected: expected.expected_return,
        };
        let verdict = st.monitor.on_bar(&aligned, expected.signal_matched);
        Ok((observation, verdict))
    }

    /// Reconcile the internal position against the broker's for this
    /// deployment. Halts the strategy on disagreement (FM-4).
    pub fn reconcile_position(
        &self,
        instrument_id: &str,
        broker_quantity: Decimal,
        kill_switch: &Arc<risk::KillSwitch>,
    ) -> DivergenceVerdict {
        let mut st = self.state.lock().expect("recorder lock");
        let internal = st.builder.position();
        st.monitor
            .on_position(instrument_id, internal, broker_quantity, kill_switch)
    }
}

impl FillObserver for ForwardRecorder {
    fn on_fill(&self, fill: &FillEvent) {
        let mut st = self.state.lock().expect("recorder lock");
        // A fill for an intent this deployment never submitted belongs to
        // another strategy, or to a manual order. Applying it here would put
        // someone else's trade into this strategy's forward record.
        //
        // The entry is *kept*, not consumed: an order that fills in three
        // partials raises three `Applied` fills against one idempotency key, and
        // consuming the context on the first would leave the other two
        // unattributed — silently shrinking the realised position while the
        // broker's grows, which is the divergence this crate is supposed to
        // detect rather than cause.
        let Some(intent) = st.pending.get(&fill.idempotency_key).cloned() else {
            return;
        };
        st.builder.on_fill(
            intent.side,
            fill.filled_qty,
            fill.fill_price,
            fill.commission,
            intent.decision_price,
        );
    }
}
