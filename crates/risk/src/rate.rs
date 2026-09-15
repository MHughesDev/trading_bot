//! Order-rate accounting owned by the risk gate.
//!
//! Before this module the two production call sites both passed `0, 0` for the
//! per-second and per-minute order counts, so [`crate::limits::check_rate_second`]
//! and [`crate::limits::check_rate_minute`] could never fire.  A limit that
//! cannot fire reads, in an audit log, exactly like a limit that passed.
//!
//! The counts are therefore no longer a caller-supplied number.  The gate keeps
//! them itself, keyed by a [`RateKey`] the caller names (one key per
//! account/strategy/instrument scope), and records a submission only when the
//! gate approves one.  A caller can still *choose the scope*, but it can no
//! longer choose the answer.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// The scope order-rate limits are counted over.
///
/// Manual orders count per user+instrument; strategy orders count per strategy
/// instance.  Both converge on the same gate, so a strategy cannot launder
/// orders through the manual door to get a fresh budget.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RateKey(String);

impl RateKey {
    /// Scope for a manual order placed through the REST door.
    #[must_use]
    pub fn manual(instrument_id: &str) -> Self {
        Self(format!("manual:{instrument_id}"))
    }

    /// Scope for orders emitted by one running strategy instance.
    #[must_use]
    pub fn strategy(strategy_id: &str, instrument_id: &str) -> Self {
        Self(format!("strategy:{strategy_id}:{instrument_id}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Submission counts observed in the trailing second and minute.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RateCounts {
    pub last_second: u32,
    pub last_minute: u32,
}

const MINUTE: Duration = Duration::from_secs(60);
const SECOND: Duration = Duration::from_secs(1);

/// Sliding-window submission log, one deque per key.
///
/// Bounded by time rather than by count: entries older than a minute are the
/// only thing the limits look at, so they are dropped on every touch.  An idle
/// key's deque is removed once it empties, so a long-running process does not
/// accumulate one entry per instrument it ever traded.
#[derive(Debug, Default)]
pub struct RateTracker {
    windows: HashMap<RateKey, VecDeque<Instant>>,
}

impl RateTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts for `key` as of `now`, evicting anything older than a minute.
    pub fn counts_at(&mut self, key: &RateKey, now: Instant) -> RateCounts {
        let Some(window) = self.windows.get_mut(key) else {
            return RateCounts::default();
        };
        while window
            .front()
            .is_some_and(|t| now.duration_since(*t) >= MINUTE)
        {
            window.pop_front();
        }
        if window.is_empty() {
            self.windows.remove(key);
            return RateCounts::default();
        }
        let last_second = window
            .iter()
            .filter(|t| now.duration_since(**t) < SECOND)
            .count() as u32;
        RateCounts {
            last_second,
            last_minute: window.len() as u32,
        }
    }

    /// Counts for `key` right now.
    pub fn counts(&mut self, key: &RateKey) -> RateCounts {
        self.counts_at(key, Instant::now())
    }

    /// Record one approved submission against `key`.
    pub fn record_at(&mut self, key: &RateKey, now: Instant) {
        self.windows.entry(key.clone()).or_default().push_back(now);
    }

    /// Record one approved submission against `key` now.
    pub fn record(&mut self, key: &RateKey) {
        self.record_at(key, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unseen_key_counts_zero() {
        let mut t = RateTracker::new();
        assert_eq!(
            t.counts(&RateKey::manual("BTC-USD")),
            RateCounts {
                last_second: 0,
                last_minute: 0
            }
        );
    }

    #[test]
    fn recorded_submissions_count_in_both_windows() {
        let mut t = RateTracker::new();
        let k = RateKey::manual("BTC-USD");
        let now = Instant::now();
        for _ in 0..3 {
            t.record_at(&k, now);
        }
        let c = t.counts_at(&k, now);
        assert_eq!(c.last_second, 3);
        assert_eq!(c.last_minute, 3);
    }

    #[test]
    fn second_window_expires_before_minute_window() {
        let mut t = RateTracker::new();
        let k = RateKey::strategy("s1", "BTC-USD");
        let t0 = Instant::now();
        t.record_at(&k, t0);
        // Two seconds later: out of the per-second window, still in the minute.
        let c = t.counts_at(&k, t0 + Duration::from_secs(2));
        assert_eq!(c.last_second, 0);
        assert_eq!(c.last_minute, 1);
    }

    #[test]
    fn minute_window_expires_and_key_is_reclaimed() {
        let mut t = RateTracker::new();
        let k = RateKey::manual("BTC-USD");
        let t0 = Instant::now();
        t.record_at(&k, t0);
        let c = t.counts_at(&k, t0 + Duration::from_secs(61));
        assert_eq!(c.last_minute, 0);
        assert!(t.windows.is_empty(), "idle key must not be retained");
    }

    #[test]
    fn scopes_are_independent() {
        let mut t = RateTracker::new();
        let manual = RateKey::manual("BTC-USD");
        let strat = RateKey::strategy("s1", "BTC-USD");
        let now = Instant::now();
        t.record_at(&manual, now);
        assert_eq!(t.counts_at(&strat, now).last_minute, 0);
        assert_eq!(t.counts_at(&manual, now).last_minute, 1);
    }
}
