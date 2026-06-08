//! The four-timestamp model and `available_time` semantics.
//!
//! `available_time` is the **load-bearing replay clock**.  The replay engine
//! sorts exclusively by `available_time`; strategies receive an event only after
//! its `available_time` has elapsed on the simulated clock.  This makes
//! lookahead impossible by construction — you cannot receive an event "from your
//! own future" because the clock only advances past its `available_time`.
//!
//! # Computing `available_time` for bars
//!
//! ```text
//! available_time = max(window_close, observed_time) + watermark + processing_delay
//! ```
//!
//! For live data the bar builder stamps `available_time` identically to what the
//! replay loader would produce for the same raw event — the builder is a pure
//! function called from both paths.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// Four timestamps carried by every `EventEnvelope`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamps {
    /// When the source *says* the event happened (optional — some venues omit it).
    pub event_time: Option<DateTime<Utc>>,
    /// When the collector received the raw bytes from the network.
    pub observed_time: DateTime<Utc>,
    /// When the normalized `EventEnvelope` was published onto the NATS bus.
    pub ingested_time: DateTime<Utc>,
    /// When a downstream consumer (strategy, feature engine) is **allowed** to use
    /// this event.  This is the replay sort key.  Always >= `ingested_time`.
    pub available_time: DateTime<Utc>,
}

/// Parameters for computing `available_time` of a bar event.
pub struct AvailableTimeParams {
    /// Timestamp at which the bar window closed (e.g. end of the 1-minute interval).
    pub window_close: DateTime<Utc>,
    /// When the collector actually observed the close data on the wire.
    pub observed_time: DateTime<Utc>,
    /// Per-source watermark allowance (typically 2 s for liquid CEX streams).
    pub watermark: Duration,
    /// Additional propagation delay through the normalization pipeline.
    pub processing_delay: Duration,
}

/// Compute `available_time = max(window_close, observed_time) + watermark + processing_delay`.
pub fn compute_available_time(p: &AvailableTimeParams) -> DateTime<Utc> {
    let base = p.window_close.max(p.observed_time);
    base + p.watermark + p.processing_delay
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).unwrap()
    }

    #[test]
    fn available_time_uses_max_of_window_and_observed() {
        let params = AvailableTimeParams {
            window_close: ts(1_000),
            observed_time: ts(1_001), // observed is later
            watermark: Duration::seconds(2),
            processing_delay: Duration::milliseconds(50),
        };
        // base = max(1000, 1001) = 1001 → + 2s + 0.05s = 1003.05s → 1003s (floor) + 50ms
        let at = compute_available_time(&params);
        assert_eq!(at, ts(1_003) + Duration::milliseconds(50));
    }

    #[test]
    fn available_time_uses_window_when_later() {
        let params = AvailableTimeParams {
            window_close: ts(2_000),
            observed_time: ts(1_999), // observed is earlier
            watermark: Duration::seconds(2),
            processing_delay: Duration::seconds(0),
        };
        let at = compute_available_time(&params);
        assert_eq!(at, ts(2_002));
    }

    #[test]
    fn processing_delay_is_additive() {
        let params = AvailableTimeParams {
            window_close: ts(500),
            observed_time: ts(500),
            watermark: Duration::seconds(2),
            processing_delay: Duration::milliseconds(100),
        };
        let at = compute_available_time(&params);
        assert_eq!(at, ts(502) + Duration::milliseconds(100));
    }
}
