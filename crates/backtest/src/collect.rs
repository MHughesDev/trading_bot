//! Automated historical data collection ("speed-run" backfill).
//!
//! Driven by the data requirements of the strategy under test: the manager
//! computes the missing ranges for the strategy's timeframe (plus indicator
//! warm-up) and this module fills them from a venue REST API in paged
//! 1000-bar requests, writing straight into the platform's `ClickHouse` store.
//!
//! Sources are additive — new asset classes plug in by extending
//! [`CollectorPlan::for_asset_class`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use chrono::{DateTime, TimeZone, Utc};
use domain::payloads::bar::Timeframe;
use serde_json::Value;

use crate::store::{BarStore, CollectedBar};
use crate::types::{MissingRange, TimeframeExt};

/// Which upstream source fills gaps for a given instrument.
#[derive(Clone, Debug)]
pub enum CollectorPlan {
    /// Coinbase Exchange public candles — unauthenticated spot crypto history
    /// with true start/end paging (300 candles per request, arbitrary depth).
    /// Kraken's OHLC endpoint was abandoned here because it serves only the
    /// most recent ~720 candles per interval: any longer backfill silently
    /// truncated (12h of 1m, 7.5d of 15m), which broke long backtests, asset
    /// seeding, and gap-fill alike.  Coinbase products use the same
    /// `BASE-QUOTE` ids as this platform (e.g. `BTC-USD`).
    CoinbaseCandles { product: String, source: String },
    /// Binance public klines — unauthenticated crypto history.  Geo-blocked
    /// from US IPs (HTTP 451), so only used for classes with no Kraken
    /// spot equivalent (perps, DEX).
    BinanceKlines { symbol: String, source: String },
    /// Alpaca market-data bars — equities/ETFs (requires API credentials).
    AlpacaBars {
        symbol: String,
        key_id: String,
        secret: String,
    },
}

impl CollectorPlan {
    /// Chooses a backfill source for the instrument, based on asset class.
    ///
    /// Crypto spot uses Kraken's public OHLC API — the same venue as the live
    /// data feed — so backtest and live history are consistent.  Perps/DEX use
    /// Binance (Kraken has limited perpetual coverage).  Equities/ETFs use
    /// Alpaca's data API with the same credentials the live collector uses
    /// (`ALPACA_API_KEY_ID` / `ALPACA_API_SECRET_KEY`).
    pub fn for_asset_class(asset_class: &str, instrument_id: &str) -> anyhow::Result<Self> {
        match asset_class {
            "crypto_spot_cex" => {
                // Coinbase product ids are exactly our BASE-QUOTE instrument
                // ids (e.g. "BTC-USD").  The quote currency is preserved
                // exactly: USD and USDT are different markets and must never
                // be proxied.  If Coinbase does not list the product the
                // fetch fails with a clear API error.
                anyhow::ensure!(
                    instrument_id.contains('-'),
                    "instrument '{instrument_id}' must be BASE-QUOTE form (e.g. BTC-USD)"
                );
                Ok(Self::CoinbaseCandles {
                    product: instrument_id.to_uppercase(),
                    source: "coinbase_rest".to_string(),
                })
            }
            "crypto_spot_dex" | "perpetual_swap" => {
                // "BTC-USDT" → "BTCUSDT": strip separators only, never rewrite
                // the quote currency.  A `-USD` instrument is a genuinely
                // different market from the `-USDT` one (USD vs. a stablecoin);
                // silently proxying it would backfill the wrong market's
                // history.  If Binance does not list the exact symbol the page
                // fetch fails with a clear error, which is the correct outcome.
                let symbol: String = instrument_id
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect();
                anyhow::ensure!(
                    !symbol.is_empty(),
                    "instrument '{instrument_id}' has no usable Binance symbol"
                );
                Ok(Self::BinanceKlines {
                    symbol,
                    source: "binance_rest".to_string(),
                })
            }
            "equity" | "etf" => {
                let key_id = std::env::var("ALPACA_API_KEY_ID").unwrap_or_default();
                let secret = std::env::var("ALPACA_API_SECRET_KEY").unwrap_or_default();
                anyhow::ensure!(
                    !key_id.is_empty() && !secret.is_empty(),
                    "equity backfill requires ALPACA_API_KEY_ID / ALPACA_API_SECRET_KEY"
                );
                Ok(Self::AlpacaBars {
                    symbol: instrument_id.to_string(),
                    key_id,
                    secret,
                })
            }
            other => anyhow::bail!(
                "automated historical collection is not yet available for asset class '{other}'"
            ),
        }
    }

    /// Whether automated backfill exists for an asset-class / timeframe pair,
    /// independent of credentials.
    ///
    /// Used to reject unsupported create requests up front (422) instead of
    /// letting a job reach `CollectingData` only to fail there (#15).  Mirrors
    /// the capability of the concrete collectors:
    /// [`coinbase_granularity`] (no 1s; 4h aggregated from 1h),
    /// [`binance_interval`] (all timeframes), and [`alpaca_interval`] (no 1s).
    pub fn auto_collect_support(asset_class: &str, timeframe: Timeframe) -> Result<(), String> {
        match asset_class {
            "crypto_spot_cex" => match timeframe {
                Timeframe::Seconds1 => Err(
                    "the crypto spot backfill (Coinbase) does not provide 1-second bars"
                        .to_string(),
                ),
                _ => Ok(()),
            },
            "crypto_spot_dex" | "perpetual_swap" => Ok(()),
            "equity" | "etf" => {
                if timeframe == Timeframe::Seconds1 {
                    Err("the equity backfill (Alpaca) does not provide 1-second bars".to_string())
                } else {
                    Ok(())
                }
            }
            other => Err(format!(
                "automated historical collection is not available for asset class '{other}'"
            )),
        }
    }

    pub fn source_name(&self) -> &str {
        match self {
            Self::CoinbaseCandles { source, .. } | Self::BinanceKlines { source, .. } => source,
            Self::AlpacaBars { .. } => "alpaca_rest",
        }
    }

    pub fn trust_tier(&self) -> &'static str {
        match self {
            Self::CoinbaseCandles { .. } | Self::BinanceKlines { .. } => "centralized_exchange",
            Self::AlpacaBars { .. } => "regulated",
        }
    }
}

/// Collects all `ranges`, inserting bars into `store` as pages arrive.
///
/// `collected` is incremented per inserted bar so the manager can surface
/// live progress; `cancel` stops cleanly at the next page boundary.
#[allow(clippy::too_many_arguments)]
pub async fn collect_ranges(
    http: &reqwest::Client,
    store: &BarStore,
    plan: &CollectorPlan,
    instrument_id: &str,
    venue_id: &str,
    timeframe: Timeframe,
    ranges: &[MissingRange],
    collected: &AtomicU64,
    cancel: &AtomicBool,
) -> anyhow::Result<u64> {
    let mut total = 0u64;
    for range in ranges {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        total += match plan {
            CollectorPlan::CoinbaseCandles { product, .. } => {
                collect_coinbase(
                    http,
                    store,
                    plan,
                    product,
                    instrument_id,
                    venue_id,
                    timeframe,
                    range,
                    collected,
                    cancel,
                )
                .await?
            }
            CollectorPlan::BinanceKlines { symbol, .. } => {
                collect_binance(
                    http,
                    store,
                    plan,
                    symbol,
                    instrument_id,
                    venue_id,
                    timeframe,
                    range,
                    collected,
                    cancel,
                )
                .await?
            }
            CollectorPlan::AlpacaBars {
                symbol,
                key_id,
                secret,
            } => {
                collect_alpaca(
                    http,
                    store,
                    plan,
                    symbol,
                    key_id,
                    secret,
                    instrument_id,
                    venue_id,
                    timeframe,
                    range,
                    collected,
                    cancel,
                )
                .await?
            }
        };
    }
    Ok(total)
}

/// Maximum attempts (1 try + retries) for a single collector page fetch.
const MAX_FETCH_ATTEMPTS: u32 = 5;

/// GETs `url` with bounded exponential backoff (2s, 4s, 8s, 16s).
///
/// Transport errors and retryable HTTP statuses (429 + any 5xx) are retried;
/// other non-success statuses fail fast.  `headers` applies per-request auth
/// (Alpaca); Binance passes an empty slice.  Cancellation short-circuits the
/// backoff so a stop request doesn't have to wait out the sleep.
async fn fetch_json_with_retry(
    http: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
    cancel: &AtomicBool,
) -> anyhow::Result<Value> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let mut req = http.get(url);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let outcome = req.send().await;

        let retryable_status = |status: reqwest::StatusCode| {
            status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
        };

        match outcome {
            Ok(resp) if resp.status().is_success() => {
                return Ok(resp.json().await?);
            }
            Ok(resp) if retryable_status(resp.status()) && attempt < MAX_FETCH_ATTEMPTS => {
                tracing::warn!(url, status = %resp.status(), attempt, "collector fetch retrying");
            }
            Ok(resp) => {
                anyhow::bail!("collector request failed: HTTP {}", resp.status());
            }
            Err(e) if attempt < MAX_FETCH_ATTEMPTS => {
                tracing::warn!(url, error = %e, attempt, "collector fetch transport error, retrying");
            }
            Err(e) => {
                return Err(anyhow::anyhow!(e)
                    .context(format!("collector request failed after {attempt} attempts")));
            }
        }

        // Exponential backoff: 2s, 4s, 8s, 16s — interruptible by cancel.
        let backoff = std::time::Duration::from_secs(2u64.saturating_pow(attempt));
        let mut waited = std::time::Duration::ZERO;
        while waited < backoff {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("collection cancelled");
            }
            let step =
                std::time::Duration::from_millis(200).min(backoff.checked_sub(waited).unwrap());
            tokio::time::sleep(step).await;
            waited += step;
        }
    }
}

/// Coinbase Exchange candle granularity for a timeframe.
///
/// Returns `(fetch_granularity_secs, aggregate_factor)`.  Coinbase supports
/// 60/300/900/3600/21600/86400 seconds; 4h is absent, so it is assembled from
/// 1h candles (factor 4) at ingestion.
fn coinbase_granularity(tf: Timeframe) -> anyhow::Result<(i64, usize)> {
    Ok(match tf {
        Timeframe::Seconds1 => anyhow::bail!("Coinbase does not provide 1-second candles"),
        Timeframe::Minutes1 => (60, 1),
        Timeframe::Minutes5 => (300, 1),
        Timeframe::Minutes15 => (900, 1),
        Timeframe::Hours1 => (3600, 1),
        Timeframe::Hours4 => (3600, 4),
        Timeframe::Daily => (86_400, 1),
    })
}

fn binance_interval(tf: Timeframe) -> &'static str {
    match tf {
        Timeframe::Seconds1 => "1s",
        Timeframe::Minutes1 => "1m",
        Timeframe::Minutes5 => "5m",
        Timeframe::Minutes15 => "15m",
        Timeframe::Hours1 => "1h",
        Timeframe::Hours4 => "4h",
        Timeframe::Daily => "1d",
    }
}

fn alpaca_interval(tf: Timeframe) -> anyhow::Result<&'static str> {
    Ok(match tf {
        Timeframe::Seconds1 => anyhow::bail!("Alpaca does not provide 1-second bars"),
        Timeframe::Minutes1 => "1Min",
        Timeframe::Minutes5 => "5Min",
        Timeframe::Minutes15 => "15Min",
        Timeframe::Hours1 => "1Hour",
        Timeframe::Hours4 => "4Hour",
        Timeframe::Daily => "1Day",
    })
}

/// One raw Coinbase candle: `(open_secs, low, high, open, close, volume)`.
type RawCandle = (i64, f64, f64, f64, f64, f64);

/// Renders an ingestion-boundary decimal string from a Coinbase float.
///
/// `format!` (not `Value::to_string`) so tiny volumes never come out in
/// scientific notation, which the downstream `Decimal` parse rejects.
fn dec_str(v: f64) -> String {
    let s = format!("{v:.8}");
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[allow(clippy::too_many_arguments)]
// OHLCV aggregation reads naturally as o/h/l/c/v; renaming would hurt more than help.
#[allow(clippy::many_single_char_names)]
async fn collect_coinbase(
    http: &reqwest::Client,
    store: &BarStore,
    plan: &CollectorPlan,
    product: &str,
    instrument_id: &str,
    venue_id: &str,
    timeframe: Timeframe,
    range: &MissingRange,
    collected: &AtomicU64,
    cancel: &AtomicBool,
) -> anyhow::Result<u64> {
    let (gran_secs, factor) = coinbase_granularity(timeframe)?;
    let tf_secs = i64::try_from(timeframe.seconds()).unwrap_or(i64::MAX);
    let start_s = range.from.timestamp();
    let end_s = range.to.timestamp();
    // The API errors when a request spans more than 300 candles, so page in
    // exactly-300-candle windows.
    let page_span = gran_secs * 300;
    let mut cursor = start_s;
    let mut total = 0u64;
    // Raw sub-candles buffered for aggregated timeframes (4h ← 1h).
    let mut sub_candles: Vec<RawCandle> = Vec::new();

    while cursor < end_s {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let page_end = (cursor + page_span).min(end_s);
        let url = format!(
            "https://api.exchange.coinbase.com/products/{product}/candles?granularity={gran_secs}&start={}&end={}",
            secs_to_utc(cursor).to_rfc3339(),
            secs_to_utc(page_end).to_rfc3339(),
        );
        // Coinbase rejects requests without a User-Agent.
        let body = fetch_json_with_retry(http, &url, &[("User-Agent", "trading-bot/1.0")], cancel)
            .await
            .map_err(|e| anyhow::anyhow!("coinbase candles for {product}: {e}"))?;
        let entries = body
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("coinbase candles for {product}: {body}"))?;

        // Response is newest-first: [ time, low, high, open, close, volume ].
        let mut page: Vec<RawCandle> = Vec::with_capacity(entries.len());
        for entry in entries {
            let num = |idx: usize| -> anyhow::Result<f64> {
                entry
                    .get(idx)
                    .and_then(Value::as_f64)
                    .ok_or_else(|| anyhow::anyhow!("coinbase candle: missing field {idx}"))
            };
            let open_s = entry
                .get(0)
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("coinbase candle: missing time"))?;
            if open_s < start_s || open_s >= end_s {
                continue;
            }
            page.push((open_s, num(1)?, num(2)?, num(3)?, num(4)?, num(5)?));
        }
        page.sort_by_key(|c| c.0);

        if factor == 1 {
            let bars: Vec<CollectedBar> = page
                .iter()
                .map(|&(open_s, low, high, open, close, volume)| CollectedBar {
                    available_time: secs_to_utc(open_s + tf_secs),
                    sequence: u64::try_from(open_s * 1_000).unwrap_or(0),
                    open: dec_str(open),
                    high: dec_str(high),
                    low: dec_str(low),
                    close: dec_str(close),
                    volume: dec_str(volume),
                    trade_count: 0,
                })
                .collect();
            if !bars.is_empty() {
                store
                    .insert_collected(
                        instrument_id,
                        venue_id,
                        plan.source_name(),
                        plan.trust_tier(),
                        timeframe,
                        &bars,
                    )
                    .await?;
                total += bars.len() as u64;
                collected.fetch_add(bars.len() as u64, Ordering::Relaxed);
            }
        } else {
            sub_candles.extend(page);
        }

        cursor = page_end;
        // Stay well under Coinbase's public rate limit.
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }

    // Aggregate buffered sub-candles into the target timeframe (e.g. 4×1h → 4h).
    if factor > 1 && !sub_candles.is_empty() {
        sub_candles.sort_by_key(|c| c.0);
        let bucket_secs = gran_secs * i64::try_from(factor).unwrap_or(i64::MAX);
        let mut bars: Vec<CollectedBar> = Vec::new();
        let mut i = 0;
        while i < sub_candles.len() {
            let bucket_start = sub_candles[i].0 - sub_candles[i].0.rem_euclid(bucket_secs);
            let mut low = f64::MAX;
            let mut high = f64::MIN;
            let open = sub_candles[i].3;
            let mut close = sub_candles[i].4;
            let mut volume = 0.0;
            while i < sub_candles.len() && sub_candles[i].0 < bucket_start + bucket_secs {
                let (_, l, h, _, c, v) = sub_candles[i];
                low = low.min(l);
                high = high.max(h);
                close = c;
                volume += v;
                i += 1;
            }
            bars.push(CollectedBar {
                available_time: secs_to_utc(bucket_start + bucket_secs),
                sequence: u64::try_from(bucket_start * 1_000).unwrap_or(0),
                open: dec_str(open),
                high: dec_str(high),
                low: dec_str(low),
                close: dec_str(close),
                volume: dec_str(volume),
                trade_count: 0,
            });
        }
        store
            .insert_collected(
                instrument_id,
                venue_id,
                plan.source_name(),
                plan.trust_tier(),
                timeframe,
                &bars,
            )
            .await?;
        total += bars.len() as u64;
        collected.fetch_add(bars.len() as u64, Ordering::Relaxed);
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
async fn collect_binance(
    http: &reqwest::Client,
    store: &BarStore,
    plan: &CollectorPlan,
    symbol: &str,
    instrument_id: &str,
    venue_id: &str,
    timeframe: Timeframe,
    range: &MissingRange,
    collected: &AtomicU64,
    cancel: &AtomicBool,
) -> anyhow::Result<u64> {
    let tf_ms = i64::try_from(timeframe.seconds() * 1_000).unwrap_or(i64::MAX);
    let end_ms = range.to.timestamp_millis();
    let mut cursor_ms = range.from.timestamp_millis();
    let mut total = 0u64;

    while cursor_ms < end_ms {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let url = format!(
            "https://api.binance.com/api/v3/klines?symbol={symbol}&interval={}&startTime={cursor_ms}&endTime={end_ms}&limit=1000",
            binance_interval(timeframe)
        );
        let body = fetch_json_with_retry(http, &url, &[], cancel)
            .await
            .map_err(|e| anyhow::anyhow!("binance klines for {symbol}: {e}"))?;
        let klines: Vec<Vec<Value>> = serde_json::from_value(body)?;
        if klines.is_empty() {
            // No data listed for this stretch (pre-listing or outage): skip
            // ahead a full page so collection cannot spin in place.
            cursor_ms += tf_ms * 1_000;
            continue;
        }

        let mut bars = Vec::with_capacity(klines.len());
        let mut last_open_ms = cursor_ms;
        for k in &klines {
            // [open_time, open, high, low, close, volume, close_time, _, trades, ...]
            let open_ms = k
                .first()
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("malformed kline: missing open time"))?;
            last_open_ms = open_ms;
            let field = |idx: usize| -> anyhow::Result<String> {
                Ok(k.get(idx)
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("malformed kline field {idx}"))?
                    .to_string())
            };
            bars.push(CollectedBar {
                available_time: ms_to_utc(open_ms + tf_ms),
                sequence: u64::try_from(open_ms).unwrap_or(0),
                open: field(1)?,
                high: field(2)?,
                low: field(3)?,
                close: field(4)?,
                volume: field(5)?,
                trade_count: k.get(8).and_then(Value::as_u64).unwrap_or(0),
            });
        }

        store
            .insert_collected(
                instrument_id,
                venue_id,
                plan.source_name(),
                plan.trust_tier(),
                timeframe,
                &bars,
            )
            .await?;
        total += bars.len() as u64;
        collected.fetch_add(bars.len() as u64, Ordering::Relaxed);
        cursor_ms = last_open_ms + tf_ms;
    }
    Ok(total)
}

#[allow(clippy::too_many_arguments)]
async fn collect_alpaca(
    http: &reqwest::Client,
    store: &BarStore,
    plan: &CollectorPlan,
    symbol: &str,
    key_id: &str,
    secret: &str,
    instrument_id: &str,
    venue_id: &str,
    timeframe: Timeframe,
    range: &MissingRange,
    collected: &AtomicU64,
    cancel: &AtomicBool,
) -> anyhow::Result<u64> {
    let interval = alpaca_interval(timeframe)?;
    let tf_secs = i64::try_from(timeframe.seconds()).unwrap_or(i64::MAX);
    let mut page_token: Option<String> = None;
    let mut total = 0u64;

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut url = format!(
            "https://data.alpaca.markets/v2/stocks/{symbol}/bars?timeframe={interval}&start={}&end={}&limit=10000&adjustment=raw",
            range.from.to_rfc3339(),
            range.to.to_rfc3339(),
        );
        if let Some(token) = &page_token {
            url.push_str("&page_token=");
            url.push_str(token);
        }
        let body = fetch_json_with_retry(
            http,
            &url,
            &[("APCA-API-KEY-ID", key_id), ("APCA-API-SECRET-KEY", secret)],
            cancel,
        )
        .await
        .map_err(|e| anyhow::anyhow!("alpaca bars for {symbol}: {e}"))?;
        let empty = Vec::new();
        let raw_bars = body.get("bars").and_then(Value::as_array).unwrap_or(&empty);

        let mut bars = Vec::with_capacity(raw_bars.len());
        for b in raw_bars {
            let open_time: DateTime<Utc> = b
                .get("t")
                .and_then(Value::as_str)
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&Utc))
                .ok_or_else(|| anyhow::anyhow!("malformed alpaca bar timestamp"))?;
            // JSON numbers are stringified at this ingestion boundary and
            // parsed downstream as Decimal — no float math is performed.
            let field = |key: &str| -> anyhow::Result<String> {
                Ok(b.get(key)
                    .ok_or_else(|| anyhow::anyhow!("malformed alpaca bar field '{key}'"))?
                    .to_string())
            };
            bars.push(CollectedBar {
                available_time: open_time + chrono::Duration::seconds(tf_secs),
                sequence: u64::try_from(open_time.timestamp_millis()).unwrap_or(0),
                open: field("o")?,
                high: field("h")?,
                low: field("l")?,
                close: field("c")?,
                volume: field("v")?,
                trade_count: b.get("n").and_then(Value::as_u64).unwrap_or(0),
            });
        }

        store
            .insert_collected(
                instrument_id,
                venue_id,
                plan.source_name(),
                plan.trust_tier(),
                timeframe,
                &bars,
            )
            .await?;
        total += bars.len() as u64;
        collected.fetch_add(bars.len() as u64, Ordering::Relaxed);

        page_token = body
            .get("next_page_token")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        if page_token.is_none() {
            break;
        }
    }
    Ok(total)
}

fn ms_to_utc(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

fn secs_to_utc(s: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(s, 0).single().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coinbase_product_mapping() {
        // Coinbase products use our BASE-QUOTE ids verbatim (upper-cased).
        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "BTC-USD").unwrap();
        match &plan {
            CollectorPlan::CoinbaseCandles { product, .. } => assert_eq!(product, "BTC-USD"),
            other => panic!("expected coinbase plan, got {other:?}"),
        }

        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "eth-usdt").unwrap();
        match &plan {
            CollectorPlan::CoinbaseCandles { product, .. } => assert_eq!(product, "ETH-USDT"),
            other => panic!("expected coinbase plan, got {other:?}"),
        }

        // A separatorless symbol is rejected (ambiguous BASE/QUOTE split).
        assert!(CollectorPlan::for_asset_class("crypto_spot_cex", "BTCUSD").is_err());
    }

    #[test]
    fn coinbase_granularity_covers_all_but_seconds() {
        assert!(coinbase_granularity(Timeframe::Seconds1).is_err());
        assert_eq!(coinbase_granularity(Timeframe::Minutes15).unwrap(), (900, 1));
        // 4h is assembled from 1h candles.
        assert_eq!(coinbase_granularity(Timeframe::Hours4).unwrap(), (3600, 4));
    }

    #[test]
    fn dec_str_never_emits_scientific_notation() {
        assert_eq!(dec_str(0.000_005), "0.000005");
        assert_eq!(dec_str(79_000.5), "79000.5");
        assert_eq!(dec_str(0.0), "0");
    }

    #[test]
    fn binance_symbol_mapping() {
        // Perps/DEX still use Binance with separators stripped.
        let plan = CollectorPlan::for_asset_class("perpetual_swap", "BTC-USDT").unwrap();
        match &plan {
            CollectorPlan::BinanceKlines { symbol, .. } => assert_eq!(symbol, "BTCUSDT"),
            other => panic!("expected binance plan, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_asset_class_is_a_clear_error() {
        let err = CollectorPlan::for_asset_class("prediction_market", "TRUMP-2028").unwrap_err();
        assert!(err.to_string().contains("prediction_market"));
    }

    #[test]
    fn equity_without_credentials_fails_closed() {
        // Credentials are read from env; the test environment has none set.
        if std::env::var("ALPACA_API_KEY_ID").is_ok() {
            return; // skip in environments that do have credentials
        }
        assert!(CollectorPlan::for_asset_class("equity", "AAPL").is_err());
    }
}
