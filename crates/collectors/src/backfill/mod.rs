//! Deep-history backfill adapters (DATA-005 §11 S1; Set P P-1.2…P-1.4).
//!
//! Three venues, one contract (`dataplane::backfill::BackfillSource`). What
//! these modules have in common is what makes a backfill safe rather than fast:
//!
//! * **A limiter that refuses.** Every adapter holds a
//!   [`dataplane::backfill::RateLimiter`] over its *declared* limit and takes a
//!   permit before every request. The limiter's decision is a pure function of
//!   injected time, so the property "it will not exceed the rate" is a test, not
//!   a stopwatch reading. [`Limited::permit`] turns a refusal into a wait; the
//!   refusal is still the thing being decided.
//! * **No `f64` anywhere near a price.** Venue JSON carries prices as unquoted
//!   numbers, and `serde_json::Value` would round-trip them through `f64`.
//!   Every parser here reads them through [`serde_json::value::RawValue`], so
//!   what reaches `BarStore` is the venue's own digits (SC-1, INV-05).
//! * **A source name that reaches the row.** Each adapter returns its
//!   `dataplane.source` name, which is what selects the declared vendor lag
//!   (Coinbase/Kraken 60 s, Alpaca 900 s, ADR-P2-24). The lag is never passed
//!   alongside the bars and never defaulted — if it were, two sources could
//!   silently stamp the same `knowledge_time` and the history would claim to
//!   have been knowable earlier than it was.
//!
//! The parsers are free functions taking bytes, and the HTTP layer is a trait,
//! so every adapter is testable without a network and without a venue.

pub mod alpaca;
pub mod coinbase;
pub mod kraken;

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use async_trait::async_trait;
use dataplane::backfill::{FetchError, RateLimit, RateLimiter};

// ── HTTP, behind a trait ─────────────────────────────────────────────────────

/// What an adapter needs from the network, and nothing else.
#[async_trait]
pub trait HttpGet: Send + Sync {
    async fn get(&self, url: &str) -> Result<HttpResponse, FetchError>;
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    /// Parsed from `Retry-After` when the venue sent one. The venue's own number
    /// outranks any backoff we would compute.
    pub retry_after_ms: Option<u64>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Turn an HTTP status into the contract's error, or hand back the body.
    ///
    /// # Errors
    /// `429`/`5xx` become pauses; `4xx` other than 429 become transport errors
    /// (a 403 from a venue is usually a geo-block, which is a real terms
    /// question, so it is surfaced rather than retried forever).
    pub fn ok_body(self, venue: &str) -> Result<Vec<u8>, FetchError> {
        match self.status {
            200..=299 => Ok(self.body),
            429 => Err(FetchError::RateLimited {
                venue: venue.to_string(),
                retry_after_ms: self.retry_after_ms.unwrap_or(1_000),
            }),
            403 | 451 => Err(FetchError::Terms(format!(
                "{venue} answered {} — access is refused for this client or region",
                self.status
            ))),
            500..=599 => Err(FetchError::Transport(format!(
                "{venue} answered {}",
                self.status
            ))),
            other => Err(FetchError::Transport(format!("{venue} answered {other}"))),
        }
    }
}

/// The production [`HttpGet`], over `reqwest`.
pub struct ReqwestGet {
    client: reqwest::Client,
    /// Sent on every request. Several free endpoints (EDGAR explicitly) require
    /// a declared, contactable User-Agent, and sending a library default is how
    /// a platform gets blocked without being told why.
    user_agent: String,
}

impl ReqwestGet {
    #[must_use]
    pub fn new(user_agent: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            user_agent: user_agent.into(),
        }
    }
}

#[async_trait]
impl HttpGet for ReqwestGet {
    async fn get(&self, url: &str) -> Result<HttpResponse, FetchError> {
        let resp = self
            .client
            .get(url)
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after_ms = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(|secs| secs.saturating_mul(1_000));
        let body = resp
            .bytes()
            .await
            .map_err(|e| FetchError::Transport(e.to_string()))?
            .to_vec();
        Ok(HttpResponse {
            status,
            retry_after_ms,
            body,
        })
    }
}

// ── the limiter, wired to a real clock ───────────────────────────────────────

/// A declared rate limit plus the monotonic clock it is measured against.
///
/// The clock is a process-start `Instant`, not wall time: a backfill that runs
/// across an NTP correction must not be handed a windfall of permits because
/// the wall clock jumped forward, and must not stall for an hour because it
/// jumped back.
pub struct Limited {
    limiter: RateLimiter,
    origin: Instant,
    /// Monotonic ms of the last permit, for diagnostics only.
    last_ms: AtomicI64,
}

impl std::fmt::Debug for Limited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Limited")
            .field("limit", &self.limiter.limit())
            .finish_non_exhaustive()
    }
}

impl Limited {
    #[must_use]
    pub fn new(limit: RateLimit) -> Self {
        Self {
            limiter: RateLimiter::new(limit),
            origin: Instant::now(),
            last_ms: AtomicI64::new(0),
        }
    }

    #[must_use]
    pub const fn limit(&self) -> &RateLimiter {
        &self.limiter
    }

    fn now_ms(&self) -> i64 {
        i64::try_from(self.origin.elapsed().as_millis()).unwrap_or(i64::MAX)
    }

    /// Wait until a permit is available, then take it.
    ///
    /// The waiting is the *consequence* of a refusal, never a substitute for
    /// one: each iteration asks the limiter, and sleeps only for exactly as long
    /// as the limiter says it must. A `sleep(spacing)` before each request would
    /// look the same on a quiet day and exceed the rate the moment two tasks
    /// share the adapter.
    pub async fn permit(&self) {
        loop {
            match self.limiter.try_acquire_at(self.now_ms()) {
                Ok(()) => {
                    self.last_ms.store(self.now_ms(), Ordering::Relaxed);
                    return;
                }
                Err(refused) => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        refused.retry_after_ms.max(1),
                    ))
                    .await;
                }
            }
        }
    }

    /// Take a permit or give up immediately, for callers that would rather
    /// report a pause than block a worker slot.
    ///
    /// # Errors
    /// [`FetchError::RateLimited`] carrying our own limiter's wait.
    pub fn try_permit(&self, venue: &str) -> Result<(), FetchError> {
        self.limiter
            .try_acquire_at(self.now_ms())
            .map_err(|r| FetchError::RateLimited {
                venue: venue.to_string(),
                retry_after_ms: r.retry_after_ms,
            })
    }
}

// ── shared parsing helpers ───────────────────────────────────────────────────

// ── paging arithmetic, in one place ──────────────────────────────────────────

/// How much bar-close time one page may cover, given a venue's own row cap.
///
/// **`cap − 1`, not `cap`, and the missing row is the whole point.** Every one of
/// these venues bounds a page by a timestamp, and whether that bound is inclusive
/// is not something this repository can settle without calling the live endpoint.
/// Asking for a span of exactly `cap` periods asks for `cap + 1` rows under the
/// inclusive reading, and what a venue does with an over-long request is its own
/// business: Coinbase answers an error, Alpaca truncates — and *which end* it
/// truncates decides whether the page silently loses its newest row. One row per
/// page lost is 144 fabricated holes across a 30-day 1m window, indistinguishable
/// from venue outages and unrecoverable once the cursor has passed them.
///
/// A span one period short is correct under **both** readings: inclusive gives
/// exactly `cap` rows with a one-row overlap into the next page, which the sink
/// records as `unchanged`; exclusive gives `cap − 1` rows and the next page picks
/// up the one left. Neither leaves a hole, and neither exceeds the cap. The cost
/// is one extra request per `cap` pages.
#[must_use]
pub const fn page_span_secs(period_secs: u32, venue_row_cap: i64) -> i64 {
    let rows = if venue_row_cap > 1 {
        venue_row_cap - 1
    } else {
        1
    };
    period_secs as i64 * rows
}

/// A venue number as its own digits.
///
/// `RawValue::get()` hands back the exact bytes the venue sent. Quoted strings
/// are unquoted; everything else is passed through. Nothing here ever becomes an
/// `f64`, which is the single reason this function exists rather than
/// `value.as_f64().to_string()`.
///
/// # Errors
/// A value that is not a number or a numeric string.
pub fn raw_number(v: &serde_json::value::RawValue) -> Result<String, FetchError> {
    let s = v.get().trim();
    let s = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    if s.is_empty() || s == "null" {
        return Err(FetchError::Integrity(
            "a price or volume field was null".into(),
        ));
    }
    let mut chars = s.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.');
    if !first_ok
        || !s
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E'))
    {
        return Err(FetchError::Integrity(format!("not a number: {s:?}")));
    }
    if s.contains(['e', 'E']) {
        // `BarStore::numeric` refuses exponent notation (it is also the shape an
        // injection attempt takes), and a bar it refuses is silently counted as
        // `rejected` — a one-bar hole nobody attributes. Low-priced tokens do
        // arrive as `1.5e-9`, so the exponent is expanded here, through
        // `Decimal` rather than `f64`, and a value `Decimal` cannot hold is an
        // explicit integrity failure instead of a dropped row.
        let d: rust_decimal::Decimal = s.parse().map_err(|_| {
            FetchError::Integrity(format!(
                "exponent-notation value {s:?} does not fit a decimal price"
            ))
        })?;
        return Ok(d.normalize().to_string());
    }
    Ok(s.to_string())
}

/// An integer field (trade counts), which may arrive quoted.
///
/// # Errors
/// A value that is not an integer.
pub fn raw_u64(v: &serde_json::value::RawValue) -> Result<u64, FetchError> {
    let s = raw_number(v)?;
    // Some venues report counts as `12.0`.
    let s = s.split_once('.').map_or(s.as_str(), |(i, _)| i);
    s.parse::<u64>()
        .map_err(|_| FetchError::Integrity(format!("not a count: {s:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::value::RawValue;

    fn raw(s: &str) -> Box<RawValue> {
        RawValue::from_string(s.to_string()).unwrap()
    }

    /// The property that keeps SC-1 true across the venue boundary: the digits
    /// that reach the store are the digits the venue sent, including ones `f64`
    /// cannot represent.
    #[test]
    fn venue_numbers_keep_every_digit_they_arrived_with() {
        let exact = "74054.123456789012345678";
        assert_eq!(raw_number(&raw(exact)).unwrap(), exact);
        // The f64 round-trip that this function exists to avoid.
        assert_ne!(exact.parse::<f64>().unwrap().to_string(), exact);
        assert_eq!(raw_number(&raw("\"0.00000001\"")).unwrap(), "0.00000001");
        // Exponent notation is expanded rather than passed through: the store
        // refuses `1e5`-shaped text, and a refused bar is a silent hole.
        assert_eq!(raw_number(&raw("-1.5e-9")).unwrap(), "-0.0000000015");
        assert_eq!(raw_number(&raw("1E5")).unwrap(), "100000");
    }

    #[test]
    fn a_non_numeric_field_is_an_integrity_failure_not_a_zero() {
        assert!(matches!(
            raw_number(&raw("null")),
            Err(FetchError::Integrity(_))
        ));
        assert!(matches!(
            raw_number(&raw("\"n/a\"")),
            Err(FetchError::Integrity(_))
        ));
        assert!(matches!(
            raw_number(&raw("\"\"")),
            Err(FetchError::Integrity(_))
        ));
        assert_eq!(raw_u64(&raw("12.0")).unwrap(), 12);
    }

    #[test]
    fn a_429_becomes_a_pause_carrying_the_venue_s_own_retry_after() {
        let r = HttpResponse {
            status: 429,
            retry_after_ms: Some(30_000),
            body: Vec::new(),
        };
        match r.ok_body("coinbase") {
            Err(FetchError::RateLimited { retry_after_ms, .. }) => {
                assert_eq!(retry_after_ms, 30_000)
            }
            other => panic!("{other:?}"),
        }
    }

    /// A geo-block is a terms question, not a transient error. Retrying it on a
    /// backoff forever is how a source nobody can use looks healthy.
    #[test]
    fn a_451_or_403_is_terminal_rather_than_retried() {
        for status in [403_u16, 451] {
            let r = HttpResponse {
                status,
                retry_after_ms: None,
                body: Vec::new(),
            };
            assert!(
                matches!(r.ok_body("binance"), Err(FetchError::Terms(_))),
                "status {status}"
            );
        }
    }

    /// The adversarial limiter test at the adapter boundary: asking for more
    /// permits than the declared rate allows, at one instant, is refused.
    #[test]
    fn the_adapter_limiter_refuses_above_its_declared_rate() {
        let l = Limited::new(RateLimit::per_second(3));
        let now = l.now_ms();
        for _ in 0..3 {
            assert!(l.limit().try_acquire_at(now).is_ok());
        }
        assert!(
            l.limit().try_acquire_at(now).is_err(),
            "the 4th request in one instant under a 3/s limit must be refused"
        );
        assert!(matches!(
            l.try_permit("coinbase"),
            Err(FetchError::RateLimited { .. })
        ));
    }
}
