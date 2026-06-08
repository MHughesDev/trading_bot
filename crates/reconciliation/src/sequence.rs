//! Sequence-gap tracking for reconciliation.
//!
//! Consumes `gap.detected` events and marks the affected lane/instrument
//! window as suspect.  The kill switch is not tripped here — a gap means
//! the data may be incomplete, which is handled by the bar-revision
//! mechanism.  Only a position divergence trips the kill switch.

use std::collections::HashSet;

/// A gap record indicating a potential data-completeness problem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SuspectWindow {
    pub instrument_id: String,
    pub lane: String,
}

/// Tracks windows that have been flagged as suspect due to sequence gaps.
#[derive(Default)]
pub struct SequenceTracker {
    suspect: HashSet<SuspectWindow>,
}

impl SequenceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a window as suspect.
    pub fn mark_suspect(&mut self, instrument_id: &str, lane: &str) {
        self.suspect.insert(SuspectWindow {
            instrument_id: instrument_id.to_owned(),
            lane: lane.to_owned(),
        });
        tracing::warn!(
            %instrument_id,
            %lane,
            "sequence gap detected — window marked suspect"
        );
    }

    /// Clear suspect status (e.g. after a successful reconciliation).
    pub fn clear(&mut self, instrument_id: &str, lane: &str) {
        self.suspect.remove(&SuspectWindow {
            instrument_id: instrument_id.to_owned(),
            lane: lane.to_owned(),
        });
    }

    pub fn is_suspect(&self, instrument_id: &str, lane: &str) -> bool {
        self.suspect.contains(&SuspectWindow {
            instrument_id: instrument_id.to_owned(),
            lane: lane.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_and_query_suspect() {
        let mut tracker = SequenceTracker::new();
        assert!(!tracker.is_suspect("BTC-USD", "market.trades"));
        tracker.mark_suspect("BTC-USD", "market.trades");
        assert!(tracker.is_suspect("BTC-USD", "market.trades"));
    }

    #[test]
    fn clear_removes_suspect() {
        let mut tracker = SequenceTracker::new();
        tracker.mark_suspect("BTC-USD", "market.trades");
        tracker.clear("BTC-USD", "market.trades");
        assert!(!tracker.is_suspect("BTC-USD", "market.trades"));
    }

    #[test]
    fn different_instruments_are_independent() {
        let mut tracker = SequenceTracker::new();
        tracker.mark_suspect("BTC-USD", "market.trades");
        assert!(!tracker.is_suspect("ETH-USD", "market.trades"));
    }
}
