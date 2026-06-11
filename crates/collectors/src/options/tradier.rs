//! Tradier options 1-minute OHLCV + quote collector.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    lanes,
    money::{Price, Size},
    payloads::bar::{BarPayload, Timeframe},
    payloads::quote::QuotePayload,
    EventEnvelope, NormalizeError,
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
#[allow(dead_code)]
pub(crate) struct TradierBar {
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
pub(crate) struct TradierQuote {
    bid: Option<f64>,
    ask: Option<f64>,
    #[serde(rename = "bidsize")]
    bid_lot: Option<f64>,
    #[serde(rename = "asksize")]
    ask_lot: Option<f64>,
}

// ── Collector ────────────────────────────────────────────────────────────────

pub struct TradierOptionsCollector {
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

    #[allow(dead_code)]
    pub(crate) fn normalize_bar(
        &self,
        bar: &TradierBar,
        seq: u64,
    ) -> Result<EventEnvelope, NormalizeError> {
        let open = wire_to_decimal(bar.open, "open")?;
        let high = wire_to_decimal(bar.high, "high")?;
        let low = wire_to_decimal(bar.low, "low")?;
        let close = wire_to_decimal(bar.close, "close")?;
        let volume_f = bar.volume.unwrap_or(0.0);
        let volume = Decimal::from_str(&volume_f.to_string())
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "volume".to_owned(),
                reason: e.to_string(),
            })?;

        let payload = BarPayload::new(Timeframe::Minutes1, open, high, low, close, volume, 0);

        let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
            .map_err(|e| NormalizeError::Deserialize(e.to_string()))?
            .into_vec();

        let timestamp_ns = bar
            .time
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|dt| dt.timestamp_nanos_opt().unwrap_or(0))
            .unwrap_or_else(|| Utc::now().timestamp_nanos_opt().unwrap_or(0));

        Ok(EventEnvelope::new(
            domain::intern_instrument(&self.instrument_id),
            domain::intern_venue(VENUE_ID),
            domain::intern_source(SOURCE),
            seq,
            timestamp_ns,
            payload_bytes,
        ))
    }

    pub(crate) fn normalize_quote(
        &self,
        quote: &TradierQuote,
        seq: u64,
    ) -> Result<EventEnvelope, NormalizeError> {
        let bid_price = wire_to_decimal(quote.bid, "bid")?;
        let ask_price = wire_to_decimal(quote.ask, "ask")?;
        let bid_size = wire_to_lot_decimal(quote.bid_lot, "bidsize")?;
        let ask_size = wire_to_lot_decimal(quote.ask_lot, "asksize")?;

        let payload = QuotePayload::new(bid_price, bid_size, ask_price, ask_size);

        let payload_bytes =
            serde_json::to_vec(&payload).map_err(|e| NormalizeError::Deserialize(e.to_string()))?;

        let timestamp_ns = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        Ok(EventEnvelope::new(
            domain::intern_instrument(&self.instrument_id),
            domain::intern_venue(VENUE_ID),
            domain::intern_source(SOURCE),
            seq,
            timestamp_ns,
            payload_bytes,
        ))
    }
}

fn wire_to_decimal(v: Option<f64>, field: &str) -> Result<Price, NormalizeError> {
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

fn wire_to_lot_decimal(v: Option<f64>, field: &str) -> Result<Size, NormalizeError> {
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
                                lanes::MARKET_QUOTES,
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
    use domain::payloads::bar::BarPayload;
    use domain::payloads::quote::QuotePayload;

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
            bid_lot: Some(10.0),
            ask_lot: Some(15.0),
        }
    }

    #[test]
    fn normalize_bar_produces_correct_ohlcv() {
        let collector = TradierOptionsCollector::new("AAPL240621C00200000");
        let result = collector.normalize_bar(&sample_bar(), 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(
            domain::instrument_name(env.instrument_id).as_deref(),
            Some("AAPL240621C00200000")
        );
        let bar = env.decode_payload::<BarPayload>().unwrap();
        assert_eq!(bar.timeframe, Timeframe::Minutes1);
    }

    #[test]
    fn normalize_quote_produces_correct_bid_ask() {
        let collector = TradierOptionsCollector::new("AAPL240621C00200000");
        let result = collector.normalize_quote(&sample_quote(), 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        let payload: QuotePayload = serde_json::from_slice(&env.payload).unwrap();
        assert!(payload.bid_price < payload.ask_price);
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
