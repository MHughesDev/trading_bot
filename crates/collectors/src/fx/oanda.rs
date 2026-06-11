//! OANDA v20 FX 1-minute OHLCV collector (demo environment).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    lanes,
    money::{Price, Size},
    payloads::bar::{BarPayload, Timeframe},
    EventEnvelope, NormalizeError,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tracing::{info, warn};

use crate::{Collector, CollectorError};

const OANDA_REST_BASE: &str = "https://api-fxtrade.oanda.com/v3";
const VENUE_ID: &str = "oanda";
const SOURCE: &str = "oanda_rest";

// ── OANDA REST response shapes ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CandlesResponse {
    candles: Vec<OandaCandle>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OandaCandle {
    time: String,
    mid: Option<OandaMid>,
    volume: u64,
    complete: bool,
}

#[derive(Debug, Deserialize)]
struct OandaMid {
    o: String,
    h: String,
    l: String,
    c: String,
}

// ── Collector ────────────────────────────────────────────────────────────────

pub struct OandaCollector {
    pub pair: String,
    pub instrument_id: String,
    pub venue_id: String,
}

impl OandaCollector {
    pub fn new(pair: impl Into<String>) -> Self {
        let pair = pair.into();
        let instrument_id = pair.replace('_', "-");
        Self {
            pair,
            instrument_id,
            venue_id: VENUE_ID.to_owned(),
        }
    }

    pub(crate) fn normalize_bar(
        &self,
        candle: &OandaCandle,
        seq: u64,
    ) -> Result<EventEnvelope, NormalizeError> {
        let mid = candle.mid.as_ref().ok_or(NormalizeError::MissingField {
            field: "mid".to_owned(),
        })?;

        let open = parse_price(&mid.o, "open")?;
        let high = parse_price(&mid.h, "high")?;
        let low = parse_price(&mid.l, "low")?;
        let close = parse_price(&mid.c, "close")?;
        let volume = Size::from_decimal(Decimal::from(candle.volume));

        let payload = BarPayload::new(Timeframe::Minutes1, open, high, low, close, volume, 0);

        let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
            .map_err(|e| NormalizeError::Deserialize(e.to_string()))?
            .into_vec();

        let timestamp_ns = chrono::DateTime::parse_from_rfc3339(&candle.time)
            .map(|dt| dt.timestamp_nanos_opt().unwrap_or(0))
            .unwrap_or_else(|_| Utc::now().timestamp_nanos_opt().unwrap_or(0));

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

fn parse_price(s: &str, field: &str) -> Result<Price, NormalizeError> {
    Decimal::from_str(s)
        .map(Price::from_decimal)
        .map_err(|e| NormalizeError::InvalidPrice {
            field: field.to_owned(),
            reason: e.to_string(),
        })
}

#[async_trait]
impl Collector for OandaCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let api_token = std::env::var("OANDA_API_TOKEN").unwrap_or_default();
        let client = reqwest::Client::new();
        let mut seq: u64 = 0;

        info!(pair = %self.pair, "OANDA collector starting");

        loop {
            let url = format!(
                "{OANDA_REST_BASE}/instruments/{}/candles?count=2&granularity=M1&price=M",
                self.pair
            );

            let result = client
                .get(&url)
                .header("Authorization", format!("Bearer {api_token}"))
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(error = %e, pair = %self.pair, "OANDA REST request failed");
                }
                Ok(resp) => {
                    let raw = resp.bytes().await.unwrap_or_default();
                    match serde_json::from_slice::<CandlesResponse>(&raw) {
                        Err(e) => {
                            let norm_err = NormalizeError::Deserialize(e.to_string());
                            if let Err(qe) =
                                quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                            {
                                warn!(error = %qe, "quarantine publish failed");
                            }
                        }
                        Ok(cr) => {
                            for candle in &cr.candles {
                                if !candle.complete {
                                    continue;
                                }
                                seq += 1;
                                let result = self.normalize_bar(candle, seq);
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

            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::payloads::bar::BarPayload;

    fn sample_candle() -> OandaCandle {
        OandaCandle {
            time: "2026-06-10T12:00:00.000000000Z".to_owned(),
            mid: Some(OandaMid {
                o: "1.07500".to_owned(),
                h: "1.07600".to_owned(),
                l: "1.07400".to_owned(),
                c: "1.07550".to_owned(),
            }),
            volume: 1234,
            complete: true,
        }
    }

    #[test]
    fn normalize_bar_produces_correct_ohlcv() {
        let collector = OandaCollector::new("EUR_USD");
        let result = collector.normalize_bar(&sample_candle(), 1);
        assert!(result.is_ok(), "normalize_bar failed: {:?}", result);
        let env = result.unwrap();
        assert_eq!(
            domain::instrument_name(env.instrument_id).as_deref(),
            Some("EUR-USD")
        );
        let bar = env.decode_payload::<BarPayload>().unwrap();
        assert_eq!(bar.timeframe, Timeframe::Minutes1);
        let open: rust_decimal::Decimal = bar.open.to_string().parse().unwrap();
        let expected: rust_decimal::Decimal = "1.075".parse().unwrap();
        assert_eq!(open, expected, "open price mismatch");
        assert_eq!(bar.volume.to_string(), "1234");
    }

    #[test]
    fn normalize_bar_missing_mid_returns_error() {
        let collector = OandaCollector::new("EUR_USD");
        let candle = OandaCandle {
            time: "2026-06-10T12:00:00Z".to_owned(),
            mid: None,
            volume: 0,
            complete: true,
        };
        assert!(collector.normalize_bar(&candle, 1).is_err());
    }

    #[test]
    fn instrument_id_uses_dash_separator() {
        let c = OandaCollector::new("USD_JPY");
        assert_eq!(c.instrument_id, "USD-JPY");
    }
}
