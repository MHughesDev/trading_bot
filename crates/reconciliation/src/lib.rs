//! Position/balance/freshness/sequence reconciliation.
//! Where money is actually saved — halt on divergence before any new orders.

pub mod divergence;
pub mod drills;
pub mod fleet;
pub mod fleet_job;
pub mod forward;
pub mod freshness;
pub mod gate15;
pub mod halts;
pub mod positions;
pub mod recorder;
pub mod sequence;
pub mod store;

pub use divergence::{
    check_position_divergence, check_position_divergence_at_venue, compare_position,
    DivergenceMonitor, DivergenceVerdict, ReconcileOutcome,
};
pub use fleet::{
    CommonModeFinding, DeploymentDivergence, Direction, FleetError, FleetInconclusive, FleetPolicy,
    FleetReport, FleetSummary, FleetVerdict,
};
pub use forward::{
    AlignedBar, AlignmentError, DeploymentSpec, DivergenceThresholds, ExpectedBar, ForwardError,
    GappedAlignment, PaperVenue, RealisedSeries, RealisedSeriesBuilder, ResumeFill, SeriesGap,
    ShadowBar,
};
pub use freshness::{check_freshness, FreshnessOutcome};
pub use gate15::record_gate15_statistics;
pub use positions::{
    compare_all, compare_one, reconcile_all, reconcile_one, reconcile_with_broker,
    InternalPosition, SweepError,
};
pub use recorder::{
    ForwardRecorder, ForwardRecorders, ForwardRecordingEvidence, ForwardRecordingGap,
    ForwardRecordingVerdict,
};
pub use sequence::SequenceTracker;

/// The monitor's store, re-exported for the platform process.
///
/// `apps/platform` depends on this crate and not on `monitor`, and its manifest
/// is not this crate's to edit. The platform's feed producer and drill schedule
/// write monitor tables through `monitor::store::MonitorStore` and nothing else
/// — one writer per table — so they reach it through here rather than with raw
/// SQL that would be a second writer.
pub use monitor;
