//! Correctness signals for the data pipeline (Set E 5.1).
//!
//! Same signatures as the Phase-1 stubs; each now sets a gauge in the
//! process-wide registry ([`crate::metrics::global`]).

use crate::metrics;

#[derive(Default, Clone, Copy)]
pub struct CorrectnessMetrics;

impl CorrectnessMetrics {
    pub fn new() -> Self {
        Self
    }

    pub fn record_consumer_lag(&self, lane: &str, lag_ms: i64) {
        metrics::set(
            "pipeline_consumer_lag_ms",
            "Milliseconds between an event's exchange time and its consumption, by lane.",
            &[("lane", lane)],
            lag_ms as f64,
        );
    }

    pub fn record_quarantine_rate(&self, source: &str, rate: f64) {
        metrics::set(
            "pipeline_quarantine_rate",
            "Fraction of a source's recent events that were quarantined.",
            &[("source", source)],
            rate,
        );
    }

    pub fn record_reconciliation_divergence(&self, count: u64) {
        metrics::set(
            "reconciliation_divergences",
            "Positions or orders on which the platform and its venue disagree.",
            &[],
            count as f64,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_correctness_signals_reach_the_registry() {
        let c = CorrectnessMetrics::new();
        c.record_consumer_lag("corr-test-lane", 42);
        c.record_reconciliation_divergence(3);
        let text = metrics::global().render();
        assert!(
            text.contains("pipeline_consumer_lag_ms{lane=\"corr-test-lane\"} 42"),
            "{text}"
        );
        assert!(text.contains("reconciliation_divergences 3"), "{text}");
    }
}
