//! Kalshi history under the same PIT discipline as everything else
//! (DATA-005 §11 S7; Set P, P-3.4).
//!
//! The live collector (`super::kalshi`) polls `/markets/{ticker}` and stamps each
//! event with the instant it was observed. History is different: the
//! candlesticks endpoint returns closed periods, and a backfilled row's knowledge
//! time was never observed — it has to be *derived* from the period close plus
//! the source's declared lag, and flagged as derived (ADR-P2-24, D-P2).
//!
//! That flag is the whole point. A backfilled probability and an observed one
//! must be distinguishable, so that Gate 15's forward-test evidence can be drawn
//! from observed rows only (D-P3) and the sentinel never reaches money.
//!
//! **Exposure only.** Kalshi's trading surface is not referenced anywhere in
//! this module or in `super::kalshi`, and `tests/kalshi_exposure_only.rs` fails
//! the build if it appears — the list of forbidden paths lives there, not here,
//! so that naming them cannot trip the guard. Event probabilities are a readable
//! series, nothing more.
//!
//! ## Free-plan depth — not independently verified
//!
//! Kalshi's market-data endpoints are public and unauthenticated for reads. This
//! module does **not** claim an archive start date: the honest answer is that
//! depth is per-market and bounded by the market's own open date, which a census
//! job measures. Markets settle and disappear from the active listing, so a
//! series whose market has closed is only as deep as what was collected while it
//! existed — which is an argument for collecting forward, not an assumption that
//! backfill will fill it.

use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use dataplane::source::{Collection, CostClass, Knowledge, SourceDecl, SourceKind};
use rust_decimal::Decimal;
use serde::Deserialize;

/// The `dataplane.source.name` this reader writes under.
///
/// **Not the same row as `kalshi_rest`.** Migration 0046 row 13 is the live
/// quote poller, whose rows carry a real receipt; this is the candlestick
/// history endpoint, every row of which is derived. They share a venue and a
/// lag and nothing else, and collapsing them would make a backfilled
/// probability indistinguishable from an observed one (D-P3). The row for this
/// name does not exist yet — see the report.
pub const SOURCE_NAME: &str = "kalshi_history";

// No `SOURCE_ID` const. `dataplane.source` ids 1–19 are allocated (migrations 0046
// and 0098) and no row for `kalshi_history` exists, so a number here would be a
// guess. The writer resolves the id by name when the migration lands.

/// The declared lag for a candlestick: the period must close before it can be
/// read. The same 60 s that migration 0046 gives `kalshi_rest` (row 13).
pub const DECLARED_LAG_MS: i64 = 60_000;

/// This reader's source declaration.
///
/// [`SourceKind::RestHistory`] with a 60 s lag, and every row it produces is
/// [`Collection::Historical`] by construction — there is no call site in this
/// module that could claim a receipt, because no receipt exists for a period
/// that closed before anyone asked.
#[must_use]
pub const fn source() -> SourceDecl {
    SourceDecl {
        name: SOURCE_NAME,
        kind: SourceKind::RestHistory,
        declared_lag: Duration::from_millis(DECLARED_LAG_MS as u64),
        cost_class: CostClass::Free,
        notes: "candlesticks describe a closed period; the earliest a period's close is readable is one period after it",
    }
}

/// Kalshi quotes probabilities in whole cents, 1..=99.
const CENTS_PER_UNIT: i64 = 100;

#[derive(Debug, thiserror::Error)]
pub enum KalshiHistoryError {
    #[error("malformed response: {0}")]
    Parse(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("transport: {0}")]
    Transport(String),
    #[error("candlestick at {ts} quotes {cents} cents, outside the 0..=100 probability range")]
    ImpossibleProbability { ts: i64, cents: i64 },
    #[error("candlestick timestamp {0} is not a valid instant")]
    BadTimestamp(i64),
}

// ── Kalshi response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CandlesticksResponse {
    #[serde(default)]
    pub candlesticks: Vec<Candlestick>,
}

#[derive(Debug, Deserialize)]
pub struct Candlestick {
    /// Unix seconds of the period's **close**.
    pub end_period_ts: i64,
    #[serde(default)]
    pub price: Option<CandlePrice>,
    /// Whole contracts (the cents-era shape).
    #[serde(default)]
    pub volume: Option<i64>,
    #[serde(default)]
    pub open_interest: Option<i64>,
    /// Fixed-point strings, e.g. `"30.00"` (the current shape, checked
    /// 2026-10-04 against `api.elections.kalshi.com`).
    #[serde(default)]
    pub volume_fp: Option<String>,
    #[serde(default)]
    pub open_interest_fp: Option<String>,
}

/// A candlestick's traded price. Kalshi moved from integer cents (`close`) to
/// dollar strings (`close_dollars`, `"0.3400"`); both are read, the dollars form
/// first, so a recorded old payload and the live endpoint parse alike. A period
/// with no trades carries an empty `price` object, which has neither.
#[derive(Debug, Deserialize)]
pub struct CandlePrice {
    #[serde(default)]
    pub open: Option<i64>,
    #[serde(default)]
    pub high: Option<i64>,
    #[serde(default)]
    pub low: Option<i64>,
    #[serde(default)]
    pub close: Option<i64>,
    #[serde(default)]
    pub close_dollars: Option<String>,
}

/// One historical observation of an event's probability, PIT-honest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KalshiHistoryBar {
    pub ticker: String,
    /// The period's close.
    pub event_time: DateTime<Utc>,
    /// Always [`Knowledge::Derived`] on this path — `event_time` plus the
    /// declared lag, carrying `BACKFILLED_KNOWLEDGE_TIME`. A row from the live
    /// collector carries an observed receipt instead.
    pub knowledge: Knowledge,
    /// Probability in 0..=1, not cents.
    pub close_probability: Decimal,
    pub volume: i64,
    pub open_interest: i64,
}

/// Parses a candlesticks payload into PIT-honest history rows.
///
/// # Errors
/// A malformed payload, an out-of-range probability, or a timestamp that is not
/// a valid instant. Candlesticks with no traded price are skipped rather than
/// invented — a period with no trades has no probability, and zero would be a
/// number a strategy could act on.
pub fn parse_candlesticks(
    ticker: &str,
    body: &str,
) -> Result<Vec<KalshiHistoryBar>, KalshiHistoryError> {
    // Decoding a body fetched from an external REST API — see the note on the
    // `lint-no-json-hotpath` exception in `collectors::macro::fred`.
    let parsed = serde_json::from_str::<CandlesticksResponse>(body)
        .map_err(|e| KalshiHistoryError::Parse(e.to_string()))?;
    let mut out = Vec::with_capacity(parsed.candlesticks.len());
    for candle in parsed.candlesticks {
        let Some(price) = candle.price.as_ref() else {
            continue;
        };
        // Dollars first (the live shape), then cents. A dollar string that does
        // not parse is an error, not a skipped period: a malformed price is a
        // venue defect worth seeing, an absent one is a period with no trades.
        let probability = match (&price.close_dollars, price.close) {
            (Some(d), _) => {
                let p = Decimal::from_str(d.trim())
                    .map_err(|e| KalshiHistoryError::Parse(format!("close_dollars {d:?}: {e}")))?;
                if p < Decimal::ZERO || p > Decimal::ONE {
                    return Err(KalshiHistoryError::ImpossibleProbability {
                        ts: candle.end_period_ts,
                        cents: (p * Decimal::from(CENTS_PER_UNIT))
                            .round()
                            .to_string()
                            .parse()
                            .unwrap_or(i64::MAX),
                    });
                }
                p
            }
            (None, Some(cents)) => {
                if !(0..=CENTS_PER_UNIT).contains(&cents) {
                    return Err(KalshiHistoryError::ImpossibleProbability {
                        ts: candle.end_period_ts,
                        cents,
                    });
                }
                Decimal::from(cents) / Decimal::from(CENTS_PER_UNIT)
            }
            (None, None) => continue,
        };
        let event_time = Utc
            .timestamp_opt(candle.end_period_ts, 0)
            .single()
            .ok_or(KalshiHistoryError::BadTimestamp(candle.end_period_ts))?;
        let knowledge = source()
            .knowledge_for(event_time, Collection::Historical)
            .map_err(|e| KalshiHistoryError::Parse(e.to_string()))?;
        out.push(KalshiHistoryBar {
            ticker: ticker.to_owned(),
            event_time,
            knowledge,
            close_probability: probability,
            volume: whole(candle.volume_fp.as_deref())
                .or(candle.volume)
                .unwrap_or(0),
            open_interest: whole(candle.open_interest_fp.as_deref())
                .or(candle.open_interest)
                .unwrap_or(0),
        });
    }
    Ok(out)
}

/// A fixed-point count string (`"30.00"`) as whole contracts, rounded.
fn whole(fp: Option<&str>) -> Option<i64> {
    let d = Decimal::from_str(fp?.trim()).ok()?;
    d.round().to_string().parse().ok()
}

/// Kalshi's public API base (the same host the live poller uses).
pub const KALSHI_REST_BASE: &str = "https://api.elections.kalshi.com/trade-api/v2";

/// The venue name rows are written under, matching the live collector's.
pub const VENUE: &str = "kalshi";

/// The series a market ticker belongs to: the part before the first `-`
/// (`KXHIGHNY-26SEP23-B65.5` is in series `KXHIGHNY`). The candlesticks URL needs
/// both; a ticker with no `-` has no series and is refused rather than guessed.
#[must_use]
pub fn series_of(ticker: &str) -> Option<&str> {
    let (series, rest) = ticker.split_once('-')?;
    (!series.is_empty() && !rest.is_empty()).then_some(series)
}

/// The self-imposed budget for this reader: Kalshi's published read ceiling is
/// far above it and this is a research collector, not a feed.
#[must_use]
pub const fn limit() -> crate::ratelimit::VenueLimit {
    crate::ratelimit::VenueLimit {
        venue: "kalshi_public",
        capacity: 5,
        refill_tokens: 5,
        refill_window: Duration::from_secs(1),
        cost_per_request: 1,
        source_quote: "No ceiling is quoted here: a self-imposed 5 requests/second for the public, unauthenticated market-data endpoints.",
    }
}

/// Fetch one market's candlesticks for `[from, to]` and parse them into
/// PIT-honest history rows. `period_minutes` is Kalshi's `period_interval`
/// (1, 60 or 1440).
///
/// The limiter is asked **before** the request is sent and a refusal is returned
/// as [`KalshiHistoryError::RateLimited`]: the request is not sent and the caller
/// waits and asks again.
///
/// # Errors
/// A ticker with no series, the limiter, transport/HTTP status, or a malformed
/// payload.
pub async fn fetch_candlesticks(
    http: &dyn crate::backfill::HttpGet,
    limiter: &crate::ratelimit::RateLimiter,
    base: &str,
    ticker: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    period_minutes: u32,
) -> Result<Vec<KalshiHistoryBar>, KalshiHistoryError> {
    let series = series_of(ticker).ok_or_else(|| {
        KalshiHistoryError::Parse(format!("ticker {ticker:?} names no series (no `-`)"))
    })?;
    limiter
        .try_acquire()
        .map_err(|d| KalshiHistoryError::RateLimited(d.to_string()))?;
    let url = format!(
        "{base}/series/{series}/markets/{ticker}/candlesticks?start_ts={}&end_ts={}&period_interval={period_minutes}",
        from.timestamp(),
        to.timestamp()
    );
    let resp = http
        .get(&url)
        .await
        .map_err(|e| KalshiHistoryError::Transport(e.to_string()))?;
    let body = resp
        .ok_body("kalshi")
        .map_err(|e| KalshiHistoryError::Transport(e.to_string()))?;
    parse_candlesticks(ticker, &String::from_utf8_lossy(&body))
}

/// History rows as stored rows. The source id is resolved by the caller from
/// `dataplane.source` by [`SOURCE_NAME`]; this does not guess one.
#[must_use]
pub fn to_rows(
    bars: &[KalshiHistoryBar],
    source_id: i32,
) -> Vec<storage::clickhouse::alt::ProbabilityRow> {
    bars.iter()
        .map(|b| {
            storage::clickhouse::alt::ProbabilityRow::new(
                VENUE,
                &b.ticker,
                b.event_time,
                b.knowledge,
                b.close_probability,
                b.volume,
                b.open_interest,
                source_id,
            )
        })
        .collect()
}

/// The rows knowable at `as_of`.
///
/// The filter is on `knowledge_time`, never on `event_time`: a period that closed
/// a minute before the cutoff was not yet readable at the cutoff.
#[must_use]
pub fn bars_as_of(rows: &[KalshiHistoryBar], as_of: DateTime<Utc>) -> Vec<&KalshiHistoryBar> {
    rows.iter()
        .filter(|r| r.knowledge.time() <= as_of)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    const PAGE: &str = r#"{"candlesticks":[
      {"end_period_ts": 1704067200, "price": {"open": 40, "high": 45, "low": 39, "close": 42},
       "volume": 1200, "open_interest": 5000},
      {"end_period_ts": 1704070800, "price": {"open": 42, "high": 42, "low": 42, "close": null},
       "volume": 0, "open_interest": 5000}
    ]}"#;

    #[test]
    fn candlesticks_become_probabilities_not_cents() {
        let rows = parse_candlesticks("KXTEST-26", PAGE).expect("fixture parses");
        // The second candle has no traded close and is skipped, not zero-filled.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].close_probability, dec!(0.42));
        assert_eq!(rows[0].volume, 1200);
    }

    /// D-P2: a backfilled row's knowledge time is derived and flagged, so an
    /// observed row and a backfilled row of the same period stay distinguishable.
    #[test]
    fn a_backfilled_row_is_flagged_and_lagged() {
        let rows = parse_candlesticks("KXTEST-26", PAGE).expect("fixture parses");
        assert!(rows[0].knowledge.is_derived());
        assert!(
            !rows[0].knowledge.is_forward_evidence(),
            "a candlestick nobody watched arrive must never reach Gate 15's evidence"
        );
        assert!(rows[0]
            .knowledge
            .quality_flags()
            .contains(dataplane::QualityFlags::BACKFILLED_KNOWLEDGE_TIME));
        assert_eq!(
            rows[0].knowledge.time(),
            rows[0].event_time + chrono::Duration::milliseconds(DECLARED_LAG_MS)
        );
        assert!(rows[0].knowledge.time() > rows[0].event_time);
    }

    /// The adversarial half: a read at the period close sees nothing, because the
    /// declared lag had not elapsed. Filtering on `event_time` would return it.
    #[test]
    fn a_bar_is_not_readable_at_its_own_close() {
        let rows = parse_candlesticks("KXTEST-26", PAGE).expect("fixture parses");
        let close = rows[0].event_time;
        assert!(bars_as_of(&rows, close).is_empty());
        assert_eq!(
            bars_as_of(
                &rows,
                close + chrono::Duration::milliseconds(DECLARED_LAG_MS)
            )
            .len(),
            1
        );
    }

    /// The live endpoint's shape (recorded 2026-10-04 from
    /// `api.elections.kalshi.com`): dollar strings and fixed-point counts, and an
    /// empty `price` object for a period with no trades.
    const LIVE_PAGE: &str = r#"{"candlesticks":[
      {"end_period_ts":1790089200,"open_interest_fp":"30.00",
       "price":{"close_dollars":"0.3400","high_dollars":"0.3400","low_dollars":"0.3400",
                "mean_dollars":"0.3400","open_dollars":"0.3400"},
       "volume_fp":"30.00","yes_ask":{"close_dollars":"0.3500"},"yes_bid":{"close_dollars":"0.3200"}},
      {"end_period_ts":1790092800,"open_interest_fp":"38.00","price":{},"volume_fp":"0.00"}
    ],"ticker":"KXHIGHNY-26SEP23-B65.5"}"#;

    #[test]
    fn the_live_dollar_shape_parses_and_a_no_trade_period_is_skipped() {
        let rows = parse_candlesticks("KXHIGHNY-26SEP23-B65.5", LIVE_PAGE).expect("live shape");
        assert_eq!(
            rows.len(),
            1,
            "an empty price object is a period with no trades"
        );
        assert_eq!(rows[0].close_probability, dec!(0.34));
        assert_eq!(rows[0].volume, 30);
        assert_eq!(rows[0].open_interest, 30);
        assert!(rows[0].knowledge.is_derived());
    }

    #[test]
    fn a_dollar_price_outside_zero_to_one_is_refused() {
        let bad =
            r#"{"candlesticks":[{"end_period_ts":1790089200,"price":{"close_dollars":"1.4000"}}]}"#;
        assert!(matches!(
            parse_candlesticks("KXTEST-26", bad),
            Err(KalshiHistoryError::ImpossibleProbability { .. })
        ));
        let junk =
            r#"{"candlesticks":[{"end_period_ts":1790089200,"price":{"close_dollars":"abc"}}]}"#;
        assert!(matches!(
            parse_candlesticks("KXTEST-26", junk),
            Err(KalshiHistoryError::Parse(_))
        ));
    }

    #[test]
    fn a_ticker_names_its_series_or_is_refused() {
        assert_eq!(series_of("KXHIGHNY-26SEP23-B65.5"), Some("KXHIGHNY"));
        assert_eq!(series_of("NODASH"), None);
        assert_eq!(series_of("-X"), None);
    }

    struct Canned(std::sync::Mutex<Vec<String>>, &'static str);

    #[async_trait::async_trait]
    impl crate::backfill::HttpGet for Canned {
        async fn get(
            &self,
            url: &str,
        ) -> Result<crate::backfill::HttpResponse, dataplane::backfill::FetchError> {
            self.0.lock().unwrap().push(url.to_owned());
            Ok(crate::backfill::HttpResponse {
                status: 200,
                retry_after_ms: None,
                body: self.1.as_bytes().to_vec(),
            })
        }
    }

    /// The fetch asks the limiter first: past the declared budget the request is
    /// **not sent**, and the URL it does send is a plain GET of the series path.
    #[tokio::test]
    async fn the_fetch_sends_a_get_and_the_limiter_refuses_past_its_budget() {
        let http = Canned(std::sync::Mutex::new(vec![]), LIVE_PAGE);
        let limiter = crate::ratelimit::RateLimiter::new(limit());
        let (from, to) = (
            Utc.timestamp_opt(1_790_000_000, 0).unwrap(),
            Utc.timestamp_opt(1_790_100_000, 0).unwrap(),
        );
        for _ in 0..limit().capacity {
            let rows = fetch_candlesticks(
                &http,
                &limiter,
                KALSHI_REST_BASE,
                "KXHIGHNY-26SEP23-B65.5",
                from,
                to,
                60,
            )
            .await
            .expect("within budget");
            assert_eq!(rows.len(), 1);
        }
        let sent = http.0.lock().unwrap().len();
        let refused = fetch_candlesticks(
            &http,
            &limiter,
            KALSHI_REST_BASE,
            "KXHIGHNY-26SEP23-B65.5",
            from,
            to,
            60,
        )
        .await;
        assert!(
            matches!(refused, Err(KalshiHistoryError::RateLimited(_))),
            "{refused:?}"
        );
        assert_eq!(
            http.0.lock().unwrap().len(),
            sent,
            "a refused request is not sent"
        );
        let url = http.0.lock().unwrap()[0].clone();
        assert!(
            url.contains("/series/KXHIGHNY/markets/KXHIGHNY-26SEP23-B65.5/candlesticks?"),
            "{url}"
        );
        assert!(url.contains("period_interval=60"), "{url}");
    }

    #[test]
    fn stored_rows_keep_the_derived_flag() {
        let rows = parse_candlesticks("KXHIGHNY-26SEP23-B65.5", LIVE_PAGE).unwrap();
        let stored = to_rows(&rows, 24);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].source_id, 24);
        assert!(
            stored[0].quality_flags & dataplane::QualityFlags::BACKFILLED_KNOWLEDGE_TIME.0 != 0,
            "a candlestick row must be written flagged"
        );
        assert!(stored[0].knowledge_time > stored[0].event_time);
    }

    #[test]
    fn an_impossible_probability_is_refused() {
        let bad = r#"{"candlesticks":[{"end_period_ts":1704067200,"price":{"close":140}}]}"#;
        assert!(matches!(
            parse_candlesticks("KXTEST-26", bad),
            Err(KalshiHistoryError::ImpossibleProbability { .. })
        ));
    }

    /// P-3.4's other half: the series carries a **declared** lag rather than an
    /// implied zero, and the number the parser uses is the number the source
    /// declares — one value, not two that can drift apart.
    #[test]
    fn the_declared_lag_is_the_one_the_source_declares() {
        assert_eq!(
            source().declared_lag,
            Duration::from_millis(DECLARED_LAG_MS as u64)
        );
        assert_eq!(source().name, SOURCE_NAME);
        assert_eq!(source().kind, SourceKind::RestHistory);
        let rows = parse_candlesticks("KXTEST-26", PAGE).expect("fixture parses");
        assert_eq!(
            rows[0].knowledge.implied_lag(rows[0].event_time),
            source().declared_lag,
            "the lag a dataset carries into embargo arithmetic is the declared one"
        );
    }
}
