//! Declared rate limits, honoured by a limiter that **refuses** (Set P §5).
//!
//! A backfill that gets the platform banned from a free endpoint costs more than
//! the history it fetched, and under a free-only data policy (ADR-0027) there is no
//! second vendor to fall back to. So the limit is not a `sleep` sprinkled through a
//! loop — a sleep is a hope, and it cannot be tested. It is a token bucket that
//! answers [`RateLimiter::try_acquire`] with [`Denied`] when the budget is spent,
//! and the caller may not proceed past a denial.
//!
//! The clock is injected, so the test that proves refusal drives time forward by
//! hand rather than sleeping. A limiter test that sleeps proves only that sleeping
//! works.
//!
//! Venue limits are declared as data in [`VenueLimit`] so the number in the code is
//! the number in the venue's documentation, quoted beside it.

use std::sync::Mutex;
use std::time::Duration;

/// A monotonic clock the limiter reads. Injected so tests advance it directly.
pub trait Clock: Send + Sync {
    /// Monotonic nanoseconds. Only differences are meaningful.
    fn now_nanos(&self) -> u128;
}

/// The real clock.
#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_nanos(&self) -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }
}

/// A clock a test drives by hand.
#[derive(Debug, Default)]
pub struct ManualClock(Mutex<u128>);

impl ManualClock {
    #[must_use]
    pub fn new() -> Self {
        Self(Mutex::new(0))
    }

    pub fn advance(&self, d: Duration) {
        let mut g = self.0.lock().expect("manual clock poisoned");
        *g += d.as_nanos();
    }
}

impl Clock for ManualClock {
    fn now_nanos(&self) -> u128 {
        *self.0.lock().expect("manual clock poisoned")
    }
}

/// A shared handle to a manual clock, so a test can hold one and hand the same
/// clock to a limiter that owns its `Box<dyn Clock>`.
impl Clock for std::sync::Arc<ManualClock> {
    fn now_nanos(&self) -> u128 {
        (**self).now_nanos()
    }
}

/// A venue's published limit, with the quote that establishes it.
///
/// `cost_per_request` is in the venue's own units — tokens, credits, or plain
/// requests — so a cost-based budget and a requests-per-second one are the same
/// shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueLimit {
    pub venue: &'static str,
    /// Maximum tokens the bucket holds — the burst.
    pub capacity: u64,
    /// Tokens added per refill window.
    pub refill_tokens: u64,
    /// The refill window.
    pub refill_window: Duration,
    /// What one request costs.
    pub cost_per_request: u64,
    /// The documented sentence this encodes, so nobody has to trust the number.
    pub source_quote: &'static str,
}

impl VenueLimit {
    /// Kraken Futures `/derivatives` REST endpoints.
    ///
    /// Kraken documents the public data endpoints as costing nothing
    /// (<https://docs.kraken.com/api/docs/guides/futures-rate-limits/>: "there is no
    /// cost associated with public endpoints"). We decline the exemption and budget
    /// them against the documented `/derivatives` pool anyway: "no published cost"
    /// is not "no limit", an undocumented limit is the kind that is discovered by
    /// being banned, and the collector's whole duty cycle is a handful of requests a
    /// minute. Spending a documented budget we are not obliged to spend costs us
    /// nothing and removes the only way this collector could lose the source.
    #[must_use]
    pub const fn kraken_futures_derivatives() -> Self {
        Self {
            venue: "kraken_futures",
            capacity: 500,
            refill_tokens: 500,
            refill_window: Duration::from_secs(10),
            cost_per_request: 10,
            source_quote:
                "For /derivatives endpoints, clients can spend up to 500 every 10 seconds.",
        }
    }

    /// Kraken Futures `/history` endpoints — a far tighter pool.
    #[must_use]
    pub const fn kraken_futures_history() -> Self {
        Self {
            venue: "kraken_futures_history",
            capacity: 100,
            refill_tokens: 100,
            refill_window: Duration::from_secs(600),
            cost_per_request: 1,
            source_quote: "For /history endpoints, clients have a pool of up to 100 tokens that continually replenishes at a rate of 100 every 10 minutes.",
        }
    }

    /// Tradier market-data endpoints, production tier.
    #[must_use]
    pub const fn tradier_market_data() -> Self {
        Self {
            venue: "tradier",
            capacity: 120,
            refill_tokens: 120,
            refill_window: Duration::from_secs(60),
            cost_per_request: 1,
            source_quote: "Market Data: 120 requests per minute",
        }
    }

    /// Tradier sandbox, which is half the production allowance.
    #[must_use]
    pub const fn tradier_sandbox() -> Self {
        Self {
            venue: "tradier_sandbox",
            capacity: 60,
            refill_tokens: 60,
            refill_window: Duration::from_secs(60),
            cost_per_request: 1,
            source_quote: "Sandbox — Market Data: 60 request per minute",
        }
    }

    /// SEC EDGAR's structured-data endpoints (Set P, P-3.2).
    ///
    /// The SEC publishes this one plainly and it is binding rather than advisory:
    /// exceeding it gets the operator's traffic blocked, and under a free-only
    /// data policy (ADR-0027) there is no second vendor for filings. Quoted from
    /// <https://www.sec.gov/search-filings/edgar-search-assistance/accessing-edgar-data>,
    /// re-verified 2026-09-22.
    ///
    /// The other half of EDGAR's fair-access policy — a User-Agent that
    /// identifies the operator — is enforced in
    /// [`crate::fundamentals::edgar::EdgarClient`], because it is a property of
    /// the request rather than of the budget.
    #[must_use]
    pub const fn sec_edgar() -> Self {
        Self {
            venue: "sec_edgar",
            capacity: 10,
            refill_tokens: 10,
            refill_window: Duration::from_secs(1),
            cost_per_request: 1,
            source_quote: "Current max request rate: 10 requests/second.",
        }
    }

    /// FRED / ALFRED (Set P, P-3.1).
    ///
    /// **This number is not quoted, because there is nothing to quote.** The FRED
    /// API documentation, the API-key page and the endpoint reference publish no
    /// request ceiling (checked 2026-09-22); a "120 requests per minute" figure
    /// circulates widely but does not appear on any page St. Louis Fed serves as
    /// documentation. An undocumented limit is the kind discovered by being
    /// banned, so this is a self-imposed budget of one request a second — far
    /// under any plausible ceiling, and ample for a vintage backfill that fetches
    /// a whole series' revision history in a single call.
    #[must_use]
    pub const fn fred_api() -> Self {
        Self {
            venue: "fred_api",
            capacity: 60,
            refill_tokens: 60,
            refill_window: Duration::from_secs(60),
            cost_per_request: 1,
            source_quote: "No request ceiling is published on any FRED API documentation page (checked 2026-09-22); this budget is self-imposed, not quoted.",
        }
    }

    /// Alpaca's historical market-data REST endpoints — news and corporate
    /// actions (Set P, P-3.3).
    ///
    /// Alpaca's news documentation states the archive depth and the supplier but
    /// not the per-plan request ceiling, and the reference page that would carry
    /// it is behind a different URL that did not resolve on 2026-09-22. So this
    /// is declared the same way as [`Self::fred_api`]: a self-imposed budget,
    /// stated as self-imposed. Two requests a second paginates a year of news in
    /// minutes and cannot plausibly breach a free-tier limit.
    #[must_use]
    pub const fn alpaca_market_data() -> Self {
        Self {
            venue: "alpaca_market_data",
            capacity: 120,
            refill_tokens: 120,
            refill_window: Duration::from_secs(60),
            cost_per_request: 1,
            source_quote: "Alpaca's historical-news documentation states the archive depth but no request ceiling (checked 2026-09-22); this budget is self-imposed, not quoted.",
        }
    }

    /// Requests per second this limit permits, sustained.
    #[must_use]
    pub fn sustained_rps(&self) -> f64 {
        let per_window = self.refill_tokens as f64 / self.cost_per_request.max(1) as f64;
        per_window / self.refill_window.as_secs_f64()
    }
}

/// Refused: the budget is spent. The caller must not proceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Denied {
    /// How long until enough tokens exist for one request.
    pub retry_after: Duration,
    pub available: u64,
    pub needed: u64,
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "rate limit refused: {} of {} tokens available, retry in {:?}",
            self.available, self.needed, self.retry_after
        )
    }
}

impl std::error::Error for Denied {}

struct Bucket {
    tokens: u64,
    last_refill_nanos: u128,
    /// Requests this limiter has permitted, ever.
    granted: u64,
    /// Requests it has refused, ever. A collector reporting zero denials over a
    /// long backfill is either well inside its budget or not consulting the
    /// limiter; the counter makes the difference visible.
    denied: u64,
}

/// A token bucket that refuses rather than blocks.
pub struct RateLimiter {
    limit: VenueLimit,
    clock: Box<dyn Clock>,
    bucket: Mutex<Bucket>,
}

/// Renders the declared limit only. It deliberately does not lock the bucket:
/// a `Debug` that takes a lock can deadlock inside a `tracing` macro that is
/// already holding it, and the declared limit is the part a reader wants.
impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}

impl RateLimiter {
    #[must_use]
    pub fn new(limit: VenueLimit) -> Self {
        Self::with_clock(limit, Box::new(SystemClock))
    }

    #[must_use]
    pub fn with_clock(limit: VenueLimit, clock: Box<dyn Clock>) -> Self {
        let now = clock.now_nanos();
        let tokens = limit.capacity;
        Self {
            limit,
            clock,
            bucket: Mutex::new(Bucket {
                tokens,
                last_refill_nanos: now,
                granted: 0,
                denied: 0,
            }),
        }
    }

    #[must_use]
    pub fn limit(&self) -> &VenueLimit {
        &self.limit
    }

    /// Take the budget for one request, or refuse.
    ///
    /// # Errors
    /// [`Denied`] when the bucket cannot pay for the request. The caller must not
    /// send it.
    pub fn try_acquire(&self) -> Result<(), Denied> {
        self.try_acquire_n(self.limit.cost_per_request)
    }

    /// Take `cost` from the budget, or refuse.
    ///
    /// # Errors
    /// [`Denied`] when the bucket holds fewer than `cost` tokens.
    pub fn try_acquire_n(&self, cost: u64) -> Result<(), Denied> {
        let now = self.clock.now_nanos();
        let mut b = self.bucket.lock().expect("rate limiter poisoned");
        self.refill(&mut b, now);
        if b.tokens >= cost {
            b.tokens -= cost;
            b.granted += 1;
            return Ok(());
        }
        b.denied += 1;
        let short = cost - b.tokens;
        Err(Denied {
            retry_after: self.time_to_accrue(short),
            available: b.tokens,
            needed: cost,
        })
    }

    /// `(granted, denied)` since construction.
    #[must_use]
    pub fn counters(&self) -> (u64, u64) {
        let b = self.bucket.lock().expect("rate limiter poisoned");
        (b.granted, b.denied)
    }

    /// Tokens currently available, after refilling to the present.
    #[must_use]
    pub fn available(&self) -> u64 {
        let now = self.clock.now_nanos();
        let mut b = self.bucket.lock().expect("rate limiter poisoned");
        self.refill(&mut b, now);
        b.tokens
    }

    fn refill(&self, b: &mut Bucket, now: u128) {
        if now <= b.last_refill_nanos {
            return;
        }
        let window = self.limit.refill_window.as_nanos().max(1);
        let elapsed = now - b.last_refill_nanos;
        // Integer arithmetic: tokens accrue in whole units, and a partial window
        // carries over rather than rounding up into free budget.
        let windows = elapsed / window;
        if windows == 0 {
            return;
        }
        let accrued = u64::try_from(windows.saturating_mul(u128::from(self.limit.refill_tokens)))
            .unwrap_or(u64::MAX);
        b.tokens = b.tokens.saturating_add(accrued).min(self.limit.capacity);
        b.last_refill_nanos += windows * window;
    }

    fn time_to_accrue(&self, tokens: u64) -> Duration {
        if self.limit.refill_tokens == 0 {
            return Duration::MAX;
        }
        let windows = tokens.div_ceil(self.limit.refill_tokens);
        self.limit
            .refill_window
            .saturating_mul(u32::try_from(windows).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn manual(limit: VenueLimit) -> (Arc<ManualClock>, RateLimiter) {
        let clock = Arc::new(ManualClock::new());
        struct Shared(Arc<ManualClock>);
        impl Clock for Shared {
            fn now_nanos(&self) -> u128 {
                self.0.now_nanos()
            }
        }
        let lim = RateLimiter::with_clock(limit, Box::new(Shared(Arc::clone(&clock))));
        (clock, lim)
    }

    /// The acceptance criterion, stated as a test: drive the limiter past its
    /// declared rate and it must **refuse**. Not sleep, not slow down — refuse.
    #[test]
    fn the_limiter_refuses_to_exceed_the_declared_rate() {
        let limit = VenueLimit::kraken_futures_derivatives();
        let (_clock, lim) = manual(limit.clone());
        // Capacity 500, cost 10 ⇒ exactly 50 requests before the bucket is empty.
        let burst = limit.capacity / limit.cost_per_request;
        for i in 0..burst {
            assert!(
                lim.try_acquire().is_ok(),
                "request {i} of the documented burst must be permitted"
            );
        }
        let denied = lim
            .try_acquire()
            .expect_err("request past the declared budget MUST be refused");
        assert_eq!(denied.available, 0);
        assert_eq!(denied.needed, limit.cost_per_request);
        assert!(denied.retry_after <= limit.refill_window);
        // And it keeps refusing while time stands still: a denial is not a retry
        // loop that eventually wears the limiter down.
        for _ in 0..1_000 {
            assert!(
                lim.try_acquire().is_err(),
                "time has not moved; the limiter must keep refusing"
            );
        }
        let (granted, refused) = lim.counters();
        assert_eq!(
            granted, burst,
            "exactly the documented burst was ever granted"
        );
        assert_eq!(refused, 1_001);
    }

    /// Sustained: over a long virtual run the limiter never lets the observed rate
    /// exceed the documented one. This is the property a `sleep` cannot prove.
    #[test]
    fn sustained_rate_never_exceeds_the_documented_rate() {
        let limit = VenueLimit::kraken_futures_history();
        let (clock, lim) = manual(limit.clone());
        // Hammer it every second for a simulated day, taking whatever it grants.
        let seconds = 86_400u64;
        let mut granted = 0u64;
        for _ in 0..seconds {
            if lim.try_acquire().is_ok() {
                granted += 1;
            }
            clock.advance(Duration::from_secs(1));
        }
        // 100 tokens per 10 minutes = 144 windows a day = 14,400, plus the initial
        // full bucket of 100.
        let ceiling =
            limit.capacity + limit.refill_tokens * (seconds / limit.refill_window.as_secs());
        assert!(
            granted <= ceiling,
            "granted {granted} exceeds the documented ceiling {ceiling}"
        );
        let observed_rps = granted as f64 / seconds as f64;
        assert!(
            observed_rps <= limit.sustained_rps() * 1.01,
            "observed {observed_rps}/s exceeds documented {}/s",
            limit.sustained_rps()
        );
    }

    #[test]
    fn budget_replenishes_only_with_time() {
        let limit = VenueLimit::tradier_market_data();
        let (clock, lim) = manual(limit.clone());
        for _ in 0..limit.capacity {
            lim.try_acquire().unwrap();
        }
        assert!(lim.try_acquire().is_err());
        // Nine tenths of a window buys nothing: a partial window may not be rounded
        // up into free budget.
        clock.advance(Duration::from_secs(54));
        assert!(
            lim.try_acquire().is_err(),
            "a partial refill window grants no tokens"
        );
        clock.advance(Duration::from_secs(6));
        assert!(lim.try_acquire().is_ok());
    }

    #[test]
    fn the_bucket_never_exceeds_its_capacity() {
        let limit = VenueLimit::tradier_sandbox();
        let (clock, lim) = manual(limit.clone());
        clock.advance(Duration::from_secs(60 * 60 * 24 * 30));
        assert_eq!(
            lim.available(),
            limit.capacity,
            "idling for a month does not bank a month of burst"
        );
    }

    /// Adversarial: a caller that asks for more than the bucket could *ever* hold
    /// is refused rather than served by draining it — otherwise one oversized
    /// request would starve every well-behaved one.
    #[test]
    fn a_request_larger_than_capacity_is_refused_not_partially_served() {
        let limit = VenueLimit::tradier_market_data();
        let (_clock, lim) = manual(limit.clone());
        let before = lim.available();
        let denied = lim
            .try_acquire_n(limit.capacity + 1)
            .expect_err("must refuse");
        assert_eq!(denied.available, before, "a refused request spends nothing");
        assert_eq!(lim.available(), before);
    }

    /// P-3.2 acceptance: the limiter **refuses** above ten requests a second.
    /// The criterion is refusal, not that ten succeed — a limiter that slows down
    /// is a limiter nobody can assert anything about.
    #[test]
    fn the_edgar_limiter_refuses_above_ten_requests_a_second() {
        let limit = VenueLimit::sec_edgar();
        let (clock, lim) = manual(limit.clone());
        for i in 0..10 {
            assert!(
                lim.try_acquire().is_ok(),
                "request {i} is inside the SEC's published rate"
            );
        }
        // Still inside the same second.
        clock.advance(Duration::from_millis(999));
        let denied = lim
            .try_acquire()
            .expect_err("the eleventh request in one second MUST be refused");
        assert_eq!(denied.available, 0);
        clock.advance(Duration::from_millis(1));
        assert!(
            lim.try_acquire().is_ok(),
            "the next second buys the next ten"
        );
        assert!((limit.sustained_rps() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn declared_limits_quote_their_source() {
        for l in [
            VenueLimit::kraken_futures_derivatives(),
            VenueLimit::kraken_futures_history(),
            VenueLimit::tradier_market_data(),
            VenueLimit::tradier_sandbox(),
            VenueLimit::sec_edgar(),
            VenueLimit::fred_api(),
            VenueLimit::alpaca_market_data(),
        ] {
            assert!(
                !l.source_quote.is_empty(),
                "{} declares a limit with no documented source",
                l.venue
            );
            assert!(l.capacity > 0 && l.refill_tokens > 0 && l.cost_per_request > 0);
            assert!(l.sustained_rps() > 0.0);
        }
    }

    /// A budget nobody published must say that it is self-imposed. Writing an
    /// unverified number into `source_quote` is worse than declaring none: it
    /// invites the next reader to trust a figure that was never quoted.
    #[test]
    fn a_self_imposed_budget_declares_itself_self_imposed() {
        for l in [VenueLimit::fred_api(), VenueLimit::alpaca_market_data()] {
            assert!(
                l.source_quote.contains("self-imposed"),
                "{} declares an unquoted budget without saying so",
                l.venue
            );
        }
        assert!(
            !VenueLimit::sec_edgar()
                .source_quote
                .contains("self-imposed"),
            "the SEC publishes its ceiling; this one is a real quote"
        );
    }

    #[test]
    fn sustained_rps_matches_the_documented_arithmetic() {
        // 120 requests per minute is 2/s.
        assert!((VenueLimit::tradier_market_data().sustained_rps() - 2.0).abs() < 1e-9);
        // 100 tokens per 10 minutes at 1 token a request is one request every 6s.
        assert!(
            (VenueLimit::kraken_futures_history().sustained_rps() - 100.0 / 600.0).abs() < 1e-9
        );
    }
}
