//! The deep-backfill contract: what a historical source is, what a page of it
//! looks like, how fast it may be asked, and where the cursor lives
//! (DATA-005 §11 S1, §12; DA-11; Set P P-1.1).
//!
//! Three ideas carry the design, and each exists because of a specific way a
//! backfill goes wrong:
//!
//! 1. **The limiter refuses; it does not sleep.** A backfill that gets the
//!    platform banned from a free endpoint costs more than the history it
//!    fetched, and a `sleep` in a loop is not a rate limit — it is a hope. The
//!    limiter here is a pure token bucket over injected time, so a test can
//!    drive it past its declared rate and observe a *refusal* rather than
//!    measure a wall-clock delay. The async wrapper that turns a refusal into a
//!    wait lives beside the adapters that need tokio; the decision does not.
//! 2. **A 429 is a pause, not a failure.** They look the same in a status
//!    column and they mean opposite things: a failed job is something to fix, a
//!    paused one is something to wait for. [`BackfillStatus::Paused`] with a
//!    recorded `backoff_until` is the difference.
//! 3. **The cursor is durable and the window is closed-open.** A killed worker
//!    resumes at the cursor and refetches nothing; a completed window refetches
//!    nothing at all.
//!
//! Prices cross this boundary as decimal strings, never `f64` (SC-1). The
//! adapters parse venue JSON, which is where a float would otherwise creep in.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── what a backfilled bar is ─────────────────────────────────────────────────

/// One historical bar as a source reported it.
///
/// `close_time` is the bar CLOSE, matching `BarStore::insert_collected`'s
/// `CollectedBar::available_time`. Venues publish opens, closes or both; the
/// adapter converts, and the conversion is the adapter's single most
/// consequential line — an off-by-one-period here shifts the entire series.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillBar {
    pub close_time: DateTime<Utc>,
    /// Decimal strings. Never floats — these are prices (SC-1).
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub trade_count: u64,
}

/// One request for a page of history.
///
/// The window is half-open, `[start, end)`, in bar-close time. Adapters page
/// newest-to-oldest or oldest-to-newest as their venue requires; the engine only
/// needs to know what was covered, which is what [`BackfillPage::covered_to`]
/// reports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageRequest {
    pub venue: String,
    pub symbol: String,
    /// `"1m"`, `"1h"`, `"1d"` — the platform's timeframe label.
    pub timeframe: String,
    pub period_secs: u32,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// A page of history, plus how far through the window it got.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillPage {
    pub bars: Vec<BackfillBar>,
    /// The exclusive high-water mark this page reached in bar-close time: the
    /// next page starts here. `None` means the source has nothing further in
    /// this window, whether or not it returned bars — an empty page with
    /// `covered_to: Some(t)` is a venue gap, not the end of the window, and the
    /// two must stay distinguishable or a gap silently terminates the job.
    pub covered_to: Option<DateTime<Utc>>,
}

impl BackfillPage {
    #[must_use]
    pub fn end_of_window() -> Self {
        Self {
            bars: Vec::new(),
            covered_to: None,
        }
    }
}

/// Why a page could not be fetched.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum FetchError {
    /// The venue (or our own limiter) refused. **Not a failure**: the job pauses
    /// with this backoff and resumes, and the row says so.
    #[error("rate limited by {venue}; retry after {retry_after_ms} ms")]
    RateLimited {
        /// The venue or limiter that refused. Named `venue`, not `source`:
        /// `thiserror` gives a field called `source` error-chain meaning.
        venue: String,
        retry_after_ms: u64,
    },
    /// The bytes arrived but do not verify — a checksum mismatch on a history
    /// archive, a truncated file, a malformed row. Never partially inserted:
    /// the job pauses so a human can look, because inserting half an archive is
    /// worse than inserting none of it.
    #[error("integrity: {0}")]
    Integrity(String),
    /// Network or HTTP failure worth retrying.
    #[error("transport: {0}")]
    Transport(String),
    /// The source's terms forbid this use. Terminal, and loud: the platform
    /// does not build against terms (Set P MASTER §5).
    #[error("terms: {0}")]
    Terms(String),
}

impl FetchError {
    /// Whether this error pauses the job rather than failing it.
    #[must_use]
    pub const fn is_pause(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::Integrity(_) | Self::Transport(_)
        )
    }

    /// The backoff a pause should record, if the error names one.
    #[must_use]
    pub const fn retry_after_ms(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after_ms, .. } => Some(*retry_after_ms),
            _ => None,
        }
    }
}

/// A historical bar source. Implemented once per venue in `collectors::backfill`.
#[async_trait::async_trait]
pub trait BackfillSource: Send + Sync {
    /// The `dataplane.source` name whose declared vendor lag stamps every row
    /// this adapter writes. It is returned rather than assumed because the lag
    /// differs per source (Coinbase 60 s, Alpaca 900 s, ADR-P2-24) and assuming
    /// them equal is the bug this contract exists to prevent.
    fn source_name(&self) -> &str;

    /// The declared rate limit, for the record and for the census.
    fn declared_limit(&self) -> RateLimit;

    /// The most bar-close time one request can carry at `period_secs`, when the
    /// venue caps a page by row count. The engine asks for exactly this much per
    /// page; without it every timeframe was fetched in the policy's fixed 5-hour
    /// slices, so a three-year daily window cost ~5,400 requests for ~1,100 bars
    /// (measured 2026-10-03) — wasteful against a free endpoint whose goodwill is
    /// the whole supply. `None` keeps the policy's span.
    fn max_page_span(&self, _period_secs: u32) -> Option<chrono::Duration> {
        None
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError>;
}

/// Where backfilled bars go.
///
/// The one implementation that matters is `BarStore` (`crates/backtest`), which
/// targets `market_bar` in ClickHouse, stamps `knowledge_time = bar close +
/// the source's declared vendor lag`, and sets `BACKFILLED_KNOWLEDGE_TIME`.
/// The engine holds this trait rather than the store so it can be driven by a
/// counting double in tests — the only way to prove "a restart fetches no page
/// twice" is to count, and you cannot count through a database.
///
/// `source` is passed through verbatim, never defaulted: it is what selects the
/// declared vendor lag, and a sink that picked its own would reintroduce exactly
/// the bug ADR-P2-24 exists to close.
#[async_trait::async_trait]
pub trait BarSink: Send + Sync {
    async fn write_page(
        &self,
        source: &str,
        venue: &str,
        instrument: &str,
        timeframe: &str,
        bars: &[BackfillBar],
    ) -> Result<SinkReport, SinkError>;
}

/// What a page of writes did. `unchanged` is the number that makes a resumed or
/// overlapping window cheap rather than duplicated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinkReport {
    pub inserted: u64,
    pub restated: u64,
    pub unchanged: u64,
    pub rejected: u64,
}

impl SinkReport {
    /// Rows this page actually added to history.
    #[must_use]
    pub const fn written(&self) -> u64 {
        self.inserted + self.restated
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("bar sink: {0}")]
    Backend(String),
}

// ── the limiter ──────────────────────────────────────────────────────────────

/// A venue's published request limit, as the platform declares it.
///
/// Stated as "`per_window` requests per `window_ms`, bursting to `burst`",
/// which is how venues publish theirs. The declared value is deliberately at or
/// below the published one: the cost of being 10 % slow is an hour; the cost of
/// being 10 % fast is a ban from a free endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    pub per_window: u32,
    pub window_ms: u64,
    pub burst: u32,
}

impl RateLimit {
    /// `n` requests per second, no burst above `n`.
    #[must_use]
    pub const fn per_second(n: u32) -> Self {
        Self {
            per_window: n,
            window_ms: 1_000,
            burst: n,
        }
    }

    /// The minimum spacing between two requests, in milliseconds.
    #[must_use]
    pub const fn spacing_ms(&self) -> u64 {
        if self.per_window == 0 {
            u64::MAX
        } else {
            self.window_ms / self.per_window as u64
        }
    }
}

/// The limiter refused; try again in this many milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("rate limit reached; {retry_after_ms} ms until the next permit")]
pub struct Refused {
    pub retry_after_ms: u64,
}

/// A token bucket over *injected* monotonic time.
///
/// Time is a parameter, not a call to the clock, for one reason: a test that
/// proves a limiter honours its rate must be able to ask for a thousand permits
/// at the same instant and see the thousand-and-first refused. A limiter that
/// reads the clock internally can only be tested by sleeping, and a test that
/// sleeps proves the machine was slow, not that the limiter refuses.
///
/// Tokens are held in micro-units so the refill arithmetic is exact integer
/// division rather than accumulating float error over an eight-hour backfill.
#[derive(Debug)]
pub struct RateLimiter {
    limit: RateLimit,
    state: Mutex<Bucket>,
}

const MICRO: u64 = 1_000_000;

#[derive(Debug)]
struct Bucket {
    /// Micro-tokens available.
    tokens: u64,
    /// Monotonic milliseconds at which `tokens` was last correct.
    last_ms: i64,
    /// Total permits ever granted. Test and telemetry surface.
    granted: u64,
}

impl RateLimiter {
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self {
            state: Mutex::new(Bucket {
                tokens: u64::from(limit.burst) * MICRO,
                last_ms: 0,
                granted: 0,
            }),
            limit,
        }
    }

    #[must_use]
    pub const fn limit(&self) -> RateLimit {
        self.limit
    }

    #[must_use]
    pub fn granted(&self) -> u64 {
        self.state.lock().map_or(0, |b| b.granted)
    }

    /// Take one permit at monotonic time `now_ms`, or refuse and say when.
    ///
    /// # Errors
    /// [`Refused`] when the bucket is empty, carrying the wait until one token
    /// has refilled. The caller decides whether to wait; the limiter never does.
    pub fn try_acquire_at(&self, now_ms: i64) -> Result<(), Refused> {
        let mut b = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.limit.per_window == 0 || self.limit.window_ms == 0 {
            return Err(Refused {
                retry_after_ms: u64::MAX,
            });
        }
        // Refill for elapsed time. Non-monotonic input cannot mint tokens.
        let elapsed_ms = u64::try_from(now_ms.saturating_sub(b.last_ms).max(0)).unwrap_or(0);
        if elapsed_ms > 0 {
            let refill = elapsed_ms
                .saturating_mul(u64::from(self.limit.per_window))
                .saturating_mul(MICRO)
                / self.limit.window_ms;
            b.tokens = b
                .tokens
                .saturating_add(refill)
                .min(u64::from(self.limit.burst) * MICRO);
            b.last_ms = now_ms;
        }
        if b.tokens >= MICRO {
            b.tokens -= MICRO;
            b.granted += 1;
            return Ok(());
        }
        let missing = MICRO - b.tokens;
        // ceil(missing / rate_per_ms) where rate_per_ms = per_window*MICRO/window_ms
        let numer = missing.saturating_mul(self.limit.window_ms);
        let denom = u64::from(self.limit.per_window).saturating_mul(MICRO);
        Err(Refused {
            retry_after_ms: numer.div_ceil(denom).max(1),
        })
    }
}

// ── the durable cursor ───────────────────────────────────────────────────────

/// The lifecycle of one `(source, instrument, timeframe, window)` backfill.
///
/// `Paused` and `Failed` are deliberately distinct states rather than one
/// "not running": a paused job is waiting for a venue's clock and needs nobody;
/// a failed job needs a person. Collapsing them is how a rate-limited backfill
/// becomes an incident report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackfillStatus {
    Pending,
    Running,
    /// Rate-limited, transiently unavailable, or an archive that did not verify.
    /// Resumes on its own once `backoff_until` passes.
    Paused,
    Completed,
    /// Terminal and human-facing: bad terms, an unknown instrument, a source
    /// that will never serve this window.
    Failed,
}

impl BackfillStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => return None,
        })
    }
}

/// The identity of a backfill window. Also the primary key of `backfill_state`
/// and the content of the job manifest, so the two cannot drift apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BackfillKey {
    pub source: String,
    pub venue: String,
    pub instrument: String,
    pub timeframe: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
}

/// One row of `backfill_state` (migration 0099).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillState {
    pub key: BackfillKey,
    /// Exclusive high-water mark in bar-close time. `None` = nothing fetched
    /// yet; resume starts at `window_start`.
    pub cursor: Option<DateTime<Utc>>,
    pub status: BackfillStatus,
    pub attempts: i32,
    pub backoff_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub pages_fetched: i64,
    pub rows_written: i64,
    pub updated_at: DateTime<Utc>,
}

impl BackfillState {
    /// Where the next page starts.
    #[must_use]
    pub fn resume_from(&self) -> DateTime<Utc> {
        self.cursor.unwrap_or(self.key.window_start)
    }
}

/// Durable cursor storage. Postgres in production
/// (`storage::backfill_state::PgBackfillState`); an in-memory double in tests.
#[async_trait::async_trait]
pub trait BackfillStateStore: Send + Sync {
    /// Read the row, if the window has ever been submitted.
    async fn load(&self, key: &BackfillKey) -> Result<Option<BackfillState>, StateError>;

    /// Create the row if absent; return it either way. Never resets a row that
    /// already exists — that is what makes resubmission free.
    async fn ensure(&self, key: &BackfillKey) -> Result<BackfillState, StateError>;

    /// Record progress after a page landed.
    ///
    /// `rows_written` is **this page's** count, and the store adds it to the
    /// row's total. Not the run's running total: that restarts at zero after a
    /// restart, and summing it page by page records a triangle number.
    async fn advance(
        &self,
        key: &BackfillKey,
        cursor: DateTime<Utc>,
        rows_written: i64,
    ) -> Result<(), StateError>;

    /// Record a pause with its backoff. Increments `attempts`; does **not** mark
    /// the job failed.
    async fn pause(
        &self,
        key: &BackfillKey,
        until: DateTime<Utc>,
        reason: &str,
    ) -> Result<(), StateError>;

    async fn complete(&self, key: &BackfillKey) -> Result<(), StateError>;

    async fn fail(&self, key: &BackfillKey, reason: &str) -> Result<(), StateError>;

    /// Mark the row claimed by a worker.
    async fn start(&self, key: &BackfillKey) -> Result<(), StateError>;
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("backfill state backend: {0}")]
    Backend(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(n: u32) -> RateLimiter {
        RateLimiter::new(RateLimit::per_second(n))
    }

    #[test]
    fn the_limiter_refuses_rather_than_exceeding_its_declared_rate() {
        let l = limiter(10);
        // The burst is spent at t=0…
        for i in 0..10 {
            assert!(l.try_acquire_at(0).is_ok(), "permit {i} within burst");
        }
        // …and the eleventh is REFUSED, not delayed.
        let refused = l
            .try_acquire_at(0)
            .expect_err("the 11th permit in one instant must be refused");
        assert_eq!(refused.retry_after_ms, 100, "1/10 s until the next token");
    }

    /// The adversarial property: however a caller hammers it, the number of
    /// permits granted over any elapsed span never exceeds the declared rate
    /// plus the declared burst. This is the test that would fail if someone
    /// "fixed" the limiter by sleeping instead of refusing.
    #[test]
    fn granted_permits_never_exceed_the_declared_rate_over_any_span() {
        let limit = RateLimit {
            per_window: 3,
            window_ms: 1_000,
            burst: 3,
        };
        let l = RateLimiter::new(limit);
        // Ask 50 times per simulated millisecond for 10 simulated seconds.
        for ms in 0..10_000_i64 {
            for _ in 0..50 {
                let _ = l.try_acquire_at(ms);
            }
        }
        let elapsed_s = 10_u64;
        let ceiling = u64::from(limit.per_window) * elapsed_s + u64::from(limit.burst);
        assert!(
            l.granted() <= ceiling,
            "granted {} permits in {elapsed_s}s under a {}/s limit (ceiling {ceiling})",
            l.granted(),
            limit.per_window
        );
        assert!(
            l.granted() >= u64::from(limit.per_window) * (elapsed_s - 1),
            "and it is not simply refusing everything"
        );
    }

    #[test]
    fn a_clock_that_goes_backwards_cannot_mint_tokens() {
        let l = limiter(2);
        assert!(l.try_acquire_at(0).is_ok());
        assert!(l.try_acquire_at(0).is_ok());
        assert!(
            l.try_acquire_at(-10_000).is_err(),
            "rewinding the clock is not a refill"
        );
        assert!(l.try_acquire_at(0).is_err());
    }

    #[test]
    fn a_zero_rate_limit_refuses_everything() {
        let l = RateLimiter::new(RateLimit {
            per_window: 0,
            window_ms: 1_000,
            burst: 0,
        });
        assert!(l.try_acquire_at(0).is_err());
        assert!(l.try_acquire_at(1_000_000).is_err());
    }

    #[test]
    fn a_rate_limit_error_pauses_and_a_terms_violation_does_not() {
        let rl = FetchError::RateLimited {
            venue: "coinbase".into(),
            retry_after_ms: 1_500,
        };
        assert!(rl.is_pause());
        assert_eq!(rl.retry_after_ms(), Some(1_500));
        let terms = FetchError::Terms("history downloads are prohibited".into());
        assert!(!terms.is_pause(), "the platform never builds against terms");
        assert_eq!(terms.retry_after_ms(), None);
        assert!(
            FetchError::Integrity("sha256 mismatch".into()).is_pause(),
            "a bad archive waits for a human, it does not fail the window"
        );
    }

    #[test]
    fn status_round_trips_through_its_stored_form() {
        for s in [
            BackfillStatus::Pending,
            BackfillStatus::Running,
            BackfillStatus::Paused,
            BackfillStatus::Completed,
            BackfillStatus::Failed,
        ] {
            assert_eq!(BackfillStatus::parse(s.as_str()), Some(s));
        }
        assert_eq!(BackfillStatus::parse("stuck"), None);
    }
}
