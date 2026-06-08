//! Position/balance/freshness/sequence reconciliation.
//! Where money is actually saved — halt on divergence before any new orders.

pub mod divergence;
pub mod freshness;
pub mod positions;
pub mod sequence;

pub use divergence::{check_position_divergence, ReconcileOutcome};
pub use freshness::{check_freshness, FreshnessOutcome};
pub use positions::{reconcile_all, reconcile_one, reconcile_with_broker, InternalPosition};
pub use sequence::SequenceTracker;
