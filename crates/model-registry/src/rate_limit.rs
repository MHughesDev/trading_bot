//! A sliding-window rate limiter for model inference (Set N N-4.5, H-4.7).
//!
//! Every inference that reaches the sidecar costs a model load, a feature row and a trace; a
//! strategy stuck in a loop, or an agent calling `call_model` in one, can issue thousands a
//! minute against one model. The only limit that existed was on the *test* route. This one sits
//! in [`crate::InferenceGateway`], so every caller (strategies, the call path, the agent's
//! tools) passes through it.
//!
//! The clock is a parameter so the window is testable without sleeping.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default: calls per model per minute. Generous for a real strategy (one call per bar per
/// instrument), tight enough that a runaway loop is stopped in seconds.
pub const DEFAULT_MAX_PER_WINDOW: usize = 1200;
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct SlidingWindow {
    max: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl SlidingWindow {
    /// # Panics
    /// If `max` is zero: a limiter that admits nothing is a configuration error, not a limit.
    #[must_use]
    pub fn new(max: usize, window: Duration) -> Self {
        assert!(max > 0, "a rate limit of zero admits nothing");
        Self {
            max,
            window,
            hits: Mutex::new(HashMap::new()),
        }
    }

    /// `MODEL_INFERENCE_MAX_PER_MIN` overrides the default; an unparseable or zero value is
    /// ignored (the default stands) rather than switching the limit off.
    #[must_use]
    pub fn from_env() -> Self {
        let max = std::env::var("MODEL_INFERENCE_MAX_PER_MIN")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_PER_WINDOW);
        Self::new(max, DEFAULT_WINDOW)
    }

    /// Admit one call for `key` at `now`, or refuse it. A refused call is not recorded, so a
    /// caller that keeps hammering does not extend its own lock-out.
    pub fn admit_at(&self, key: &str, now: Instant) -> bool {
        let mut map = self.hits.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let q = map.entry(key.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.window)
        {
            q.pop_front();
        }
        if q.len() >= self.max {
            return false;
        }
        q.push_back(now);
        true
    }

    pub fn admit(&self, key: &str) -> bool {
        self.admit_at(key, Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nth_plus_one_call_in_a_window_is_refused_and_the_window_slides() {
        let l = SlidingWindow::new(3, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(l.admit_at("m", t0));
        assert!(l.admit_at("m", t0 + Duration::from_secs(1)));
        assert!(l.admit_at("m", t0 + Duration::from_secs(2)));
        assert!(!l.admit_at("m", t0 + Duration::from_secs(3)), "the fourth call is refused");
        // The calls at t0 and t0 + 1 have aged out by t0 + 61: two slots open, no more.
        assert!(l.admit_at("m", t0 + Duration::from_secs(61)));
        assert!(l.admit_at("m", t0 + Duration::from_secs(61)));
        assert!(!l.admit_at("m", t0 + Duration::from_secs(61)));
    }

    #[test]
    fn each_model_has_its_own_window() {
        let l = SlidingWindow::new(1, Duration::from_secs(60));
        let t = Instant::now();
        assert!(l.admit_at("a", t));
        assert!(!l.admit_at("a", t));
        assert!(l.admit_at("b", t));
    }

    #[test]
    fn a_refused_call_does_not_extend_the_lockout() {
        let l = SlidingWindow::new(1, Duration::from_secs(10));
        let t = Instant::now();
        assert!(l.admit_at("m", t));
        for s in 1..9 {
            assert!(!l.admit_at("m", t + Duration::from_secs(s)));
        }
        assert!(l.admit_at("m", t + Duration::from_secs(10)), "the one recorded call has aged out");
    }

    #[test]
    #[should_panic(expected = "admits nothing")]
    fn a_zero_limit_is_a_configuration_error() {
        let _ = SlidingWindow::new(0, Duration::from_secs(1));
    }
}
