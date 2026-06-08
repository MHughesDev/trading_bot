//! TODO(Phase 1): NATS JetStream producer/consumer wrappers and lane naming.
//! This is the only crate that knows the bus is NATS; everyone else uses these traits.
pub mod backpressure;
pub mod lanes;
pub mod nats;
pub mod publish;
pub mod quarantine;
pub mod subscribe;
