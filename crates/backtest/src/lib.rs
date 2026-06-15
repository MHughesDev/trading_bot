//! Backtesting orchestrator — runs strategy definitions against historical
//! data using the `market_simulator` engine as an embedded SDK.
//!
//! # Architecture
//!
//! The platform owns everything: strategy definitions (v1.0 JSON), historical
//! bars (`ClickHouse` `market_bars`), and job state.  The simulator is a pure
//! processing engine — it receives bars and a callback in memory, simulates
//! fills per asset class, and returns results.  Nothing is persisted on the
//! simulator side.
//!
//! # Job lifecycle
//!
//! ```text
//! Queued → CheckingData → [CollectingData] → LoadingData → Simulating → Completed
//!                                                              ↘ Failed / Cancelled
//! ```
//!
//! `CheckingData` derives the data requirements of the strategy being tested
//! (timeframe lane + indicator warm-up) and measures `ClickHouse` coverage for
//! the requested window.  When coverage has gaps and auto-collect is enabled,
//! `CollectingData` speed-runs a historical backfill from a venue REST API
//! (paged 1000-bar requests) straight into `ClickHouse`, then re-checks.

pub mod aggregate;
pub mod collect;
pub mod gaps;
pub mod manager;
pub mod requirements;
pub mod sim;
pub mod store;
pub mod types;
pub mod warmup;

pub use aggregate::aggregate_bars;
pub use collect::CollectorPlan;
pub use manager::BacktestManager;
pub use types::{
    BacktestRequest, BacktestSnapshot, BacktestStatus, DataCoverage, MissingRange, ResolvedSpec,
    TimeframeExt,
};
pub use store::{BarStore, CollectedBar};
pub use warmup::{WarmState, load_warm_state, run_indicators};
