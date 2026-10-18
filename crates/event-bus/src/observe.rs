//! Metric hooks for crates that feed the bus but carry no `observability`
//! dependency of their own (Set E 5.1).
//!
//! `collectors` calls [`gap_detected`] from `GapDetector::check`, the one place
//! every collector's sequence gaps pass through, so each of them is counted
//! without each collector being edited.

/// A sequence gap was detected on `lane`.
pub fn gap_detected(lane: &str) {
    observability::Metrics::new().increment_gap_detected(lane);
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_gap_reaches_the_registry_by_lane() {
        super::gap_detected("observe-test-lane");
        super::gap_detected("observe-test-lane");
        assert_eq!(
            observability::metrics::global().counter_value(
                "pipeline_sequence_gaps_total",
                &[("lane", "observe-test-lane")]
            ),
            Some(2)
        );
    }
}
