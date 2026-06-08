//! TODO(Phase 1): aggregates lane/instrument demand; starts/stops pipelines on demand.
//! Data engines never start at system init — only when at least one consumer declares demand.
pub mod registry;
pub mod lifecycle;
