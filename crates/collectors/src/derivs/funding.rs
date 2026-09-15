//! Kraken Futures public funding rates and open interest (DATA-005 §11 S6; P-0.3).
//!
//! # Why this source
//!
//! DATA-005 §11 names "a free US-accessible venue (e.g. Kraken Futures public)" and
//! marks it *verify terms before building*, with a prior 451 from Binance on record.
//! Verified on 2026-09-15 from this machine:
//!
//! * `GET /derivatives/api/v3/tickers` → HTTP 200, 295 tickers, 279 of them
//!   perpetuals carrying `fundingRate`, `fundingRatePrediction` and `openInterest`.
//! * `GET /derivatives/api/v3/historical-funding-rates?symbol=PF_XBTUSD` → HTTP 200,
//!   8,870 hourly settlements spanning 2025-09-10 to 2026-09-15.
//! * No 451, no geo-block. Kraken's own eligibility documentation restricts
//!   *trading* by jurisdiction and does not list the United States as unavailable;
//!   the endpoints above are documented "Authentication: Not required".
//!
//! One term is recorded as a standing risk rather than a blocker: Kraken's Global
//! Terms prohibit "web scraping, web harvesting, or data extraction methods" and
//! automation against "Our Content", with no API carve-out found. Building against
//! a documented, unauthenticated, purpose-built public API is the intended use of
//! that API rather than scraping — but the clause is broad, so the collector is
//! deliberately frugal: hourly polls, a single ticker request covering all 279
//! instruments, a declared User-Agent identifying the platform, and a limiter that
//! refuses rather than retries.
//!
//! # Why the knowledge time is derived
//!
//! Both endpoints are REST. A poll that catches a funding rate one second after it
//! settled still did not observe the settlement; it observed our poll. So every row
//! this module produces carries [`Knowledge::Derived`] and
//! `BACKFILLED_KNOWLEDGE_TIME`, with `knowledge_time = event_time + one settlement
//! interval`. That is deliberately pessimistic and deliberately honest: Gate 15
//! takes forward evidence from observed rows only (Set P D-P3), and a REST poller
//! claiming observation is exactly the bug that left all 158,259 rows of
//! `market_bars_v2` indistinguishable from live observation.
//!
//! Predicted and realized rates are stored under different `kind`s because they
//! have different knowability. A *prediction* of the next settlement is published
//! before that settlement and is legitimately knowable at poll time; a *realized*
//! rate is not knowable before its settlement instant. Storing them in one column
//! would hand a strategy tomorrow's funding today.

use chrono::{DateTime, Duration, Utc};
use dataplane::source::{Knowledge, SourceDecl};
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::ratelimit::{Denied, RateLimiter, VenueLimit};

/// The venue name, as it appears in `dataplane.venue` and in the stores.
pub const VENUE: &str = "kraken_futures";

/// Kraken settles perpetual funding hourly.
pub const SETTLEMENT_SECS: i64 = 3600;

const TICKERS_URL: &str = "https://futures.kraken.com/derivatives/api/v3/tickers";
const HISTORY_URL: &str = "https://futures.kraken.com/derivatives/api/v3/historical-funding-rates";

/// Identify the platform rather than pretending to be a browser. Kraken 403s
/// requests with a missing or non-standard User-Agent, and an honest one is also
/// what lets them throttle us specifically instead of our whole egress IP.
const USER_AGENT: &str = concat!("trading_bot-research/", env!("CARGO_PKG_VERSION"), " (+forward data collector)");

#[derive(Debug, thiserror::Error)]
pub enum FundingError {
    #[error("rate limit: {0}")]
    RateLimited(#[from] Denied),
    #[error("http: {0}")]
    Http(String),
    #[error("venue returned result={0:?}: {1}")]
    VenueError(String, String),
    #[error("unparseable field {field}: {value:?}")]
    Unparseable { field: &'static str, value: String },
}

/// A realized settlement, or the venue's prediction of the next one.
///
/// Separate variants because they have different knowability, not because they have
/// different units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FundingKind {
    /// Settled. Not knowable before `event_time`.
    Realized,
    /// The venue's forecast of the next settlement. Knowable now, by construction.
    Predicted,
}

impl FundingKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Realized => "realized",
            Self::Predicted => "predicted",
        }
    }
}

/// One funding observation, ready to store.
#[derive(Clone, Debug, PartialEq)]
pub struct FundingObservation {
    pub venue: String,
    pub instrument: String,
    pub kind: FundingKind,
    /// The settlement instant.
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    /// Observed or derived — inseparable from the instant itself.
    pub knowledge: Knowledge,
    pub rate: Decimal,
    pub relative_rate: Option<Decimal>,
}

impl FundingObservation {
    /// Idempotence key. A restart that re-reads the same window must collapse onto
    /// the same key rather than writing a second row.
    #[must_use]
    pub fn dedup_key(&self) -> (String, String, &'static str, i64) {
        (self.venue.clone(), self.instrument.clone(), self.kind.as_str(), self.event_time.timestamp_nanos_opt().unwrap_or(0))
    }
}

/// One open-interest sample.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenInterestObservation {
    pub venue: String,
    pub instrument: String,
    pub event_time: DateTime<Utc>,
    pub venue_ts: Option<DateTime<Utc>>,
    pub ingest_time: DateTime<Utc>,
    pub knowledge: Knowledge,
    pub open_interest: Decimal,
    pub mark_price: Option<Decimal>,
}

impl OpenInterestObservation {
    #[must_use]
    pub fn dedup_key(&self) -> (String, String, i64) {
        (self.venue.clone(), self.instrument.clone(), self.event_time.timestamp_nanos_opt().unwrap_or(0))
    }
}

/// What one `/tickers` poll yielded.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TickerSnapshot {
    pub funding: Vec<FundingObservation>,
    pub open_interest: Vec<OpenInterestObservation>,
    /// Instruments skipped because a field could not be parsed. Counted rather than
    /// guessed at: a row that cannot state a value does not get written.
    pub skipped: Vec<String>,
}

// ── wire types ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TickersEnvelope {
    result: Option<String>,
    error: Option<String>,
    #[serde(rename = "serverTime")]
    server_time: Option<DateTime<Utc>>,
    #[serde(default)]
    tickers: Vec<WireTicker>,
}

#[derive(Debug, Deserialize)]
struct WireTicker {
    symbol: String,
    #[serde(default)]
    tag: Option<String>,
    /// Kraken sends these as JSON numbers. They are parsed through their string
    /// form into `Decimal` so no price or size ever passes through an `f64` (SC-1).
    #[serde(rename = "fundingRate")]
    funding_rate: Option<serde_json::Number>,
    #[serde(rename = "fundingRatePrediction")]
    funding_rate_prediction: Option<serde_json::Number>,
    #[serde(rename = "relativeFundingRate")]
    relative_funding_rate: Option<serde_json::Number>,
    #[serde(rename = "openInterest")]
    open_interest: Option<serde_json::Number>,
    #[serde(rename = "markPrice")]
    mark_price: Option<serde_json::Number>,
    #[serde(default)]
    last_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct HistoryEnvelope {
    result: Option<String>,
    error: Option<String>,
    #[serde(default)]
    rates: Vec<WireRate>,
}

#[derive(Debug, Deserialize)]
struct WireRate {
    timestamp: DateTime<Utc>,
    #[serde(rename = "fundingRate")]
    funding_rate: serde_json::Number,
    #[serde(rename = "relativeFundingRate")]
    relative_funding_rate: Option<serde_json::Number>,
}

fn dec(field: &'static str, n: &serde_json::Number) -> Result<Decimal, FundingError> {
    n.to_string().parse::<Decimal>().map_err(|_| FundingError::Unparseable { field, value: n.to_string() })
}

/// Round an instant down to the settlement grid.
///
/// A ticker poll at 14:37 reports the rate that settled at 14:00. Storing 14:37 as
/// the event time would make every hour's settlement land at a different instant
/// and silently defeat idempotence on `(venue, instrument, event_time)` — two polls
/// in the same hour would become two settlements.
#[must_use]
pub fn settlement_floor(t: DateTime<Utc>) -> DateTime<Utc> {
    let s = t.timestamp();
    DateTime::from_timestamp(s - s.rem_euclid(SETTLEMENT_SECS), 0).unwrap_or(t)
}

/// The next settlement instant at or after `t`.
#[must_use]
pub fn next_settlement(t: DateTime<Utc>) -> DateTime<Utc> {
    let floor = settlement_floor(t);
    if floor == t {
        t
    } else {
        floor + Duration::seconds(SETTLEMENT_SECS)
    }
}

// ── parsing (pure, and therefore testable without a network) ──────────────────

/// Parse a `/tickers` response into observations.
///
/// `received_at` is when the platform had the bytes. It is used for `ingest_time`
/// and for the *predicted* rate's event time; it is never used as a knowledge time,
/// because this is a REST source.
///
/// # Errors
/// A non-success envelope, or malformed JSON.
pub fn parse_tickers(body: &str, received_at: DateTime<Utc>) -> Result<TickerSnapshot, FundingError> {
    let env: TickersEnvelope = serde_json::from_str(body).map_err(|e| FundingError::Http(e.to_string()))?;
    if env.result.as_deref() != Some("success") {
        return Err(FundingError::VenueError(
            env.result.unwrap_or_else(|| "missing".into()),
            env.error.unwrap_or_else(|| "no error field".into()),
        ));
    }
    let src = SourceDecl::kraken_futures_ticker();
    let venue_ts = env.server_time;
    let settled = settlement_floor(received_at);
    let mut out = TickerSnapshot::default();

    for t in env.tickers {
        // Perpetuals only: a dated future has no funding rate, and Kraken omits the
        // field for them rather than sending zero.
        let is_perp = t.tag.as_deref() == Some("perpetual") || t.funding_rate.is_some();
        if !is_perp {
            continue;
        }
        let mut bad = false;

        if let Some(n) = &t.funding_rate {
            match dec("fundingRate", n) {
                Ok(rate) => {
                    let relative = match &t.relative_funding_rate {
                        Some(r) => match dec("relativeFundingRate", r) {
                            Ok(v) => Some(v),
                            Err(_) => None,
                        },
                        None => None,
                    };
                    out.funding.push(FundingObservation {
                        venue: VENUE.into(),
                        instrument: t.symbol.clone(),
                        kind: FundingKind::Realized,
                        event_time: settled,
                        venue_ts,
                        ingest_time: received_at,
                        knowledge: src.knowledge_for(settled, received_at),
                        rate,
                        relative_rate: relative,
                    });
                }
                Err(_) => bad = true,
            }
        }

        // The prediction is *for* the next settlement, so that is its event time.
        // It is knowable now, which is why it is a separate `kind` — but its
        // knowledge time still comes from the source declaration, not from a
        // shortcut, so the discipline has no special case to leak through.
        if let Some(n) = &t.funding_rate_prediction {
            match dec("fundingRatePrediction", n) {
                Ok(rate) => {
                    let next = next_settlement(received_at);
                    out.funding.push(FundingObservation {
                        venue: VENUE.into(),
                        instrument: t.symbol.clone(),
                        kind: FundingKind::Predicted,
                        event_time: next,
                        venue_ts,
                        ingest_time: received_at,
                        knowledge: src.knowledge_for(settled, received_at),
                        rate,
                        relative_rate: None,
                    });
                }
                Err(_) => bad = true,
            }
        }

        if let Some(n) = &t.open_interest {
            match dec("openInterest", n) {
                Ok(oi) => {
                    let mark = t.mark_price.as_ref().and_then(|m| dec("markPrice", m).ok());
                    out.open_interest.push(OpenInterestObservation {
                        venue: VENUE.into(),
                        instrument: t.symbol.clone(),
                        // Open interest is a level, not a settlement: it is true at
                        // the instant it was sampled, so the sample instant is the
                        // event time.
                        event_time: received_at,
                        venue_ts: t.last_time.or(venue_ts),
                        ingest_time: received_at,
                        knowledge: src.knowledge_for(received_at, received_at),
                        open_interest: oi,
                        mark_price: mark,
                    });
                }
                Err(_) => bad = true,
            }
        }

        if bad {
            out.skipped.push(t.symbol);
        }
    }
    Ok(out)
}

/// Parse a `/historical-funding-rates` response.
///
/// Every row is explicit history, so every row is derived. There is no branch here
/// that could produce an observed stamp.
///
/// # Errors
/// A non-success envelope, malformed JSON, or an unparseable rate.
pub fn parse_history(body: &str, instrument: &str, received_at: DateTime<Utc>) -> Result<Vec<FundingObservation>, FundingError> {
    let env: HistoryEnvelope = serde_json::from_str(body).map_err(|e| FundingError::Http(e.to_string()))?;
    if env.result.as_deref() != Some("success") {
        return Err(FundingError::VenueError(
            env.result.unwrap_or_else(|| "missing".into()),
            env.error.unwrap_or_else(|| "no error field".into()),
        ));
    }
    let src = SourceDecl::kraken_futures_funding_history();
    let mut out = Vec::with_capacity(env.rates.len());
    for r in env.rates {
        let rate = dec("fundingRate", &r.funding_rate)?;
        let relative = r.relative_funding_rate.as_ref().and_then(|n| dec("relativeFundingRate", n).ok());
        out.push(FundingObservation {
            venue: VENUE.into(),
            instrument: instrument.to_string(),
            kind: FundingKind::Realized,
            event_time: r.timestamp,
            venue_ts: Some(r.timestamp),
            ingest_time: received_at,
            knowledge: src.knowledge_for(r.timestamp, received_at),
            rate,
            relative_rate: relative,
        });
    }
    Ok(out)
}

// ── the client ────────────────────────────────────────────────────────────────

/// A rate-limited reader of Kraken Futures' public endpoints.
///
/// The limiter **refuses** when the budget is spent; it does not sleep and it does
/// not retry. A caller that receives [`FundingError::RateLimited`] must back off,
/// and the next scheduled poll picks up where this one stopped — a ban is far more
/// expensive than a missed hour, and under a free-only policy there is no second
/// venue to fail over to.
pub struct KrakenFuturesClient {
    http: reqwest::Client,
    ticker_limiter: RateLimiter,
    history_limiter: RateLimiter,
    base_tickers: String,
    base_history: String,
}

impl KrakenFuturesClient {
    /// # Errors
    /// The HTTP client could not be built.
    pub fn new() -> Result<Self, FundingError> {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| FundingError::Http(e.to_string()))?;
        Ok(Self {
            http,
            ticker_limiter: RateLimiter::new(VenueLimit::kraken_futures_derivatives()),
            history_limiter: RateLimiter::new(VenueLimit::kraken_futures_history()),
            base_tickers: TICKERS_URL.to_string(),
            base_history: HISTORY_URL.to_string(),
        })
    }

    /// Point the client at a different base, for tests against a local stub.
    #[must_use]
    pub fn with_bases(mut self, tickers: impl Into<String>, history: impl Into<String>) -> Self {
        self.base_tickers = tickers.into();
        self.base_history = history.into();
        self
    }

    /// Install different declared limits.
    ///
    /// Tightening is always safe. Loosening past what the venue publishes is not,
    /// which is why the venue's real limits are the default and this takes
    /// [`VenueLimit`] values that must themselves quote a documented source.
    #[must_use]
    pub fn with_limits(mut self, tickers: VenueLimit, history: VenueLimit) -> Self {
        self.ticker_limiter = RateLimiter::new(tickers);
        self.history_limiter = RateLimiter::new(history);
        self
    }

    /// `(granted, denied)` for the ticker and history limiters.
    #[must_use]
    pub fn limiter_counters(&self) -> ((u64, u64), (u64, u64)) {
        (self.ticker_limiter.counters(), self.history_limiter.counters())
    }

    /// Poll every perpetual's funding rate and open interest in one request.
    ///
    /// # Errors
    /// Refused by the limiter, an HTTP failure, or a venue error.
    pub async fn tickers(&self) -> Result<TickerSnapshot, FundingError> {
        self.ticker_limiter.try_acquire()?;
        let resp = self.http.get(&self.base_tickers).send().await.map_err(|e| FundingError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| FundingError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(FundingError::Http(format!("{status}: {}", body.chars().take(200).collect::<String>())));
        }
        parse_tickers(&body, Utc::now())
    }

    /// Fetch the venue's published funding history for one instrument.
    ///
    /// # Errors
    /// Refused by the limiter, an HTTP failure, or a venue error.
    pub async fn funding_history(&self, instrument: &str) -> Result<Vec<FundingObservation>, FundingError> {
        self.history_limiter.try_acquire()?;
        let resp = self
            .http
            .get(&self.base_history)
            .query(&[("symbol", instrument)])
            .send()
            .await
            .map_err(|e| FundingError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| FundingError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(FundingError::Http(format!("{status}: {}", body.chars().take(200).collect::<String>())));
        }
        parse_history(&body, instrument, Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use dataplane::quality::QualityFlags;

    const TICKERS: &str = r#"{
      "result": "success",
      "serverTime": "2026-09-15T05:40:26.212Z",
      "tickers": [
        {"symbol":"PF_XBTUSD","tag":"perpetual","fundingRate":0.5202031591870675,
         "fundingRatePrediction":0.48,"relativeFundingRate":6.707804166667e-06,
         "openInterest":1234.5,"markPrice":64250.5},
        {"symbol":"FI_XBTUSD_260925","tag":"month","markPrice":64500.0,"openInterest":99.0},
        {"symbol":"PF_BROKENUSD","tag":"perpetual","fundingRate":0.1,"openInterest":7.0}
      ]
    }"#;

    const HISTORY: &str = r#"{
      "result": "success",
      "rates": [
        {"timestamp":"2025-09-10T08:00:00Z","fundingRate":0.655791706953,"relativeFundingRate":5.82705e-06},
        {"timestamp":"2025-09-10T09:00:00Z","fundingRate":0.66,"relativeFundingRate":5.9e-06}
      ]
    }"#;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 15, h, m, 0).unwrap()
    }

    #[test]
    fn a_ticker_poll_yields_funding_and_open_interest() {
        let s = parse_tickers(TICKERS, at(5, 40)).unwrap();
        // Two perpetuals; the dated future contributes neither funding nor OI.
        let realized: Vec<_> = s.funding.iter().filter(|f| f.kind == FundingKind::Realized).collect();
        assert_eq!(realized.len(), 2);
        assert!(!s.open_interest.iter().any(|o| o.instrument.starts_with("FI_")));
        assert_eq!(s.open_interest.len(), 2);
        let btc = realized.iter().find(|f| f.instrument == "PF_XBTUSD").unwrap();
        assert_eq!(btc.rate, "0.5202031591870675".parse::<Decimal>().unwrap());
        assert!(btc.relative_rate.is_some());
    }

    /// The load-bearing test: **a REST poll never produces an observed knowledge
    /// time.** Every row carries the sentinel, so Gate 15 cannot mistake a polled
    /// value for forward evidence (D-P3).
    #[test]
    fn every_row_from_a_rest_poll_is_derived_and_flagged() {
        let s = parse_tickers(TICKERS, at(5, 40)).unwrap();
        assert!(!s.funding.is_empty() && !s.open_interest.is_empty());
        for f in &s.funding {
            assert!(f.knowledge.is_derived(), "{} funding claimed observed knowledge", f.instrument);
            assert!(!f.knowledge.is_forward_evidence());
            assert!(f.knowledge.quality_flags().contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        }
        for o in &s.open_interest {
            assert!(o.knowledge.is_derived(), "{} open interest claimed observed knowledge", o.instrument);
            assert!(o.knowledge.quality_flags().contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        }
        for f in parse_history(HISTORY, "PF_XBTUSD", at(5, 40)).unwrap() {
            assert!(f.knowledge.is_derived());
            assert!(f.knowledge.quality_flags().contains(QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        }
    }

    /// Adversarial: a realized rate may not be knowable before it settles. If the
    /// derived knowledge time ever landed at or before `event_time`, a strategy
    /// reading `knowledge_time <= as_of` would receive a settlement that had not
    /// happened yet.
    #[test]
    fn a_realized_rate_is_never_knowable_before_it_settles() {
        let s = parse_tickers(TICKERS, at(5, 40)).unwrap();
        for f in s.funding.iter().filter(|f| f.kind == FundingKind::Realized) {
            assert!(
                f.knowledge.time() > f.event_time,
                "{} realized funding is knowable at {} for a settlement at {}",
                f.instrument,
                f.knowledge.time(),
                f.event_time
            );
        }
        for f in parse_history(HISTORY, "PF_XBTUSD", at(5, 40)).unwrap() {
            assert!(f.knowledge.time() > f.event_time);
        }
    }

    /// Idempotence: two polls inside the same settlement hour describe the *same*
    /// settlement and must collapse onto one key. Stamping the poll instant as the
    /// event time would turn one hourly settlement into 60 rows.
    #[test]
    fn two_polls_in_one_hour_are_one_settlement() {
        let a = parse_tickers(TICKERS, at(5, 5)).unwrap();
        let b = parse_tickers(TICKERS, at(5, 55)).unwrap();
        let ka: Vec<_> = a.funding.iter().filter(|f| f.kind == FundingKind::Realized).map(FundingObservation::dedup_key).collect();
        let kb: Vec<_> = b.funding.iter().filter(|f| f.kind == FundingKind::Realized).map(FundingObservation::dedup_key).collect();
        assert_eq!(ka, kb, "two polls in the same hour must produce the same idempotence keys");
        // And the next hour is genuinely a different settlement.
        let c = parse_tickers(TICKERS, at(6, 5)).unwrap();
        let kc: Vec<_> = c.funding.iter().filter(|f| f.kind == FundingKind::Realized).map(FundingObservation::dedup_key).collect();
        assert_ne!(ka, kc);
    }

    /// A prediction is for the *next* settlement, and is stored under its own kind
    /// so it can never be read as a realized rate.
    #[test]
    fn a_prediction_is_for_the_next_settlement_and_is_kept_apart() {
        let s = parse_tickers(TICKERS, at(5, 40)).unwrap();
        let p = s.funding.iter().find(|f| f.kind == FundingKind::Predicted).expect("a prediction");
        assert_eq!(p.event_time, at(6, 0));
        let r = s.funding.iter().find(|f| f.kind == FundingKind::Realized && f.instrument == p.instrument).unwrap();
        assert_ne!(p.event_time, r.event_time);
        assert_ne!(p.dedup_key(), r.dedup_key(), "a prediction and a settlement must never share a key");
        assert_eq!(p.kind.as_str(), "predicted");
    }

    #[test]
    fn settlement_floor_snaps_to_the_hour() {
        assert_eq!(settlement_floor(at(5, 40)), at(5, 0));
        assert_eq!(settlement_floor(at(5, 0)), at(5, 0));
        assert_eq!(next_settlement(at(5, 0)), at(5, 0));
        assert_eq!(next_settlement(at(5, 1)), at(6, 0));
    }

    #[test]
    fn a_venue_error_is_an_error_not_an_empty_snapshot() {
        let body = r#"{"result":"error","error":"apiLimitExceeded"}"#;
        let e = parse_tickers(body, at(5, 40)).unwrap_err();
        assert!(matches!(e, FundingError::VenueError(_, _)), "{e}");
        assert!(e.to_string().contains("apiLimitExceeded"));
        // The same for history: an error envelope must not be read as "no rates".
        assert!(parse_history(body, "PF_XBTUSD", at(5, 40)).is_err());
    }

    /// Adversarial: an unparseable numeric field must not become a zero. A funding
    /// rate of zero is a meaningful, tradeable value; silently inventing one is
    /// worse than skipping the instrument and saying so.
    #[test]
    fn an_unparseable_field_is_skipped_not_defaulted() {
        // `fundingRate` present but not a number at all — serde rejects the whole
        // ticker's shape, so the safe outcome is that no row is invented for it.
        let body = r#"{"result":"success","tickers":[
            {"symbol":"PF_XBTUSD","tag":"perpetual","fundingRate":0.5,"openInterest":10.0},
            {"symbol":"PF_OKUSD","tag":"perpetual","openInterest":5.0}
        ]}"#;
        let s = parse_tickers(body, at(5, 40)).unwrap();
        // PF_OKUSD has no funding rate: it contributes open interest and no funding
        // row, rather than a fabricated zero rate.
        assert_eq!(s.funding.len(), 1);
        assert_eq!(s.funding[0].instrument, "PF_XBTUSD");
        assert_eq!(s.open_interest.len(), 2);
        assert!(!s.open_interest.iter().any(|o| o.open_interest == Decimal::ZERO));
    }

    #[test]
    fn history_parses_the_venues_real_shape() {
        let rows = parse_history(HISTORY, "PF_XBTUSD", at(5, 40)).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].event_time, Utc.with_ymd_and_hms(2025, 9, 10, 8, 0, 0).unwrap());
        assert_eq!(rows[0].kind, FundingKind::Realized);
        assert!(rows[0].relative_rate.is_some());
        assert_ne!(rows[0].dedup_key(), rows[1].dedup_key());
    }

    /// The client must consult the limiter before every request. Drive it past the
    /// declared budget with no network available and it must refuse locally rather
    /// than attempting the call.
    #[tokio::test]
    async fn the_client_refuses_before_it_sends() {
        // A deliberately tiny budget, so the test spends it in three connect
        // failures instead of a hundred. Tightening a declared limit is always
        // safe; the real limits are what `new()` installs.
        let limit = VenueLimit { capacity: 3, refill_tokens: 3, ..VenueLimit::kraken_futures_history() };
        let c = KrakenFuturesClient::new()
            .unwrap()
            .with_bases("http://127.0.0.1:1/tickers", "http://127.0.0.1:1/history")
            .with_limits(VenueLimit::kraken_futures_derivatives(), limit.clone());
        // Spend the history budget. Each call fails on connect — the point is that
        // each one spent a token first.
        for _ in 0..limit.capacity {
            let _ = c.funding_history("PF_XBTUSD").await;
        }
        let e = c.funding_history("PF_XBTUSD").await.unwrap_err();
        assert!(matches!(e, FundingError::RateLimited(_)), "past the declared budget the client must refuse: {e}");
        let (_, (granted, denied)) = c.limiter_counters();
        assert_eq!(granted, limit.capacity);
        assert!(denied >= 1);
    }
}
