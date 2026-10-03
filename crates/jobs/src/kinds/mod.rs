//! Job kinds whose logic belongs to the job service rather than to a caller.
//!
//! Most kinds are implemented where their subject lives — a backtest worker
//! beside the backtest engine, a gate worker beside the gates. A kind lands here
//! when what it *is* is a job-service concern: durability, resumption and
//! idempotency, with the domain work delegated through a trait.

pub mod backfill;
pub mod predict_series;
