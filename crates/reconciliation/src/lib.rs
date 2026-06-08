//! TODO(Phase 2): position/balance/freshness/sequence reconciliation.
//! Where money is actually saved — halt on divergence before any new orders.
pub mod positions;
pub mod freshness;
pub mod sequence;
pub mod divergence;
