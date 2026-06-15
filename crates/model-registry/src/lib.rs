//! AI Model Studio orchestrator — mirrors `crates/backtest`.
//!
//! `ModelManager` owns model identity, training/eval jobs, alias management,
//! and drives async job execution.  In Phase 1, training and evaluation jobs
//! are stub drivers that advance through phases on a timer.  Phase 2 replaces
//! the driver body with real Python sidecar calls without changing the public API.

pub mod job;
pub mod manager;
pub mod types;

pub use manager::ModelManager;
pub use types::{
    CreateModelRequest, ModelRecord, ModelRunKind, ModelRunSnapshot, RunStatus, TrainRequest,
};
