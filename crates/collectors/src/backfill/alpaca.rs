//! Alpaca free-plan historical bars (DATA-005 §11 S1; Set P P-1.4).
//!
//! The third venue. Crypto bars (`/v1beta3/crypto/us/bars`) need no credentials;
//! equity bars do, and on the free plan they are IEX-only and 15 minutes
//! delayed.
//!
//! **Declared vendor lag: 900 s** (`alpaca_rest` in `dataplane.source`,
//! ADR-P2-24) — fifteen times Coinbase's and Kraken's 60 s, because the free
//! tier's SIP data is fifteen-minute delayed. This is the number that must reach
//! the row rather than being assumed equal to the crypto venues': a bar that
//! Alpaca could not have told us about until 15 minutes after it closed, stamped
//! as knowable at close + 60 s, is fourteen minutes of lookahead in every study
//! that touches it.
//!
//! This adapter does not stamp the lag. It returns [`SOURCE`], `BarStore` reads
//! `declared_vendor_lag_ms` from `dataplane.source`, and the two sources' rows
//! come out different because the table says they are different. A lag carried
//! alongside the bars is a lag that can disagree with the table, and the
//! disagreement would be invisible.
//!
//! **Declared rate limit: 190 requests/minute**, inside the free plan's
//! published 200/min, honoured by a limiter that refuses.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dataplane::backfill::{
    BackfillBar, BackfillPage, BackfillSource, FetchError, PageRequest, RateLimit,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;

use super::{raw_number, raw_u64, HttpGet, Limited};

/// `dataplane.source.name` — selects the **900 s** declared vendor lag.
pub const SOURCE: &str = "alpaca_rest";
pub const VENUE: &str = "alpaca";
const CRYPTO_API: &str = "https://data.alpaca.markets/v1beta3/crypto/us/bars";
/// The venue's own page cap.
pub const MAX_BARS: i64 = 10_000;

/// Published free-plan limit is 200 requests/minute; the platform declares 190.
#[must_use]
pub const fn declared_rate_limit() -> RateLimit {
    RateLimit {
        per_window: 190,
        window_ms: 60_000,
        burst: 20,
    }
}

/// Alpaca's `timeframe` parameter.
///
/// # Errors
/// A period with no Alpaca spelling.
pub fn timeframe_param(period_secs: u32) -> Result<String, FetchError> {
    Ok(match period_secs {
        60 => "1Min".into(),
        300 => "5Min".into(),
        900 => "15Min".into(),
        3_600 => "1Hour".into(),
        14_400 => "4Hour".into(),
        86_400 => "1Day".into(),
        other => {
            return Err(FetchError::Integrity(format!(
                "no Alpaca timeframe for {other} s bars"
            )))
        }
    })
}

#[derive(Deserialize)]
struct BarsEnvelope<'a> {
    #[serde(borrow, default)]
    bars: BTreeMap<&'a str, Vec<Bar<'a>>>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct Bar<'a> {
    /// RFC 3339 bar **open**.
    t: &'a str,
    #[serde(borrow)]
    o: &'a RawValue,
    #[serde(borrow)]
    h: &'a RawValue,
    #[serde(borrow)]
    l: &'a RawValue,
    #[serde(borrow)]
    c: &'a RawValue,
    #[serde(borrow)]
    v: &'a RawValue,
    #[serde(borrow, default)]
    n: Option<&'a RawValue>,
}

/// What one page of the bars endpoint contained.
#[derive(Debug)]
pub struct ParsedBars {
    pub bars: Vec<BackfillBar>,
    /// Alpaca's own continuation token. Held for completeness; the engine's
    /// cursor is time-based, so a token is a within-page optimisation rather
    /// than the unit of resumption — a token is not durable across a restart
    /// and a bar-close time is.
    pub next_page_token: Option<String>,
}

/// Parse a crypto/stock bars response.
///
/// # Errors
/// Malformed JSON, an error `message` from the venue, or an unreadable field.
pub fn parse_bars(body: &[u8], period_secs: u32) -> Result<ParsedBars, FetchError> {
    let env: BarsEnvelope = serde_json::from_slice(body)
        .map_err(|e| FetchError::Integrity(format!("alpaca bars: {e}")))?;
    if let Some(m) = env.message {
        // Alpaca answers plan violations with a message rather than a status a
        // client can branch on.
        if m.to_lowercase().contains("subscription") || m.to_lowercase().contains("not permitted") {
            return Err(FetchError::Terms(format!("alpaca: {m}")));
        }
        return Err(FetchError::Integrity(format!("alpaca: {m}")));
    }
    let mut out = Vec::new();
    for rows in env.bars.values() {
        for b in rows {
            let open_time: DateTime<Utc> = DateTime::parse_from_rfc3339(b.t)
                .map_err(|e| FetchError::Integrity(format!("alpaca bar time {:?}: {e}", b.t)))?
                .with_timezone(&Utc);
            out.push(BackfillBar {
                close_time: open_time + chrono::Duration::seconds(i64::from(period_secs)),
                open: raw_number(b.o)?,
                high: raw_number(b.h)?,
                low: raw_number(b.l)?,
                close: raw_number(b.c)?,
                volume: raw_number(b.v)?,
                trade_count: b.n.map(raw_u64).transpose()?.unwrap_or(0),
            });
        }
    }
    out.sort_by_key(|b| b.close_time);
    Ok(ParsedBars {
        bars: out,
        next_page_token: env.next_page_token,
    })
}

/// Alpaca uses `BTC/USD`, the platform uses `BTC-USD`.
#[must_use]
pub fn venue_symbol(platform_symbol: &str) -> String {
    platform_symbol.replace('-', "/")
}

#[must_use]
pub fn crypto_bars_url(
    symbol: &str,
    timeframe: &str,
    open_start: DateTime<Utc>,
    open_end: DateTime<Utc>,
    token: Option<&str>,
) -> String {
    let mut url = format!(
        "{CRYPTO_API}?symbols={}&timeframe={timeframe}&start={}&end={}&limit={MAX_BARS}&sort=asc",
        urlencode(&venue_symbol(symbol)),
        open_start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        open_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    if let Some(t) = token {
        url.push_str("&page_token=");
        url.push_str(&urlencode(t));
    }
    url
}

/// Minimal percent-encoding for the few characters these URLs actually carry.
/// A dependency for this would be a dependency for `/` and `:`.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Alpaca free-plan bar backfill.
pub struct AlpacaBackfill {
    http: Arc<dyn HttpGet>,
    limiter: Limited,
}

impl AlpacaBackfill {
    #[must_use]
    pub fn new(http: Arc<dyn HttpGet>) -> Self {
        Self {
            http,
            limiter: Limited::new(declared_rate_limit()),
        }
    }

    #[must_use]
    pub fn page_span_secs(period_secs: u32) -> i64 {
        i64::from(period_secs) * MAX_BARS
    }
}

#[async_trait]
impl BackfillSource for AlpacaBackfill {
    fn source_name(&self) -> &str {
        SOURCE
    }

    fn declared_limit(&self) -> RateLimit {
        declared_rate_limit()
    }

    async fn fetch_page(&self, req: &PageRequest) -> Result<BackfillPage, FetchError> {
        let tf = timeframe_param(req.period_secs)?;
        let period = chrono::Duration::seconds(i64::from(req.period_secs));
        let span = Self::page_span_secs(req.period_secs);
        let page_end: DateTime<Utc> =
            std::cmp::min(req.end, req.start + chrono::Duration::seconds(span));
        if page_end <= req.start {
            return Ok(BackfillPage::end_of_window());
        }

        self.limiter.permit().await;
        let url = crypto_bars_url(
            &req.symbol,
            &tf,
            req.start - period,
            page_end - period,
            None,
        );
        let body = self.http.get(&url).await?.ok_body(VENUE)?;
        let parsed = parse_bars(&body, req.period_secs)?;
        let bars: Vec<_> = parsed
            .bars
            .into_iter()
            .filter(|b| b.close_time >= req.start && b.close_time < page_end)
            .collect();
        Ok(BackfillPage {
            bars,
            covered_to: Some(page_end),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const BODY: &[u8] = br#"{"bars":{"BTC/USD":[
      {"t":"2023-11-14T22:13:00Z","o":73850.0,"h":73899.9,"l":73800.0,"c":73861.99,"v":2.0,"n":7},
      {"t":"2023-11-14T22:14:00Z","o":73950.0,"h":73999.9,"l":73900.0,"c":73961.99,"v":1.4108,"n":3}
    ]},"next_page_token":"abc"}"#;

    #[test]
    fn bars_parse_with_the_open_shifted_to_a_close() {
        let p = parse_bars(BODY, 60).unwrap();
        assert_eq!(p.bars.len(), 2);
        assert_eq!(
            p.bars[0].close_time,
            Utc.with_ymd_and_hms(2023, 11, 14, 22, 14, 0).unwrap()
        );
        assert_eq!(p.bars[1].close, "73961.99");
        assert_eq!(p.bars[1].trade_count, 3);
        assert_eq!(p.next_page_token.as_deref(), Some("abc"));
    }

    /// **The P-1.4 acceptance, stated where it can be checked without a
    /// database**: Alpaca's declared lag is 900 s and the crypto venues' is
    /// 60 s, so two backfilled rows for the same bar close from the two sources
    /// get different `knowledge_time`s. The seeding lives in migration 0046 and
    /// the arithmetic in `dataplane::bar::stamp_knowledge_time`; what this
    /// asserts is that the adapters name *different sources*, which is the only
    /// part an adapter can get wrong.
    #[test]
    fn alpaca_names_a_different_source_than_the_crypto_venues() {
        assert_eq!(SOURCE, "alpaca_rest");
        assert_ne!(SOURCE, super::super::coinbase::SOURCE);
        assert_ne!(SOURCE, super::super::kraken::SOURCE);
    }

    #[test]
    fn a_plan_restriction_is_a_terms_refusal_not_a_retry_forever() {
        let body = br#"{"message":"subscription does not permit querying recent SIP data"}"#;
        let e = parse_bars(body, 60).expect_err("expected a refusal");
        assert!(matches!(e, FetchError::Terms(_)), "{e:?}");
        assert!(
            !e.is_pause(),
            "a plan restriction is not something to wait out"
        );
    }

    #[test]
    fn a_float_price_keeps_its_digits_through_the_parser() {
        // 0.1 + 0.2 is the canonical f64 failure; here it is only ever text.
        let body = br#"{"bars":{"X/USD":[{"t":"2023-11-14T22:13:00Z","o":0.30000000000000004,"h":1,"l":1,"c":1,"v":1}]}}"#;
        let p = parse_bars(body, 60).unwrap();
        assert_eq!(p.bars[0].open, "0.30000000000000004");
    }

    #[test]
    fn the_declared_limit_sits_under_the_published_free_plan_limit() {
        let l = declared_rate_limit();
        assert_eq!(l.window_ms, 60_000);
        assert!(
            l.per_window <= 200,
            "Alpaca's free plan publishes 200 requests/minute"
        );
        assert!(
            l.per_window >= 100,
            "and declaring 1/min would make a three-year backfill take months"
        );
    }

    #[test]
    fn the_platform_symbol_is_translated_and_encoded_once() {
        assert_eq!(venue_symbol("BTC-USD"), "BTC/USD");
        let url = crypto_bars_url(
            "BTC-USD",
            "1Min",
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2023, 1, 2, 0, 0, 0).unwrap(),
            None,
        );
        assert!(url.contains("symbols=BTC%2FUSD"), "{url}");
        assert!(url.contains("sort=asc"), "{url}");
        assert!(!url.contains("page_token"), "{url}");
    }

    #[test]
    fn an_unknown_period_is_refused_rather_than_mapped_to_the_nearest() {
        assert_eq!(timeframe_param(3_600).unwrap(), "1Hour");
        assert!(timeframe_param(120).is_err());
    }
}
