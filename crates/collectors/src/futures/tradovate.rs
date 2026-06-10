//! Tradovate futures 1-minute OHLCV collector (demo environment).
//!
//! Authenticates via `POST /auth/oauthtoken`, then polls
//! `GET /md/getChart` for 1-minute bars.  Normalizes into
//! `EventEnvelope<BarPayload>` with `AssetClass::FuturesExpiring` and
//! `TrustTier::Regulated` (futures are exchange-regulated).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    event_id_from_key,
    money::{Price, Size},
    payloads::bar::{BarPayload, Timeframe},
    sequenced_key, EventEnvelope, NormalizeError, TrustTier,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tracing::{info, warn};

use crate::{Collector, CollectorError};

const TRADOVATE_DEMO_BASE: &str = "https://demo.tradovateapi.com/v1";
const VENUE_ID: &str = "tradovate";
const SOURCE: &str = "tradovate_rest";

// ── Tradovate response shapes ────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct AuthRequest<'a> {
    name: &'a str,
    password: &'a str,
    #[serde(rename = "appId")]
    app_id: &'a str,
    #[serde(rename = "appVersion")]
    app_version: &'a str,
    #[serde(rename = "cid")]
    cid: &'a str,
    sec: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChartResponse {
    bars: Option<Vec<TradovateBar>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TradovateBar {
    timestamp: Option<String>,
    open: Option<f64>,
    high: Option<f64>,
    low: Option<f64>,
    close: Option<f64>,
    up_volume: Option<f64>,
    down_volume: Option<f64>,
}

// ── Collector ────────────────────────────────────────────────────────────────

/// Tradovate futures 1-minute OHLCV collector.
pub struct TradovateCollector {
    /// Tradovate contract symbol, e.g. `"ESH4"` (E-mini S&P 500 March 2024).
    pub symbol: String,
    pub instrument_id: String,
}

impl TradovateCollector {
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        let instrument_id = symbol.clone();
        Self {
            symbol,
            instrument_id,
        }
    }

    /// Normalize a Tradovate bar into an `EventEnvelope<BarPayload>`.
    pub(crate) fn normalize_bar(
        &self,
        bar: &TradovateBar,
        seq: u64,
    ) -> Result<EventEnvelope<BarPayload>, NormalizeError> {
        let open = wire_to_decimal(bar.open, "open")?;
        let high = wire_to_decimal(bar.high, "high")?;
        let low = wire_to_decimal(bar.low, "low")?;
        let close = wire_to_decimal(bar.close, "close")?;

        let up_vol = bar.up_volume.unwrap_or(0.0);
        let down_vol = bar.down_volume.unwrap_or(0.0);
        let total_vol = up_vol + down_vol;
        let volume = Decimal::from_str(&total_vol.to_string())
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "volume".to_owned(),
                reason: e.to_string(),
            })?;

        let payload = BarPayload::new(Timeframe::Minutes1, open, high, low, close, volume, 0);
        let dedup = sequenced_key("market.bars.1m", &self.instrument_id, VENUE_ID, seq, SOURCE);
        let event_id = event_id_from_key(&dedup);

        let event_time = bar.timestamp.as_deref().and_then(|t| {
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

#[async_trait]
impl Collector for TradovateCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let username = std::env::var("TRADOVATE_USERNAME").unwrap_or_default();
        let password = std::env::var("TRADOVATE_PASSWORD").unwrap_or_default();
        let app_id = std::env::var("TRADOVATE_APP_ID").unwrap_or_default();
        let app_version = std::env::var("TRADOVATE_APP_VERSION").unwrap_or_else(|_| "1.0".into());
        let cid = std::env::var("TRADOVATE_CID").unwrap_or_default();
        let sec = std::env::var("TRADOVATE_SEC").unwrap_or_default();

        let client = reqwest::Client::new();
        let mut seq: u64 = 0;

        info!(symbol = %self.symbol, "Tradovate collector starting");

        // Obtain access token.
        let auth_body = AuthRequest {
            name: &username,
            password: &password,
            app_id: &app_id,
            app_version: &app_version,
            cid: &cid,
            sec: &sec,
        };

        let auth_resp = client
            .post(format!("{TRADOVATE_DEMO_BASE}/auth/oauthtoken"))
            .json(&auth_body)
            .send()
            .await
            .map_err(|e| CollectorError::Connect(e.to_string()))?;

        let auth_data: AuthResponse = auth_resp
            .json()
            .await
            .map_err(|e| CollectorError::Connect(e.to_string()))?;

        let access_token = auth_data.access_token.unwrap_or_default();

        loop {
            let url = format!(
                "{TRADOVATE_DEMO_BASE}/md/getChart?symbol={}&chartDescription={{\"underlyingType\":\"MinuteBar\",\"elementSize\":1,\"elementSizeUnit\":\"UnderlyingUnits\"}}",
                self.symbol
            );

            let result = client
                .get(&url)
                .header("Authorization", format!("Bearer {access_token}"))
                .send()
                .await;

            match result {
                Err(e) => {
                    warn!(error = %e, symbol = %self.symbol, "Tradovate REST request failed");
                }
                Ok(resp) => {
                    let raw = resp.bytes().await.unwrap_or_default();
                    match serde_json::from_slice::<ChartResponse>(&raw) {
                        Err(e) => {
                            let norm_err = NormalizeError::Deserialize(e.to_string());
                            if let Err(qe) =
                                quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                            {
                                warn!(error = %qe, "quarantine publish failed");
                            }
                        }
                        Ok(cr) => {
                            for bar in cr.bars.unwrap_or_default().iter() {
                                seq += 1;
                                let result = self.normalize_bar(bar, seq);
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
            }

            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bar() -> TradovateBar {
        TradovateBar {
            timestamp: Some("2026-06-10T15:30:00Z".to_owned()),
            open: Some(5320.25),
            high: Some(5325.50),
            low: Some(5318.75),
            close: Some(5323.00),
            up_volume: Some(1500.0),
            down_volume: Some(800.0),
        }
    }

    #[test]
    fn normalize_bar_produces_correct_ohlcv() {
        let collector = TradovateCollector::new("ESH4");
        let result = collector.normalize_bar(&sample_bar(), 1);
        assert!(result.is_ok(), "{:?}", result);
        let env = result.unwrap();
        assert_eq!(env.instrument_id, "ESH4");
        assert_eq!(env.venue_id, "tradovate");
        assert_eq!(env.trust_tier, TrustTier::Regulated);
        assert_eq!(env.payload.timeframe, Timeframe::Minutes1);
        // Volume = up + down
        let vol: f64 = env.payload.volume.to_string().parse().unwrap();
        assert!((vol - 2300.0).abs() < 0.01, "volume mismatch: {vol}");
    }

    #[test]
    fn normalize_bar_missing_open_returns_error() {
        let collector = TradovateCollector::new("ESH4");
        let bar = TradovateBar {
            timestamp: None,
            open: None,
            high: Some(5325.0),
            low: Some(5318.0),
            close: Some(5323.0),
            up_volume: Some(1000.0),
            down_volume: Some(500.0),
        };
        assert!(collector.normalize_bar(&bar, 1).is_err());
    }

    #[test]
    fn instrument_id_matches_symbol() {
        let c = TradovateCollector::new("NQH4");
        assert_eq!(c.instrument_id, "NQH4");
    }
}
