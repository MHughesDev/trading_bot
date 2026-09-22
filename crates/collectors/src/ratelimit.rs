//! Rate limiting that can be *proven*, not hoped for (Set P MASTER §5).
//!
//! A `sleep` in a loop is not a rate limit. It is a delay whose correctness
//! nobody can assert, it drifts under retries, and the first concurrent caller
//! doubles the real rate. Getting the platform banned from a free endpoint costs
//! more than every row the burst fetched, so the limit is a value here, honoured
//! by a token bucket, and the tests assert the bucket **refuses**.
//!
//! The clock is a parameter. [`TokenBucket::try_acquire_at`] takes the instant it
//! is being asked about, so a test can drive a whole second of traffic in
//! microseconds and assert the eleventh request in that second is denied. The
//! wall-clock wrapper [`TokenBucket::try_acquire`] exists for production and is
//! two lines over the same code path.

use std::time::{Duration, Instant};

/// A token bucket: `capacity` tokens, refilled at `rate_per_sec`.
///
/// Fractional tokens are tracked in micro-tokens (1e-6 of a token) so the refill
/// is exact integer arithmetic — a float accumulator drifts, and a limiter that
/// drifts upward is a limiter that eventually bans you.
#[derive(Clone, Debug)]
pub struct TokenBucket {
    capacity_micro: u64,
    refill_micro_per_sec: u64,
    tokens_micro: u64,
    last_refill: Duration,
    origin: Option<Instant>,
}

const MICRO: u64 = 1_000_000;

impl TokenBucket {
    /// A bucket permitting `rate_per_sec` sustained requests with a burst of
    /// `burst` (use `burst == rate_per_sec` for a plain per-second cap).
    ///
    /// # Panics
    /// If `rate_per_sec` or `burst` is zero — a limiter that permits nothing is a
    /// configuration mistake, not a strict policy.
    #[must_use]
    pub fn new(rate_per_sec: u32, burst: u32) -> Self {
        assert!(rate_per_sec > 0, "rate_per_sec must be positive");
        assert!(burst > 0, "burst must be positive");
        let capacity_micro = u64::from(burst) * MICRO;
        Self {
            capacity_micro,
            refill_micro_per_sec: u64::from(rate_per_sec) * MICRO,
            tokens_micro: capacity_micro,
            last_refill: Duration::ZERO,
            origin: None,
        }
    }

    /// The sustained rate this bucket permits, in requests per second.
    #[must_use]
    pub fn rate_per_sec(&self) -> u64 {
        self.refill_micro_per_sec / MICRO
    }

    /// Whether a request is permitted at `now`, consuming a token if so.
    ///
    /// `now` is measured from an arbitrary origin and must be non-decreasing
    /// across calls; a rewound clock refills nothing rather than granting free
    /// tokens.
    pub fn try_acquire_at(&mut self, now: Duration) -> bool {
        if now > self.last_refill {
            let elapsed = now - self.last_refill;
            let refill = (elapsed.as_micros() as u64)
                .saturating_mul(self.refill_micro_per_sec)
                / MICRO;
            self.tokens_micro = self.tokens_micro.saturating_add(refill).min(self.capacity_micro);
            self.last_refill = now;
        }
        if self.tokens_micro >= MICRO {
            self.tokens_micro -= MICRO;
            true
        } else {
            false
        }
    }

    /// Wall-clock variant for production callers.
    pub fn try_acquire(&mut self) -> bool {
        let origin = *self.origin.get_or_insert_with(Instant::now);
        self.try_acquire_at(origin.elapsed())
    }

    /// How long to wait, from `now`, before a token is available.
    #[must_use]
    pub fn wait_for_token(&self, now: Duration) -> Duration {
        let refilled = if now > self.last_refill {
            let elapsed = now - self.last_refill;
            (elapsed.as_micros() as u64).saturating_mul(self.refill_micro_per_sec) / MICRO
        } else {
            0
        };
        let available = self
            .tokens_micro
            .saturating_add(refilled)
            .min(self.capacity_micro);
        if available >= MICRO {
            return Duration::ZERO;
        }
        let deficit = MICRO - available;
        Duration::from_micros(deficit.saturating_mul(MICRO) / self.refill_micro_per_sec)
    }
}

/// Blocks until a token is available, then takes it.
///
/// The only way collectors in this crate reach an upstream endpoint. Async, so a
/// paused collector costs a task rather than a thread.
pub async fn acquire(bucket: &mut TokenBucket) {
    loop {
        let origin = *bucket.origin.get_or_insert_with(Instant::now);
        let now = origin.elapsed();
        if bucket.try_acquire_at(now) {
            return;
        }
        let wait = bucket.wait_for_token(now).max(Duration::from_millis(1));
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P-3.2 acceptance: the limiter REFUSES above 10 req/s. This is the whole
    /// point — a test that only shows ten requests succeed proves nothing about
    /// the eleventh.
    #[test]
    fn ten_per_second_refuses_the_eleventh() {
        let mut b = TokenBucket::new(10, 10);
        for i in 0..10 {
            assert!(
                b.try_acquire_at(Duration::ZERO),
                "request {i} within the first second should be permitted"
            );
        }
        assert!(
            !b.try_acquire_at(Duration::from_millis(999)),
            "the eleventh request inside one second must be refused"
        );
    }

    #[test]
    fn a_full_second_restores_the_whole_budget() {
        let mut b = TokenBucket::new(10, 10);
        for _ in 0..10 {
            assert!(b.try_acquire_at(Duration::ZERO));
        }
        assert!(!b.try_acquire_at(Duration::from_millis(50)));
        assert!(b.try_acquire_at(Duration::from_secs(1)));
    }

    /// The sustained rate over a long window never exceeds the declared rate,
    /// whatever pattern the caller uses.
    #[test]
    fn the_sustained_rate_never_exceeds_the_declared_rate() {
        let mut b = TokenBucket::new(10, 10);
        let mut granted = 0usize;
        // Ask every millisecond for ten seconds: 10_000 attempts.
        for ms in 0..10_000u64 {
            if b.try_acquire_at(Duration::from_millis(ms)) {
                granted += 1;
            }
        }
        // 10 burst + 10/s for ~10s. Never more.
        assert!(granted <= 10 + 10 * 10, "granted {granted} exceeds the budget");
    }

    #[test]
    fn a_rewound_clock_grants_no_free_tokens() {
        let mut b = TokenBucket::new(1, 1);
        assert!(b.try_acquire_at(Duration::from_secs(10)));
        // Clock goes backwards (NTP step, or a caller mixing clocks).
        assert!(!b.try_acquire_at(Duration::from_secs(1)));
    }

    #[test]
    fn the_wait_is_the_time_to_the_next_token() {
        let mut b = TokenBucket::new(2, 2);
        assert!(b.try_acquire_at(Duration::ZERO));
        assert!(b.try_acquire_at(Duration::ZERO));
        assert!(!b.try_acquire_at(Duration::ZERO));
        assert_eq!(b.wait_for_token(Duration::ZERO), Duration::from_millis(500));
    }

    #[test]
    fn a_burst_larger_than_the_rate_is_capped_at_the_burst() {
        let mut b = TokenBucket::new(1, 5);
        for _ in 0..5 {
            assert!(b.try_acquire_at(Duration::ZERO));
        }
        assert!(!b.try_acquire_at(Duration::ZERO));
        // Idle for a minute: the bucket fills to the burst, not beyond.
        let mut granted = 0;
        for _ in 0..10 {
            if b.try_acquire_at(Duration::from_secs(60)) {
                granted += 1;
            }
        }
        assert_eq!(granted, 5);
    }
}
