pub mod correctness;
pub mod metrics;
pub mod tracing_setup;

pub use correctness::CorrectnessMetrics;
pub use metrics::Metrics;

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
