//! Position/balance/freshness/sequence reconciliation.
//! Where money is actually saved — halt on divergence before any new orders.

pub mod divergence;
pub mod forward;
pub mod freshness;
pub mod gate15;
pub mod positions;
pub mod recorder;
pub mod sequence;
pub mod store;

pub use divergence::{
    check_position_divergence, check_position_divergence_at_venue, DivergenceMonitor,
    DivergenceVerdict, ReconcileOutcome,
};
pub use forward::{
    AlignedBar, AlignmentError, DeploymentSpec, DivergenceThresholds, ExpectedBar, ForwardError,
    PaperVenue, RealisedSeries, RealisedSeriesBuilder,
};
pub use freshness::{check_freshness, FreshnessOutcome};
pub use gate15::record_gate15_statistics;
pub use positions::{reconcile_all, reconcile_one, reconcile_with_broker, InternalPosition};
pub use recorder::ForwardRecorder;
pub use sequence::SequenceTracker;
