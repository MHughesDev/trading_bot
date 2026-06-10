//! Aggregates lane/instrument demand; starts/stops pipelines on demand.
//! Data engines never start at system init — only when at least one consumer declares demand.
pub mod lane_key;
pub mod lifecycle;
pub mod rate_budget;
pub mod registry;

pub use lane_key::LaneKey;
pub use lifecycle::{NoopPipelineFactory, PipelineFactory, PipelineHandle};
pub use rate_budget::{BudgetExceeded, RateBudget, VenueBudget};
pub use registry::DemandRegistry;
