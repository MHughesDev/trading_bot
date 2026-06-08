//! Adapter to github.com/MHughesDev/market_simulator.
//!
//! No fill simulation logic and no replay engine in this crate.
//! Translates between this repo's domain types and market_simulator's Arrow IPC contracts.
//!
//! TODO(Phase 4): implement export, run_request, results parsing.
pub mod contract;
pub mod export;
pub mod results;
pub mod run_request;
