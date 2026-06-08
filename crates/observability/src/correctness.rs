/// Correctness metrics for the data pipeline — Phase 1 stubs.
#[derive(Default)]
pub struct CorrectnessMetrics;

impl CorrectnessMetrics {
    pub fn new() -> Self {
        Self
    }

    pub fn record_consumer_lag(&self, _lane: &str, _lag_ms: i64) {}

    pub fn record_quarantine_rate(&self, _source: &str, _rate: f64) {}

    pub fn record_reconciliation_divergence(&self, _count: u64) {}
}
