//! The wire between the paper fill stream and the forward record (T-0.1).
//!
//! `crates/reconciliation` existed and nothing called it. This is the piece
//! that calls it: [`ForwardRecorders`] is the [`FillObserver`] the execution
//! engine notifies on every newly-applied fill, holding one
//! [`ForwardRecorder`] — one [`RealisedSeriesBuilder`] and one
//! [`DivergenceMonitor`] — per deployed strategy.
//!
//! ## The boot sequence
//!
//! This is what a binary that may place orders has to do, and the whole of it:
//!
//! ```no_run
//! // Once, at startup, before the first order can be submitted.
//! let recorders = reconciliation::ForwardRecorders::install();
//!
//! // Nothing is listening is a startup failure, not a quiet zero.
//! execution::fills::require_recording().expect("the fill stream is recorded");
//!
//! // Later, when a strategy is deployed to paper (T-0.4):
//! // let recorder = recorders.deploy(spec, opening_balance, halts)?;
//! # let _ = recorders;
//! ```
//!
//! `tests/the_boot_sequence.rs` runs exactly that, so the instruction cannot go
//! stale the way `ExecutionEngine::with_fill_observer` did — a method three
//! plan files told tracks to call and that never existed.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

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

    /// Take the deployment's opening mark. Must precede the first
    /// [`Self::close_bar`].
    ///
    /// # Errors
    /// [`ForwardError::AlreadyOpened`], [`ForwardError::MisalignedBar`], or
    /// [`ForwardError::DegenerateEquity`].
    pub fn open(
        &self,
        bar: chrono::DateTime<chrono::Utc>,
        mark: Price,
    ) -> Result<(), ForwardError> {
        self.state
            .lock()
            .expect("recorder lock")
            .builder
            .open(bar, mark)
    }

    /// Note a broker reject attributable to the model's own output (§12.6).
    pub fn on_model_reject(&self) {
        self.state
            .lock()
            .expect("recorder lock")
            .builder
            .on_model_reject();
    }

    /// Whether this recorder's deployment names a paper venue.
    ///
    /// Always true by construction — [`crate::forward::PaperVenue`] has no live
    /// variant — and exposed so the entry-point test in
    /// `tests/paper_only_entry_points.rs` can assert it at the boundary rather
    /// than only at the type.
    #[must_use]
    pub fn spec_is_paper(&self) -> bool {
        self.state
            .lock()
            .expect("recorder lock")
            .builder
            .spec()
            .venue
            .is_paper()
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

impl ForwardRecorder {
    /// Apply a fill if it belongs to this deployment. Returns whether it did.
    ///
    /// The return value is the whole reason this is not a [`FillObserver`]
    /// method: `on_fill` returns `()`, so a recorder that declined a fill and a
    /// recorder that took one are the same event to the caller, and
    /// [`ForwardRecorders`] would have no way to tell a fill that reached a
    /// deployment from a fill that reached nobody.
    ///
    /// A fill for an intent this deployment never submitted belongs to another
    /// strategy, or to a manual order. Applying it here would put someone
    /// else's trade into this strategy's forward record.
    ///
    /// The pending entry is *kept*, not consumed: an order that fills in three
    /// partials raises three `Applied` fills against one idempotency key, and
    /// consuming the context on the first would leave the other two
    /// unattributed — silently shrinking the realised position while the
    /// broker's grows, which is the divergence this crate is supposed to
    /// detect rather than cause.
    pub fn apply_fill(&self, fill: &FillEvent) -> bool {
        let mut st = self.state.lock().expect("recorder lock");
        let Some(intent) = st.pending.get(&fill.idempotency_key).cloned() else {
            return false;
        };
        st.builder.on_fill(
            intent.side,
            fill.filled_qty,
            fill.fill_price,
            fill.commission,
            intent.decision_price,
        );
        true
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The wire itself
// ─────────────────────────────────────────────────────────────────────────────

/// The process's one subscription to the fill stream, and the only route by
/// which a [`ForwardRecorder`] ever sees a fill.
///
/// ## Why this type exists at all
///
/// `ForwardRecorder` used to implement [`FillObserver`] directly, which made
/// "is this deployment recording?" a question answered by whether some caller
/// had remembered to call `execution::fills::register_observer` with it. Nobody
/// did: T-0.1 shipped with the recorder constructed only in tests. So the
/// implementation moved here, and a recorder acquires fills **only** by being
/// deployed into a registry that has already subscribed — [`Self::install`]
/// registers before it returns, so a deployed recorder is an attached recorder
/// as a matter of construction rather than of discipline.
///
/// ## What it counts, and why an absent count is not a zero
///
/// `execution::fills` counts fills that reached *no observer*. That is not the
/// same question as "did they reach **reconciliation**": another observer being
/// registered makes that counter read zero while the forward record stays
/// empty. So this type keeps its own, and records the stream's published count
/// at the moment it attached — every fill before that watermark is one the
/// forward record can never be given, because the fill stream has no replay.
#[derive(Debug)]
pub struct ForwardRecorders {
    deployments: RwLock<Vec<(Uuid, Arc<ForwardRecorder>)>>,
    /// `execution::fills`' published count at the instant this attached.
    fills_before_install: u64,
    fills_seen: AtomicU64,
    fills_attributed: AtomicU64,
    fills_unattributed: AtomicU64,
    recorder_failures: AtomicU64,
}

static INSTALLED: OnceLock<Arc<ForwardRecorders>> = OnceLock::new();

impl ForwardRecorders {
    /// Attach reconciliation to the process-wide fill stream, once.
    ///
    /// Idempotent: the second call returns the first registry rather than
    /// subscribing twice. Two subscriptions would deliver every fill to every
    /// deployment twice, and a double-counted forward observation inflates
    /// exactly the number that is hardest to acquire.
    ///
    /// Call it at startup, **before the first order can be submitted**. Calling
    /// it later is not an error and is not silent: the fills published in the
    /// meantime are counted as
    /// [`ForwardRecordingEvidence::fills_before_install`] and the verdict is
    /// [`ForwardRecordingVerdict::AttachedLate`] for the life of the process.
    #[must_use]
    pub fn install() -> Arc<Self> {
        Arc::clone(INSTALLED.get_or_init(|| {
            let registry = Arc::new(Self {
                deployments: RwLock::new(Vec::new()),
                fills_before_install: execution::fills::evidence().fills_published,
                fills_seen: AtomicU64::new(0),
                fills_attributed: AtomicU64::new(0),
                fills_unattributed: AtomicU64::new(0),
                recorder_failures: AtomicU64::new(0),
            });
            execution::fills::register_observer(Arc::clone(&registry) as Arc<dyn FillObserver>);
            registry
        }))
    }

    /// The installed registry, or `None` when nothing has attached.
    ///
    /// `None` is not "no fills". It is "this process is not on the fill
    /// stream", which is the state an empty `mlops.forward_observation` cannot
    /// be read as evidence from.
    #[must_use]
    pub fn installed() -> Option<Arc<Self>> {
        INSTALLED.get().map(Arc::clone)
    }

    /// Build a deployment's recorder and attach it in one step.
    ///
    /// There is no way to attach a recorder built elsewhere, and no way to
    /// build one here without attaching it.
    ///
    /// # Errors
    /// An invalid [`DeploymentSpec`] or a non-positive opening balance.
    pub fn deploy(
        &self,
        spec: DeploymentSpec,
        opening_balance: Decimal,
        halts: Arc<risk::StrategyHaltRegistry>,
    ) -> Result<Arc<ForwardRecorder>, ForwardError> {
        let deployment_id = spec.deployment_id;
        let recorder = Arc::new(ForwardRecorder::new(spec, opening_balance, halts)?);
        self.deployments
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((deployment_id, Arc::clone(&recorder)));
        Ok(recorder)
    }

    /// The recorder for a deployment, if it is deployed here.
    #[must_use]
    pub fn recorder(&self, deployment_id: Uuid) -> Option<Arc<ForwardRecorder>> {
        self.deployments
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(id, _)| *id == deployment_id)
            .map(|(_, r)| Arc::clone(r))
    }

    /// Tell every deployment about a submitted intent and the mark it decided
    /// at. Each ignores intents that are not its own.
    pub fn on_submit(&self, intent: &OrderIntent, decision_price: Price) {
        for (_, recorder) in self.snapshot() {
            recorder.on_submit(intent, decision_price);
        }
    }

    /// How many deployments are recording here.
    #[must_use]
    pub fn deployment_count(&self) -> usize {
        self.deployments
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    fn snapshot(&self) -> Vec<(Uuid, Arc<ForwardRecorder>)> {
        self.deployments
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// What this registry can honestly say about its own forward record.
    #[must_use]
    pub fn evidence(&self) -> ForwardRecordingEvidence {
        ForwardRecordingEvidence {
            installed: true,
            deployments: self.deployment_count(),
            fills_before_install: self.fills_before_install,
            fills_seen: self.fills_seen.load(Ordering::Relaxed),
            fills_attributed: self.fills_attributed.load(Ordering::Relaxed),
            fills_unattributed: self.fills_unattributed.load(Ordering::Relaxed),
            recorder_failures: self.recorder_failures.load(Ordering::Relaxed),
        }
    }

    /// The same report for a process that may never have installed anything.
    #[must_use]
    pub fn process_evidence() -> ForwardRecordingEvidence {
        Self::installed().map_or(
            ForwardRecordingEvidence {
                installed: false,
                deployments: 0,
                // Every fill this process has taken is one reconciliation never
                // saw, and the count says how many rather than reading zero.
                fills_before_install: execution::fills::evidence().fills_published,
                fills_seen: 0,
                fills_attributed: 0,
                fills_unattributed: 0,
                recorder_failures: 0,
            },
            |r| r.evidence(),
        )
    }

    /// Startup guard for a process that is about to run a deployment forward.
    ///
    /// Refuses rather than returning a reassuring zero. The failure this exists
    /// to prevent is spending a month of calendar time — the one input Set T
    /// cannot manufacture — and finding an empty table nobody can tell apart
    /// from a strategy that never traded.
    ///
    /// # Errors
    /// [`ForwardRecordingGap`], one variant per way the record can be untrue.
    pub fn require_recording() -> Result<ForwardRecordingEvidence, ForwardRecordingGap> {
        let ev = Self::process_evidence();
        match ev.verdict() {
            ForwardRecordingVerdict::NotInstalled => Err(ForwardRecordingGap::NotInstalled),
            ForwardRecordingVerdict::AttachedLate { fills } => {
                Err(ForwardRecordingGap::AttachedLate { fills })
            }
            ForwardRecordingVerdict::NoDeployment { .. } => Err(ForwardRecordingGap::NoDeployment),
            ForwardRecordingVerdict::Recording { .. } => Ok(ev),
        }
    }
}

impl FillObserver for ForwardRecorders {
    fn on_fill(&self, fill: &FillEvent) {
        self.fills_seen.fetch_add(1, Ordering::Relaxed);
        let deployments = self.snapshot();

        let mut attributed = false;
        for (deployment_id, recorder) in &deployments {
            // One deployment's poisoned recorder lock must not stop the fill
            // reaching the others: `execution::fills` isolates *observers*, and
            // this whole registry is one observer, so without this the first
            // deployment to panic would silently end the forward record of
            // every deployment behind it in the list.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                recorder.apply_fill(fill)
            })) {
                Ok(true) => attributed = true,
                Ok(false) => {}
                Err(_) => {
                    self.recorder_failures.fetch_add(1, Ordering::Relaxed);
                    tracing::error!(
                        %deployment_id,
                        idempotency_key = %fill.idempotency_key,
                        "a forward recorder panicked on a fill — this deployment's realised \
                         series is missing that fill and cannot be given it"
                    );
                }
            }
        }

        if attributed {
            self.fills_attributed.fetch_add(1, Ordering::Relaxed);
        } else {
            self.fills_unattributed.fetch_add(1, Ordering::Relaxed);
            // Not an error: a manual paper order is a fill no deployment
            // submitted, and it is correct for it to enter no forward record.
            // It is counted so that an empty forward record beside a busy
            // `fills` table is an explained fact rather than a mystery.
            tracing::debug!(
                idempotency_key = %fill.idempotency_key,
                deployments = deployments.len(),
                "a fill matched no deployment — in no forward record, by design"
            );
        }
    }
}

/// What a process can say about its own forward record without guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForwardRecordingEvidence {
    /// Whether [`ForwardRecorders::install`] has run in this process.
    pub installed: bool,
    /// Deployments recording here.
    pub deployments: usize,
    /// Fills the execution stream had already published when this attached
    /// (or, when nothing is installed, every fill the process has taken).
    /// Unrecoverable — the fill stream has no replay.
    pub fills_before_install: u64,
    /// Fills delivered to this registry.
    pub fills_seen: u64,
    /// Of those, the ones a deployment claimed.
    pub fills_attributed: u64,
    /// Of those, the ones no deployment claimed. Expected for manual orders.
    pub fills_unattributed: u64,
    /// Deliveries lost to a panicking recorder.
    pub recorder_failures: u64,
}

impl ForwardRecordingEvidence {
    /// The state of this process's forward record, as one value.
    #[must_use]
    pub fn verdict(&self) -> ForwardRecordingVerdict {
        if !self.installed {
            return ForwardRecordingVerdict::NotInstalled;
        }
        let lost = self.fills_before_install + self.recorder_failures;
        if lost > 0 {
            return ForwardRecordingVerdict::AttachedLate { fills: lost };
        }
        if self.deployments == 0 {
            return ForwardRecordingVerdict::NoDeployment {
                fills_seen: self.fills_seen,
            };
        }
        ForwardRecordingVerdict::Recording {
            fills_attributed: self.fills_attributed,
            fills_unattributed: self.fills_unattributed,
        }
    }
}

/// Why `mlops.forward_observation` holds the number of rows it holds.
///
/// Four distinguishable states. Three of them produce an empty table and they
/// do not mean the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardRecordingVerdict {
    /// Reconciliation never attached to the fill stream. An empty forward
    /// record measures nothing — it is not a zero.
    NotInstalled,
    /// Attached after fills had already been published, or lost a delivery to a
    /// panicking recorder. Rows are missing that cannot be supplied.
    AttachedLate { fills: u64 },
    /// Attached, nothing deployed. The forward clock has not started; fills
    /// seen here belong to manual orders.
    NoDeployment { fills_seen: u64 },
    /// Attached with deployments, no gap. An empty forward record in this state
    /// genuinely means those deployments have not traded.
    Recording {
        fills_attributed: u64,
        fills_unattributed: u64,
    },
}

/// The reason a process must not be trusted to produce forward evidence.
#[derive(Clone, Copy, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ForwardRecordingGap {
    #[error(
        "reconciliation is not attached to the fill stream: fills would be taken and no forward \
         observation recorded. Call reconciliation::ForwardRecorders::install() at startup, \
         before the first order can be submitted"
    )]
    NotInstalled,
    #[error(
        "{fills} fill(s) were published before reconciliation attached, or were lost to a \
         panicking recorder; the forward record is missing rows that cannot be recovered — the \
         fill stream has no replay"
    )]
    AttachedLate { fills: u64 },
    #[error(
        "no deployment is recording: the forward clock has not started, so an empty forward \
         record is an absent measurement rather than a zero"
    )]
    NoDeployment,
}
