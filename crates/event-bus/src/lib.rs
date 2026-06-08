//! NATS JetStream producer/consumer wrappers and lane naming.
//!
//! This is the only crate that knows the bus is NATS; every other crate depends
//! on these types and never imports `async_nats` directly.

pub mod backpressure;
pub mod lanes;
pub mod nats;
pub mod publish;
pub mod quarantine;
pub mod subscribe;

pub use backpressure::Backpressure;
pub use lanes::subject_for;
pub use nats::{connect, setup_streams, NatsClient};
pub use publish::Publisher;
pub use quarantine::QuarantinePublisher;
pub use subscribe::Subscriber;

/// Alias for the bus-level error type.
pub type BusError = nats::BusError;
