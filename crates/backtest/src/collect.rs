//! Automated historical data collection ("speed-run" backfill).
//!
//! Driven by the data requirements of the strategy under test: the manager
//! computes the missing ranges for the strategy's timeframe (plus indicator
//! warm-up) and this module fills them from a venue REST API in paged
//! 1000-bar requests, writing straight into the platform's ClickHouse store.
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
    /// Binance public klines — fast, unauthenticated crypto history.
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
    /// Crypto uses Binance's public klines API (deep history, 1000 bars per
    /// page, no credentials).  Equities/ETFs use Alpaca's data API with the
    /// same credentials the live collector uses (`ALPACA_API_KEY_ID` /
    /// `ALPACA_API_SECRET_KEY`).
    pub fn for_asset_class(asset_class: &str, instrument_id: &str) -> anyhow::Result<Self> {
        match asset_class {
            "crypto_spot_cex" | "crypto_spot_dex" | "perpetual_swap" => {
                // "BTC-USDT" → "BTCUSDT"; plain-USD pairs are proxied to the
                // USDT market, which Binance actually lists.
                let compact: String = instrument_id
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect();
                let symbol = if compact.ends_with("USD") {
                    format!("{compact}T")
                } else {
                    compact
                };
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

    pub fn source_name(&self) -> &str {
        match self {
            Self::BinanceKlines { source, .. } => source,
            Self::AlpacaBars { .. } => "alpaca_rest",
        }
    }

    pub fn trust_tier(&self) -> &'static str {
        match self {
            Self::BinanceKlines { .. } => "centralized_exchange",
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
            CollectorPlan::BinanceKlines { symbol, .. } => {
                collect_binance(
                    http, store, plan, symbol, instrument_id, venue_id, timeframe, range,
                    collected, cancel,
                )
                .await?
            }
            CollectorPlan::AlpacaBars {
                symbol,
                key_id,
                secret,
            } => {
                collect_alpaca(
                    http, store, plan, symbol, key_id, secret, instrument_id, venue_id,
                    timeframe, range, collected, cancel,
                )
                .await?
            }
        };
    }
    Ok(total)
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
    let tf_ms = (timeframe.seconds() * 1_000) as i64;
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
        let resp = http.get(&url).send().await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "binance klines request failed for {symbol}: HTTP {}",
            resp.status()
        );
        let klines: Vec<Vec<Value>> = resp.json().await?;
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
                sequence: open_ms as u64,
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
    let tf_secs = timeframe.seconds() as i64;
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
            url.push_str(&format!("&page_token={token}"));
        }
        let resp = http
            .get(&url)
            .header("APCA-API-KEY-ID", key_id)
            .header("APCA-API-SECRET-KEY", secret)
            .send()
            .await?;
        anyhow::ensure!(
            resp.status().is_success(),
            "alpaca bars request failed for {symbol}: HTTP {}",
            resp.status()
        );
        let body: Value = resp.json().await?;
        let empty = Vec::new();
        let raw_bars = body
            .get("bars")
            .and_then(Value::as_array)
            .unwrap_or(&empty);

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
                sequence: open_time.timestamp_millis() as u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binance_symbol_mapping() {
        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "BTC-USDT").unwrap();
        match &plan {
            CollectorPlan::BinanceKlines { symbol, .. } => assert_eq!(symbol, "BTCUSDT"),
            CollectorPlan::AlpacaBars { .. } => panic!("expected binance plan"),
        }

        // Plain-USD pairs proxy to the USDT market.
        let plan = CollectorPlan::for_asset_class("crypto_spot_cex", "BTC-USD").unwrap();
        match &plan {
            CollectorPlan::BinanceKlines { symbol, .. } => assert_eq!(symbol, "BTCUSDT"),
            CollectorPlan::AlpacaBars { .. } => panic!("expected binance plan"),
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
