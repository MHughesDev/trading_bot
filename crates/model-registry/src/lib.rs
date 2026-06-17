//! AI Model Studio orchestrator — mirrors `crates/backtest`.
//!
//! `ModelManager` owns model identity, training/eval jobs, alias management,
//! and drives async job execution.  In Phase 1, training and evaluation jobs
//! are stub drivers that advance through phases on a timer.  Phase 2 replaces
//! the driver body with real Python sidecar calls without changing the public API.

pub mod backtest_bridge;
pub mod data_view;
pub mod datasets;
pub mod ensemble_manager;
pub mod inference_gateway;
pub mod job;
pub mod leaderboard;
pub mod manager;
pub mod metrics;
pub mod nats_bridge;
pub mod pipeline_manager;
pub mod quality_monitor;
pub mod regression;
pub mod report;
pub mod scheduler;
pub mod scorecard;
pub mod sidecar;
pub mod spec_hash;
pub mod tags;
pub mod types;

pub use inference_gateway::InferenceGateway;
pub use manager::ModelManager;
pub use types::{
    CreateModelRequest, ModelRecord, ModelRunKind, ModelRunSnapshot, RunStatus, TrainDataSelection,
    TrainRequest,
};
