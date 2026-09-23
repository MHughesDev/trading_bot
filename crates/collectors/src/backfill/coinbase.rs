//! Coinbase Exchange historical candles (DATA-005 §11 S1; Set P P-1.2).
//!
//! `GET /products/{id}/candles?granularity=&start=&end=` returns at most 300
//! candles as `[time, low, high, open, close, volume]`, where `time` is the bar
//! **open** in epoch seconds. Two consequences shape this module:
//!
//! * The platform stores and requests bar **closes**; the venue speaks opens.
//!   The conversion happens in exactly one place ([`parse_candles`]) because an
//!   off-by-one-period here shifts the whole series by a bar and nothing later
//!   in the pipeline can detect it.
//! * 300 candles is 5 hours of 1m data. The page span is derived from the
//!   granularity rather than fixed, so a 1h window is not fetched 300 bars at a
//!   time when the venue would have served 300 hours.
//!
//! **Declared rate limit: 8 requests/second.** Coinbase publishes 10 rps for
//! public endpoints with a short burst allowance; the platform declares 8 and
//! honours it with a limiter that *refuses*. The margin is deliberate — being
//! 20 % slow costs an hour on a multi-year backfill, and being 10 % fast costs
//! the endpoint.
//!
//! **Declared vendor lag: 60 s** (`coinbase_rest` in `dataplane.source`,
//! ADR-P2-24). Every row this adapter's bars become carries
//! `knowledge_time = bar close + 60 s` and `BACKFILLED_KNOWLEDGE_TIME`. This
//! adapter does not stamp it and must not: it returns its source *name*, and
//! `BarStore` reads the lag from the source row. A lag passed alongside the data
//! is a lag that can disagree with the source table.

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use dataplane::backfill::{
    BackfillBar, BackfillPage, BackfillSource, FetchError, PageRequest, RateLimit,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::sync::Arc;

use super::{raw_number, HttpGet, Limited};

/// `dataplane.source.name` — selects the 60 s declared vendor lag.
pub const SOURCE: &str = "coinbase_rest";
pub const VENUE: &str = "coinbase";
const API: &str = "https://api.exchange.coinbase.com";
/// The venue's own page cap.
pub const MAX_CANDLES: i64 = 300;

/// The limit the platform declares for itself, at or under the published one.
#[must_use]
pub const fn declared_rate_limit() -> RateLimit {
    RateLimit {
        per_window: 8,
        window_ms: 1_000,
        burst: 8,
    }
}

/// Coinbase exposes only a fixed set of granularities; anything else is a
/// silently resampled series if you guess.
///
/// # Errors
/// A period the venue does not serve.
pub fn granularity(period_secs: u32) -> Result<u32, FetchError> {
    match period_secs {
        60 | 300 | 900 | 3_600 | 21_600 | 86_400 => Ok(period_secs),
        other => Err(FetchError::Integrity(format!(
            "Coinbase Exchange serves 60/300/900/3600/21600/86400 s candles; {other} s is not one of them"
        ))),
    }
}

/// `[time, low, high, open, close, volume]`, every field read as raw digits.
#[derive(Deserialize)]
struct Candle<'a>(
    #[serde(borrow)] &'a RawValue, // time (bar open, epoch seconds)
    #[serde(borrow)] &'a RawValue, // low
    #[serde(borrow)] &'a RawValue, // high
    #[serde(borrow)] &'a RawValue, // open
    #[serde(borrow)] &'a RawValue, // close
    #[serde(borrow)] &'a RawValue, // volume
);

/// Parse a candles response into platform bars, oldest first.
///
/// `period_secs` converts the venue's bar-open timestamps into the bar-close
/// times the platform stores.
///
/// # Errors
/// Malformed JSON, a non-numeric field, or a timestamp outside the representable
/// range — all [`FetchError::Integrity`], because a backfill that quietly drops
/// the rows it could not read produces a gap nobody can attribute.
pub fn parse_candles(body: &[u8], period_secs: u32) -> Result<Vec<BackfillBar>, FetchError> {
    // json-at-boundary: Coinbase's candles REST response, not one of our envelopes
    let candles: Vec<Candle> = serde_json::from_slice(body)
        .map_err(|e| FetchError::Integrity(format!("coinbase candles: {e}")))?;
    let mut out = Vec::with_capacity(candles.len());
    for c in candles {
        let open_s: i64 = raw_number(c.0)?
            .parse()
            .map_err(|_| FetchError::Integrity("coinbase candle time is not an integer".into()))?;
        let open_time = Utc.timestamp_opt(open_s, 0).single().ok_or_else(|| {
            FetchError::Integrity(format!(
                "coinbase candle time {open_s} is not a valid instant"
            ))
        })?;
        out.push(BackfillBar {
            // The one conversion: venue open + period = platform close.
            close_time: open_time + chrono::Duration::seconds(i64::from(period_secs)),
            low: raw_number(c.1)?,
            high: raw_number(c.2)?,
            open: raw_number(c.3)?,
            close: raw_number(c.4)?,
            volume: raw_number(c.5)?,
            // Coinbase's candle endpoint does not report a trade count. Zero is
            // the honest value: the field means "trades we were told about".
            trade_count: 0,
        });
    }
    out.sort_by_key(|b| b.close_time);
    Ok(out)
}

/// The URL for one page. `start`/`end` are the venue's bar-**open** seconds.
#[must_use]
pub fn candles_url(product: &str, granularity: u32, open_start: i64, open_end: i64) -> String {
    format!("{API}/products/{product}/candles?granularity={granularity}&start={open_start}&end={open_end}")
}

/// Coinbase Exchange candle backfill.
pub struct CoinbaseBackfill {
    http: Arc<dyn HttpGet>,
    limiter: Limited,
}

impl CoinbaseBackfill {
    #[must_use]
    pub fn new(http: Arc<dyn HttpGet>) -> Self {
        Self {
            http,
            limiter: Limited::new(declared_rate_limit()),
        }
    }

    /// How much bar-close time one page can carry, given the venue's 300-candle
    /// cap. Derived, never assumed: a fixed span would either waste requests on
    /// coarse timeframes or silently truncate fine ones.
    #[must_use]
    pub fn page_span_secs(period_secs: u32) -> i64 {
        i64::from(period_secs) * MAX_CANDLES
    }
}

#[async_trait]
impl BackfillSource for CoinbaseBackfill {
    fn source_name(&self) -> &str {
        SOURCE
    }

    fn declared_limit(&self) -> RateLimit {
        declared_rate_limit()
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        let g = granularity(req.period_secs)?;
        let period = i64::from(req.period_secs);
        // Clip the page to what the venue will serve in one request, so the
        // engine's cursor advances by exactly what was actually covered.
        let span = Self::page_span_secs(req.period_secs);
        let page_end: DateTime<Utc> =
            std::cmp::min(req.end, req.start + chrono::Duration::seconds(span));
        if page_end <= req.start {
            return Ok(BackfillPage::end_of_window());
        }
        // Platform closes → venue opens.
        let open_start = req.start.timestamp() - period;
        let open_end = page_end.timestamp() - period;

        // The permit is taken before the request, every time, with no path that
        // skips it.
        self.limiter.permit().await;
        let body = self
            .http
            .get(&candles_url(&req.symbol, g, open_start, open_end))
            .await?
            .ok_body(VENUE)?;
        let bars = parse_candles(&body, req.period_secs)?;

        // `covered_to` is the page window, not the last bar: a venue outage that
        // returns no candles for a real range is a gap in the data, not the end
        // of the window, and treating it as the end would truncate the backfill
        // at the first quiet hour.
        Ok(BackfillPage {
            bars,
            covered_to: Some(page_end),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const BODY: &[u8] = br#"[
      [1700000120, "74000.11", "74100.72", "74054.00", "73947.01", "75.06970001"],
      [1700000060, "73900.00", "73999.99", "73950.00", "73961.99", "1.41080000"],
      [1700000000, "73800.00", "73899.99", "73850.00", "73861.99", "2.00000000"]
    ]"#;

    #[test]
    fn a_venue_bar_open_becomes_a_platform_bar_close() {
        let bars = parse_candles(BODY, 60).unwrap();
        assert_eq!(bars.len(), 3);
        // Oldest first, regardless of the venue's newest-first ordering.
        assert_eq!(
            bars[0].close_time,
            Utc.timestamp_opt(1_700_000_060, 0).unwrap()
        );
        assert_eq!(
            bars[2].close_time,
            Utc.timestamp_opt(1_700_000_180, 0).unwrap()
        );
        // The venue's field order is [time, low, high, open, close, volume] —
        // not OHLC. Getting this wrong produces bars that still validate.
        assert_eq!(bars[2].low, "74000.11");
        assert_eq!(bars[2].high, "74100.72");
        assert_eq!(bars[2].open, "74054.00");
        assert_eq!(bars[2].close, "73947.01");
        assert_eq!(bars[2].volume, "75.06970001");
    }

    #[test]
    fn the_period_is_what_shifts_a_bar_from_open_to_close() {
        let m1 = parse_candles(BODY, 60).unwrap();
        let h1 = parse_candles(BODY, 3_600).unwrap();
        assert_eq!(
            h1[0].close_time - m1[0].close_time,
            Duration::seconds(3_540)
        );
    }

    #[test]
    fn a_granularity_the_venue_does_not_serve_is_refused_rather_than_resampled() {
        assert!(granularity(60).is_ok());
        assert!(
            granularity(14_400).is_err(),
            "Coinbase has no 4h candle; asking for one must not silently return 1h"
        );
    }

    #[test]
    fn a_malformed_candle_is_an_integrity_failure_not_a_silent_gap() {
        assert!(matches!(
            parse_candles(b"[[1,2,3]]", 60),
            Err(FetchError::Integrity(_))
        ));
        assert!(matches!(
            parse_candles(br#"[[1700000000,"a","b","c","d","e"]]"#, 60),
            Err(FetchError::Integrity(_))
        ));
    }

    #[test]
    fn the_page_span_follows_the_venue_s_candle_cap() {
        assert_eq!(CoinbaseBackfill::page_span_secs(60), 18_000, "300 minutes");
        assert_eq!(
            CoinbaseBackfill::page_span_secs(86_400),
            25_920_000,
            "300 days"
        );
    }

    #[test]
    fn the_declared_limit_sits_under_the_published_one() {
        let l = declared_rate_limit();
        assert!(
            l.per_window <= 10,
            "Coinbase publishes 10 rps for public endpoints"
        );
        assert_eq!(l.window_ms, 1_000);
    }

    #[test]
    fn the_url_speaks_venue_opens_not_platform_closes() {
        // A 1m window whose first bar CLOSES at 1700000060 opens at 1700000000.
        let url = candles_url("BTC-USD", 60, 1_700_000_060 - 60, 1_700_000_360 - 60);
        assert!(url.contains("start=1700000000"), "{url}");
        assert!(url.contains("end=1700000300"), "{url}");
        assert!(url.contains("granularity=60"));
    }
}
