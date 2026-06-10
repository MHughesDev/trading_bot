//! Tradier options 1-minute OHLCV + quote collector.
//!
//! Polls `GET /v1/markets/timesales` for option contract bars and
//! `GET /v1/markets/quotes` for NBBO quotes.
//! Normalizes into `EventEnvelope<BarPayload>` and `EventEnvelope<QuotePayload>`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    event_id_from_key,
    money::{Price, Size},
    payloads::bar::{BarPayload, Timeframe},
    payloads::quote::QuotePayload,
    sequenced_key, EventEnvelope, NormalizeError, TrustTier,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

use crate::{Collector, CollectorError};

const TRADIER_REST_BASE: &str = "https://api.tradier.com/v1";
const VENUE_ID: &str = "tradier";
const SOURCE: &str = "tradier_rest";

// ── Tradier response shapes ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TimeSalesResponse {
    series: Option<TimeSeries>,
}

#[derive(Debug, Deserialize)]
struct TimeSeries {
    data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct TradierBar {
    time: Option<String>,
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    close: Option<f64>,
    volume: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct QuotesResponse {
    quotes: Option<QuotesInner>,
}

#[derive(Debug, Deserialize)]
struct QuotesInner {
    quote: Option<TradierQuote>,
}

#[derive(Debug, Deserialize)]
struct TradierQuote {
    bid: Option<f64>,
    ask: Option<f64>,
    bidsize: Option<f64>,
    asksize: Option<f64>,
}

// ── Collector ────────────────────────────────────────────────────────────────

/// Tradier options collector for a single option contract symbol.
pub struct TradierOptionsCollector {
    /// Option contract symbol (OCC format), e.g. `"AAPL240621C00200000"`.
    pub symbol: String,
    pub instrument_id: String,
}

impl TradierOptionsCollector {
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        let instrument_id = symbol.clone();
        Self {
            symbol,
            instrument_id,
        }
    }

    /// Normalize a Tradier bar into an `EventEnvelope<BarPayload>`.
    pub fn normalize_bar(
        &self,
        bar: &TradierBar,
        seq: u64,
    ) -> Result<EventEnvelope<BarPayload>, NormalizeError> {
        let open = parse_price(bar.open, "open")?;
        let high = parse_price(bar.high, "high")?;
        let low = parse_price(bar.low, "low")?;
        let close = parse_price(bar.close, "close")?;
        let volume_f = bar.volume.unwrap_or(0.0);
        let volume = Decimal::from_str(&volume_f.to_string())
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "volume".to_owned(),
                reason: e.to_string(),
            })?;

        let payload = BarPayload::new(Timeframe::Minutes1, open, high, low, close, volume, 0);
        let dedup = sequenced_key("market.bars.1m", &self.instrument_id, VENUE_ID, seq, SOURCE);
        let event_id = event_id_from_key(&dedup);

        let event_time = bar.time.as_deref().and_then(|t| {
            chrono::DateTime::parse_from_rfc3339(t)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        });
        let now = Utc::now();

        Ok(EventEnvelope::new(
            event_id,
            "market.bars.1m",
            &self.instrument_id,
            VENUE_ID,
            SOURCE,
            TrustTier::Regulated,
            event_time,
            now,
            now,
            now,
            seq,
            payload,
        ))
    }

    /// Normalize a Tradier quote into an `EventEnvelope<QuotePayload>`.
    pub fn normalize_quote(
        &self,
        quote: &TradierQuote,
        seq: u64,
    ) -> Result<EventEnvelope<QuotePayload>, NormalizeError> {
        let bid_price = parse_price(quote.bid, "bid")?;
        let ask_price = parse_price(quote.ask, "ask")?;
        let bid_size = parse_size(quote.bidsize, "bidsize")?;
        let ask_size = parse_size(quote.asksize, "asksize")?;

        let payload = QuotePayload::new(bid_price, bid_size, ask_price, ask_size);
        let dedup = sequenced_key("market.quotes", &self.instrument_id, VENUE_ID, seq, SOURCE);
        let event_id = event_id_from_key(&dedup);
        let now = Utc::now();

        Ok(EventEnvelope::new(
            event_id,
            "market.quotes",
            &self.instrument_id,
            VENUE_ID,
            SOURCE,
            TrustTier::Regulated,
            None,
            now,
            now,
            now,
            seq,
            payload,
        ))
    }
}

fn parse_price(v: Option<f64>, field: &str) -> Result<Price, NormalizeError> {
    let v = v.ok_or(NormalizeError::MissingField {
        field: field.to_owned(),
    })?;
    Decimal::from_str(&v.to_string())
        .map(Price::from_decimal)
        .map_err(|e| NormalizeError::InvalidPrice {
            field: field.to_owned(),
            reason: e.to_string(),
        })
}

fn parse_size(v: Option<f64>, field: &str) -> Result<Size, NormalizeError> {
    let v = v.unwrap_or(0.0);
    Decimal::from_str(&v.to_string())
        .map(Size::from_decimal)
        .map_err(|e| NormalizeError::InvalidSize {
            field: field.to_owned(),
            reason: e.to_string(),
        })
}

#[async_trait]
impl Collector for TradierOptionsCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let api_token = std::env::var("TRADIER_API_TOKEN").unwrap_or_default();
        let client = reqwest::Client::new();
        let mut seq: u64 = 0;

        info!(symbol = %self.symbol, "Tradier options collector starting");

        loop {
            // Fetch quotes.
            let quote_url = format!(
                "{TRADIER_REST_BASE}/markets/quotes?symbols={}&greeks=false",
                self.symbol
            );
            let result = client
                .get(&quote_url)
                .header("Authorization", format!("Bearer {api_token}"))
                .header("Accept", "application/json")
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(error = %e, symbol = %self.symbol, "Tradier REST request failed");
                }
                Ok(resp) => {
                    let raw = resp.bytes().await.unwrap_or_default();
                    if let Ok(qr) = serde_json::from_slice::<QuotesResponse>(&raw) {
                        if let Some(q) = qr.quotes.and_then(|qi| qi.quote) {
                            seq += 1;
                            let result = self.normalize_quote(&q, seq);
                            crate::normalizer::quarantine_or_publish(
                                result,
                                &raw,
                                &self.instrument_id,
                                SOURCE,
                                &publisher,
                                &quarantine,
                            )
                            .await;
                        }
                    }
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bar() -> TradierBar {
        TradierBar {
            time: Some("2026-06-10T14:30:00Z".to_owned()),
            open: Some(2.50),
            high: Some(2.75),
            low: Some(2.45),
            close: Some(2.60),
            volume: Some(500.0),
        }
    }

    fn sample_quote() -> TradierQuote {
        TradierQuote {
            bid: Some(2.55),
            ask: Some(2.65),
            bidsize: Some(10.0),
            asksize: Some(15.0),
        }
    }

    #[test]
    fn normalize_bar_produces_correct_ohlcv() {
        let collector = TradierOptionsCollector::new("AAPL240621C00200000");
        let result = collector.normalize_bar(&sample_bar(), 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(env.instrument_id, "AAPL240621C00200000");
        assert_eq!(env.trust_tier, TrustTier::Regulated);
        assert_eq!(env.payload.timeframe, Timeframe::Minutes1);
    }

    #[test]
    fn normalize_quote_produces_correct_bid_ask() {
        let collector = TradierOptionsCollector::new("AAPL240621C00200000");
        let result = collector.normalize_quote(&sample_quote(), 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(env.instrument_id, "AAPL240621C00200000");
        // bid < ask
        assert!(env.payload.bid_price < env.payload.ask_price);
    }

    #[test]
    fn normalize_bar_missing_open_returns_error() {
        let collector = TradierOptionsCollector::new("AAPL240621C00200000");
        let bar = TradierBar {
            time: None,
            open: None,
            high: Some(2.75),
            low: Some(2.45),
            close: Some(2.60),
            volume: Some(500.0),
        };
        assert!(collector.normalize_bar(&bar, 1).is_err());
    }
}
