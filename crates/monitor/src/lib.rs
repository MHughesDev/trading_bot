//! COMP-007 — the live strategy monitor and kill switch.
//!
//! # The one property this crate exists to have
//!
//! **The checking mechanism is independent of the thing it checks.** A monitor that
//! runs inside the API process dies with it, and a monitor that died has not
//! checked anything — it has only stopped disagreeing. So this is a separate
//! binary, with its own database connection and its own heartbeat, and the only
//! thing joining it to the order path is a lease row it renews every cadence.
//!
//! The coupling runs the safe way round. The order path does not ask "has the
//! monitor told me to stop"; it asks "has the monitor told me it is alive,
//! recently". Silence is a halt. A crash, a wedge, a lost database connection and a
//! deliberate global trip all produce the same answer at the gate, and that answer
//! is *no*.
//!
//! # What it watches, and in what order
//!
//! P&L is a lagging signal. By the time an equity curve says a strategy is broken,
//! it has been broken for a while and has been paying for it. So the families are
//! ordered inputs-first: execution quality and behaviour drift before P&L, data
//! staleness and venue health before either.
//!
//! # Why there is dwell, and why the monitor is scored
//!
//! A 3σ band on a noisy intraday metric breaches by construction. A monitor that
//! cries wolf trains the operator to click through it, and here the thing being
//! clicked through is the last line before money. So a metric must sit outside its
//! limit for three consecutive evaluations before anything happens, recovery needs
//! twice that, and every breach is labelled afterwards so the monitor's own
//! precision per family is a published number rather than an assumption.
//!
//! Three things skip dwell, because they are facts rather than estimates: a
//! reconciliation mismatch, a lost lease, and data staleness past the hard
//! multiple.

pub mod baseline;
pub mod control;
pub mod drill;
pub mod dwell;
pub mod evaluate;
pub mod events;
pub mod family;
pub mod lease;
pub mod limits;
pub mod precision;
pub mod service;
pub mod state;
pub mod store;
pub mod window;

pub use baseline::{Cusum, CusumVerdict, Ewma};
pub use control::{ResetRefusal, ResetRequest};
pub use drill::{DrillOutcome, DrillTrigger};
pub use dwell::{DwellConfig, DwellTracker, DwellVerdict};
pub use evaluate::{Evaluation, Evaluator, MetricReading, Outcome};
pub use events::{EventKind, MonitorEvent};
pub use family::{Fact, Family};
pub use lease::{Lease, LeaseStatus, LeaseTerms};
pub use limits::{ControlLimit, LimitBook, LimitMethod, LimitSeverity, PromotionRefusal};
pub use precision::{FamilyPrecision, PrecisionLabel};
pub use service::{LeaseWatcher, MonitorService};
pub use state::{Action, HaltPolicy, Severity, StrategyState, Transition};
pub use store::{MonitorStore, StoreError};
pub use window::MaintenanceWindow;

/// The monitor's default evaluation cadence (COMP-007 §3).
pub const DEFAULT_CADENCE_MS: u64 = 60_000;

/// The faster cadence for order-flow metrics (COMP-007 §3).
pub const ORDER_FLOW_CADENCE_MS: u64 = 5_000;

/// Size a breached strategy is scaled to, in per cent (COMP-007 §4).
///
/// Fixed rather than proportional to the breach, per §10's open question: start
/// fixed, revisit with monitor event data rather than with an opinion.
pub const SCALE_DOWN_PCT: u8 = 25;

/// The lease id. One row, one monitor.
pub const LEASE_ID: &str = "tbot-monitor";
