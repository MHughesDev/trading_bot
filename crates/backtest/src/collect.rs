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
    /// Kraken public OHLC — unauthenticated spot crypto history.  Matches the
    /// live data venue for all crypto instruments in this system.  The pair is
    /// derived from `instrument_id` by stripping the separator and mapping
    /// `BTC` → `XBT` (Kraken's name for Bitcoin), e.g. `BTC-USD` → `XBTUSD`.
    KrakenOhlc { pair: String, source: String },
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
                // Kraken expects the pair without a separator and uses `XBT`
                // instead of `BTC`, e.g. "BTC-USD" → "XBTUSD".  The quote
                // currency is preserved exactly: USD and USDT are different
                // markets and must never be proxied.  If Kraken does not list
                // the pair the fetch fails with a clear API error.
                anyhow::ensure!(
                    instrument_id.contains('-'),
                    "instrument '{instrument_id}' must be BASE-QUOTE form (e.g. BTC-USD)"
                );
                let pair = instrument_id
                    .to_uppercase()
                    .replace('-', "")
                    .replace("BTC", "XBT");
                Ok(Self::KrakenOhlc {
                    pair,
                    source: "kraken_rest".to_string(),
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
    /// [`kraken_interval`] (no 1s), [`binance_interval`] (all timeframes),
    /// and [`alpaca_interval`] (no 1s).
    pub fn auto_collect_support(asset_class: &str, timeframe: Timeframe) -> Result<(), String> {
        match asset_class {
            "crypto_spot_cex" => match timeframe {
                // Kraken OHLC supports 1m, 5m, 15m, 1h, 4h, 1d — only 1s is absent.
                Timeframe::Seconds1 => Err(
                    "the crypto spot backfill (Kraken) does not provide 1-second bars".to_string(),
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
            Self::KrakenOhlc { source, .. } | Self::BinanceKlines { source, .. } => source,
            Self::AlpacaBars { .. } => "alpaca_rest",
        }
    }

    pub fn trust_tier(&self) -> &'static str {
        match self {
            Self::KrakenOhlc { .. } | Self::BinanceKlines { .. } => "centralized_exchange",
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
            CollectorPlan::KrakenOhlc { pair, .. } => {
                collect_kraken(
                    http,
                    store,
                    plan,
                    pair,
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

/// Kraken OHLC interval in minutes.  Supported: 1, 5, 15, 60, 240, 1440.
fn kraken_interval(tf: Timeframe) -> anyhow::Result<u32> {
    Ok(match tf {
        Timeframe::Seconds1 => anyhow::bail!("Kraken does not provide 1-second bars"),
        Timeframe::Minutes1 => 1,
        Timeframe::Minutes5 => 5,
        Timeframe::Minutes15 => 15,
        Timeframe::Hours1 => 60,
        Timeframe::Hours4 => 240,
        Timeframe::Daily => 1440,
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

#[allow(clippy::too_many_arguments)]
async fn collect_kraken(
    http: &reqwest::Client,
    store: &BarStore,
    plan: &CollectorPlan,
    pair: &str,
    instrument_id: &str,
    venue_id: &str,
    timeframe: Timeframe,
    range: &MissingRange,
    collected: &AtomicU64,
    cancel: &AtomicBool,
) -> anyhow::Result<u64> {
    let interval = kraken_interval(timeframe)?;
    let tf_secs = i64::try_from(timeframe.seconds()).unwrap_or(i64::MAX);
    let end_s = range.to.timestamp();
    // Kraken returns bars with time > since.  Subtract 1 so the first bar
    // at range.from is included.
    let mut since = range.from.timestamp() - 1;
    let mut total = 0u64;

    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let url = format!(
            "https://api.kraken.com/0/public/OHLC?pair={pair}&interval={interval}&since={since}"
        );
        let body = fetch_json_with_retry(http, &url, &[], cancel)
            .await
            .map_err(|e| anyhow::anyhow!("kraken OHLC for {pair}: {e}"))?;

        if let Some(errs) = body.get("error").and_then(Value::as_array) {
            if !errs.is_empty() {
                anyhow::bail!("kraken OHLC error for {pair}: {errs:?}");
            }
        }
        let result = body
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("kraken OHLC: missing result for {pair}"))?;

        let next_since = result
            .get("last")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MAX);

        // The pair key in the response may differ from the request (e.g.
        // XBTUSD → XXBTZUSD).  Take the first key that is not "last".
        let ohlc = result
            .as_object()
            .and_then(|m| m.iter().find(|(k, _)| *k != "last").map(|(_, v)| v))
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("kraken OHLC: no bar array for {pair}"))?;

        if ohlc.is_empty() {
            break;
        }

        let mut bars = Vec::with_capacity(ohlc.len());
        for entry in ohlc {
            // [time, open, high, low, close, vwap, volume, count] — all strings except time.
            let open_s = entry
                .get(0)
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("kraken OHLC: missing time field"))?;
            if open_s < range.from.timestamp() || open_s >= end_s {
                continue;
            }
            let field = |idx: usize| -> anyhow::Result<String> {
                Ok(entry
                    .get(idx)
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("kraken OHLC: missing field {idx}"))?
                    .to_string())
            };
            bars.push(CollectedBar {
                available_time: secs_to_utc(open_s + tf_secs),
                sequence: u64::try_from(open_s * 1_000).unwrap_or(0),
                open: field(1)?,
                high: field(2)?,
                low: field(3)?,
                close: field(4)?,
                volume: field(6)?,
                trade_count: entry.get(7).and_then(Value::as_u64).unwrap_or(0),
            });
        }

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

        // `last` from the response is the next `since` cursor.  Stop when the
        // API signals no more data, the cursor didn't advance, or we've passed
        // the end of the requested range.
        if next_since >= end_s || next_since <= since {
            break;
        }
        since = next_since;
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
    fn kraken_pair_mapping() {
        // BTC maps to XBT (Kraken's name for Bitcoin); separator is stripped.
        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "BTC-USD").unwrap();
        match &plan {
            CollectorPlan::KrakenOhlc { pair, .. } => assert_eq!(pair, "XBTUSD"),
            other => panic!("expected kraken plan, got {other:?}"),
        }

        // Lower-case input is normalised; BTC→XBT still applies.
        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "btc-usdt").unwrap();
        match &plan {
            CollectorPlan::KrakenOhlc { pair, .. } => assert_eq!(pair, "XBTUSDT"),
            other => panic!("expected kraken plan, got {other:?}"),
        }

        // ETH does not get the BTC→XBT substitution.
        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "ETH-USD").unwrap();
        match &plan {
            CollectorPlan::KrakenOhlc { pair, .. } => assert_eq!(pair, "ETHUSD"),
            other => panic!("expected kraken plan, got {other:?}"),
        }

        // A separatorless symbol is rejected (ambiguous BASE/QUOTE split).
        assert!(CollectorPlan::for_asset_class("crypto_spot_cex", "BTCUSD").is_err());
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
