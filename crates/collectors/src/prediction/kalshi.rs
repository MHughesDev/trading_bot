//! Kalshi prediction-market YES/NO price + perpetuals OHLCV collector.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    lanes,
    money::{Price, Size},
    payloads::bar::{BarPayload, Timeframe},
    payloads::prediction_price::PredictionPricePayload,
    EventEnvelope, NormalizeError,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

use crate::{Collector, CollectorError};

const KALSHI_REST_BASE: &str = "https://api.elections.kalshi.com/trade-api/v2";
const VENUE_ID: &str = "kalshi";
const SOURCE: &str = "kalshi_rest";

// ── Kalshi response shapes ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct MarketResponse {
    market: KalshiMarket,
}

#[derive(Debug, Deserialize)]
pub(crate) struct KalshiMarket {
    #[allow(dead_code)]
    ticker: String,
    yes_bid: Option<f64>,
    yes_ask: Option<f64>,
    no_bid: Option<f64>,
    no_ask: Option<f64>,
    volume: Option<f64>,
}

// ── Collector ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum KalshiMarketKind {
    Prediction,
    Perpetual,
}

pub struct KalshiCollector {
    pub ticker: String,
    pub instrument_id: String,
    pub kind: KalshiMarketKind,
}

impl KalshiCollector {
    pub fn new_prediction(ticker: impl Into<String>) -> Self {
        let ticker = ticker.into();
        let instrument_id = ticker.clone();
        Self {
            ticker,
            instrument_id,
            kind: KalshiMarketKind::Prediction,
        }
    }

    pub fn new_perpetual(ticker: impl Into<String>) -> Self {
        let ticker = ticker.into();
        let instrument_id = ticker.clone();
        Self {
            ticker,
            instrument_id,
            kind: KalshiMarketKind::Perpetual,
        }
    }

    pub(crate) fn normalize_prediction(
        &self,
        market: &KalshiMarket,
        seq: u64,
    ) -> Result<EventEnvelope, NormalizeError> {
        let yes_bid = market.yes_bid.unwrap_or(0.0);
        let yes_ask = market.yes_ask.unwrap_or(0.0);
        let yes_mid = (yes_bid + yes_ask) / 2.0;

        let yes_price = Decimal::from_str(&yes_mid.to_string())
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "yes_mid".to_owned(),
                reason: e.to_string(),
            })?;

        let no_mid = 1.0 - yes_mid;
        let no_price = Decimal::from_str(&no_mid.to_string())
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "no_mid".to_owned(),
                reason: e.to_string(),
            })?;

        let volume = market.volume.and_then(|v| {
            Decimal::from_str(&v.to_string())
                .ok()
                .map(Price::from_decimal)
        });

        let payload = PredictionPricePayload::new(yes_price, no_price, volume);

        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| NormalizeError::Deserialize(e.to_string()))?;

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

    pub(crate) fn normalize_perpetual_bar(
        &self,
        market: &KalshiMarket,
        seq: u64,
    ) -> Result<EventEnvelope, NormalizeError> {
        let mid_price = {
            let bid = market.no_bid.unwrap_or(0.0);
            let ask = market.no_ask.unwrap_or(0.0);
            (bid + ask) / 2.0
        };

        let price = Decimal::from_str(&mid_price.to_string())
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "mid".to_owned(),
                reason: e.to_string(),
            })?;

        let volume_raw = market.volume.unwrap_or(0.0);
        let volume = Decimal::from_str(&volume_raw.to_string())
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "volume".to_owned(),
                reason: e.to_string(),
            })?;

        let payload = BarPayload::new(Timeframe::Minutes1, price, price, price, price, volume, 0);

        let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
            .map_err(|e| NormalizeError::Deserialize(e.to_string()))?
            .into_vec();

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

#[async_trait]
impl Collector for KalshiCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let api_token = std::env::var("KALSHI_API_TOKEN").unwrap_or_default();
        let client = reqwest::Client::new();
        let mut seq: u64 = 0;

        info!(ticker = %self.ticker, "Kalshi collector starting");

        loop {
            let url = format!("{KALSHI_REST_BASE}/markets/{}", self.ticker);
            let result = client
                .get(&url)
                .header("Authorization", format!("Bearer {api_token}"))
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(error = %e, ticker = %self.ticker, "Kalshi REST request failed");
                }
                Ok(resp) => {
                    let raw = resp.bytes().await.unwrap_or_default();
                    match serde_json::from_slice::<MarketResponse>(&raw) {
                        Err(e) => {
                            let norm_err = NormalizeError::Deserialize(e.to_string());
                            if let Err(qe) =
                                quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                            {
                                warn!(error = %qe, "quarantine publish failed");
                            }
                        }
                        Ok(mr) => {
                            seq += 1;
                            match self.kind {
                                KalshiMarketKind::Prediction => {
                                    let result = self.normalize_prediction(&mr.market, seq);
                                    crate::normalizer::quarantine_or_publish(
                                        result,
                                        &raw,
                                        &self.instrument_id,
                                        "prediction.price",
                                        SOURCE,
                                        &publisher,
                                        &quarantine,
                                    )
                                    .await;
                                }
                                KalshiMarketKind::Perpetual => {
                                    let result = self.normalize_perpetual_bar(&mr.market, seq);
                                    crate::normalizer::quarantine_or_publish(
                                        result,
                                        &raw,
                                        &self.instrument_id,
                                        lanes::MARKET_BARS_1M,
                                        SOURCE,
                                        &publisher,
                                        &quarantine,
                                    )
                                    .await;
                                }
                            }
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
    use domain::payloads::prediction_price::PredictionPricePayload;

    fn sample_prediction_market() -> KalshiMarket {
        KalshiMarket {
            ticker: "BTC-ABOVE-60K".to_owned(),
            yes_bid: Some(0.52),
            yes_ask: Some(0.54),
            no_bid: Some(0.46),
            no_ask: Some(0.48),
            volume: Some(10000.0),
        }
    }

    #[test]
    fn normalize_prediction_produces_yes_no_in_range() {
        let collector = KalshiCollector::new_prediction("BTC-ABOVE-60K");
        let market = sample_prediction_market();
        let result = collector.normalize_prediction(&market, 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(
            domain::instrument_name(env.instrument_id).as_deref(),
            Some("BTC-ABOVE-60K")
        );
        let payload: PredictionPricePayload = serde_json::from_slice(&env.payload).unwrap();
        let yes: f64 = payload.yes_price.to_string().parse().unwrap();
        let no: f64 = payload.no_price.to_string().parse().unwrap();
        assert!((0.0..=1.0).contains(&yes), "yes_price out of range: {yes}");
        assert!((0.0..=1.0).contains(&no), "no_price out of range: {no}");
    }

    #[test]
    fn normalize_perpetual_bar_produces_ohlcv() {
        let collector = KalshiCollector::new_perpetual("BTC-PERP");
        let market = KalshiMarket {
            ticker: "BTC-PERP".to_owned(),
            yes_bid: None,
            yes_ask: None,
            no_bid: Some(0.49),
            no_ask: Some(0.51),
            volume: Some(50000.0),
        };
        let result = collector.normalize_perpetual_bar(&market, 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(
            domain::instrument_name(env.instrument_id).as_deref(),
            Some("BTC-PERP")
        );
    }
}
