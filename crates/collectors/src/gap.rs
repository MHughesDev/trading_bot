//! Sequence-gap detection for per-lane, per-instrument streams.
//!
//! Each collector maintains a [`GapDetector`] per stream.  When a sequence
//! number arrives that is not the expected next value, the detector emits a
//! [`GapEvent`] so the caller can trigger a snapshot re-request or metric.

/// Describes a detected sequence gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapEvent {
    pub instrument_id: String,
    pub lane: String,
    pub expected: u64,
    pub got: u64,
}

/// Tracks the last observed sequence number and detects gaps.
pub struct GapDetector {
    last_sequence: Option<u64>,
    instrument_id: String,
    lane: String,
}

impl GapDetector {
    /// Create a new detector for `instrument_id` on `lane`.
    pub fn new(instrument_id: impl Into<String>, lane: impl Into<String>) -> Self {
        Self {
            last_sequence: None,
            instrument_id: instrument_id.into(),
            lane: lane.into(),
        }
    }

    /// Check a new sequence number.
    ///
    /// Returns `Some(GapEvent)` when the sequence is not contiguous with the
    /// previous one.  Returns `None` on the very first call (no baseline yet)
    /// and when the sequence is exactly `last + 1`.
    pub fn check(&mut self, sequence: u64) -> Option<GapEvent> {
        let result = match self.last_sequence {
            None => None,
            Some(last) => {
                let expected = last.wrapping_add(1);
                if sequence != expected {
                    Some(GapEvent {
                        instrument_id: self.instrument_id.clone(),
                        lane: self.lane.clone(),
                        expected,
                        got: sequence,
                    })
                } else {
                    None
                }
            }
        };
        self.last_sequence = Some(sequence);
        result
    }

    /// Reset the detector (e.g. after a reconnect / snapshot).
    pub fn reset(&mut self) {
        self.last_sequence = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gap_on_first_sequence() {
        let mut d = GapDetector::new("BTC-USD", "market.trades");
        assert!(d.check(1).is_none());
    }

    #[test]
    fn no_gap_on_consecutive_sequences() {
        let mut d = GapDetector::new("BTC-USD", "market.trades");
        d.check(1);
        assert!(d.check(2).is_none());
        assert!(d.check(3).is_none());
    }

    #[test]
    fn detects_gap() {
        let mut d = GapDetector::new("BTC-USD", "market.trades");
        d.check(1);
        let gap = d.check(5);
        assert!(gap.is_some());
        let g = gap.unwrap();
        assert_eq!(g.expected, 2);
        assert_eq!(g.got, 5);
    }

    #[test]
    fn reset_clears_baseline() {
        let mut d = GapDetector::new("BTC-USD", "market.trades");
        d.check(100);
        d.reset();
        // After reset the first sequence should never trigger a gap.
        assert!(d.check(999).is_none());
    }

    #[test]
    fn gap_event_fields_correct() {
        let mut d = GapDetector::new("ETH-USD", "market.quotes");
        d.check(10);
        let gap = d.check(15).unwrap();
        assert_eq!(gap.instrument_id, "ETH-USD");
        assert_eq!(gap.lane, "market.quotes");
        assert_eq!(gap.expected, 11);
        assert_eq!(gap.got, 15);
    }
}
