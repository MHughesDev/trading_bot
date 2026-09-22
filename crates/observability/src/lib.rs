//! Tracing setup, metric handles, and the silence alarm.
//!
//! [`silence`] is the substantial module here: it decides whether a data feed
//! that has stopped arriving is a collector that died or a market that is shut.
//! The rest is process plumbing.

pub mod correctness;
pub mod metrics;
pub mod silence;
pub mod tracing_setup;

pub use correctness::CorrectnessMetrics;
pub use metrics::Metrics;
pub use silence::{
    emit, evaluate, Arrival, Census, FeedExpectation, FeedKey, Heartbeat, RegistryRow, Roster, Severity,
    SilenceReport, Verdict,
};

/// Initialize tracing for a service using the default (non-JSON) format.
///
/// Reads `RUST_LOG` from the environment; falls back to `"info"`.
pub fn init(service_name: &str) {
    tracing_setup::init_tracing(service_name, false);
}

/// Initialize tracing with JSON output (for production containers).
pub fn init_json(service_name: &str) {
    tracing_setup::init_tracing(service_name, true);
}
