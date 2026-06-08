//! Aggregates lane/instrument demand; starts/stops pipelines on demand.
//! Data engines never start at system init — only when at least one consumer declares demand.
pub mod lifecycle;
pub mod registry;

pub use lifecycle::{NoopPipelineFactory, PipelineFactory, PipelineHandle};
pub use registry::DemandRegistry;
