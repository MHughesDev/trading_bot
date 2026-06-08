//! Alpaca WS data feed collector for equities.
//!
//! Connects to `wss://stream.data.alpaca.markets/v2/iex`, authenticates with
//! `ALPACA_API_KEY_ID` / `ALPACA_API_SECRET_KEY`, subscribes to real-time trade
//! events, and normalizes them into `EventEnvelope<TradePayload>` on the same
//! `market.trades` lane as the Kraken crypto collector — with zero changes to
//! any downstream consumer.
//!
//! Deliberately structured differently from the Kraken collector to prove that
//! the lane abstraction is venue-agnostic (Phase 6 requirement).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    event_id_from_key,
    lanes::MARKET_TRADES,
    money::{Price, Size},
    payloads::trade::{TradePayload, TradeSide},
    trade_key, EventEnvelope, NormalizeError, TrustTier,
};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

use crate::gap::GapDetector;
use crate::reconnect::ReconnectPolicy;
use crate::{Collector, CollectorError};

/// Free-tier IEX feed.  Use `stream.data.alpaca.markets/v2/sip` for the SIP feed.
const ALPACA_WS_URL: &str = "wss://stream.data.alpaca.markets/v2/iex";
const VENUE_ID: &str = "alpaca";
const SOURCE: &str = "alpaca_ws";

// ── Alpaca message shapes ────────────────────────────────────────────────────

/// Alpaca streaming message — deserialized from the array envelopes Alpaca sends.
#[derive(Debug, Deserialize)]
struct AlpacaMessage {
    #[serde(rename = "T")]
    msg_type: String,
    #[serde(default)]
    msg: Option<String>,
    #[serde(rename = "S")]
    symbol: Option<String>,
    #[serde(rename = "p")]
    price: Option<f64>,
    #[serde(rename = "s")]
    size: Option<f64>,
    #[serde(rename = "t")]
    timestamp: Option<String>,
    /// Trade ID from Alpaca (not always present on IEX).
    #[serde(rename = "i", default)]
    trade_id: Option<u64>,
}

// ── AlpacaDataCollector ──────────────────────────────────────────────────────

/// Alpaca equity data feed connector.
///
/// Publishes `TrustTier::Regulated` trade events (equities are exchange-regulated
/// vs `CentralizedExchange` for crypto) on the same `market.trades` lane.
pub struct AlpacaDataCollector {
    /// Symbol in Alpaca/domain format, e.g. `"AAPL"`.
    pub symbol: String,
    pub venue_id: String,
}

impl AlpacaDataCollector {
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        Self {
            symbol,
            venue_id: VENUE_ID.to_owned(),
        }
    }

    fn normalize(
        &self,
        msg: &AlpacaMessage,
        raw: &[u8],
        seq: u64,
    ) -> Result<EventEnvelope<TradePayload>, NormalizeError> {
        let price_f = msg.price.ok_or_else(|| NormalizeError::MissingField {
            field: "p".to_owned(),
        })?;
        let size_f = msg.size.ok_or_else(|| NormalizeError::MissingField {
            field: "s".to_owned(),
        })?;
        let ts_str = msg.timestamp.as_deref().unwrap_or("");

        let price = Decimal::from_str(&price_f.to_string())
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "p".to_owned(),
                reason: e.to_string(),
            })?;

        let size = Decimal::from_str(&size_f.to_string())
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "s".to_owned(),
                reason: e.to_string(),
            })?;

        // Alpaca trade messages carry no aggressor side; equity trades are bilateral.
        let side = TradeSide::Unknown;

        let exchange_trade_id = msg
            .trade_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| seq.to_string());

        let payload = TradePayload::new(price, size, side, &exchange_trade_id);

        let dedup = trade_key(VENUE_ID, &exchange_trade_id);
        let event_id = event_id_from_key(&dedup);

        let event_time = chrono::DateTime::parse_from_rfc3339(ts_str)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));

        let now = Utc::now();

        let envelope = EventEnvelope::new(
            event_id,
            MARKET_TRADES,
            &self.symbol,
            VENUE_ID,
            SOURCE,
            TrustTier::Regulated, // Equity events carry Regulated trust tier
            event_time,
            now,
            now,
            now,
            seq,
            payload,
        );

        let _ = raw;
        Ok(envelope)
    }
}

#[async_trait]
impl Collector for AlpacaDataCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let api_key = std::env::var("ALPACA_API_KEY_ID").unwrap_or_default();
        let api_secret = std::env::var("ALPACA_API_SECRET_KEY").unwrap_or_default();

        let mut policy = ReconnectPolicy::default();
        let mut gap_detector = GapDetector::new(&self.symbol, MARKET_TRADES);
        let mut seq: u64 = 0;

        loop {
            info!(symbol = %self.symbol, "connecting to Alpaca WS data feed");

            let (mut ws_stream, _) = match connect_async(ALPACA_WS_URL).await {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "Alpaca WS connect failed, retrying");
                    policy.wait().await;
                    continue;
                }
            };

            // Authenticate.
            let auth_msg = serde_json::json!({
                "action": "auth",
                "key": api_key,
                "secret": api_secret,
            });
            if let Err(e) = ws_stream
                .send(Message::Text(auth_msg.to_string()))
                .await
            {
                warn!(error = %e, "Alpaca WS auth send failed");
                policy.wait().await;
                continue;
            }

            // Subscribe to trade events for this symbol.
            let subscribe_msg = serde_json::json!({
                "action": "subscribe",
                "trades": [self.symbol],
            });
            if let Err(e) = ws_stream
                .send(Message::Text(subscribe_msg.to_string()))
                .await
            {
                warn!(error = %e, "Alpaca WS subscribe send failed");
                policy.wait().await;
                continue;
            }

            policy.reset();
            gap_detector.reset();
            info!(symbol = %self.symbol, "subscribed to Alpaca trades");

            loop {
                let msg = ws_stream.next().await;
                match msg {
                    None => {
                        warn!(symbol = %self.symbol, "Alpaca WS stream ended");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "Alpaca WS read error");
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        let raw = text.as_bytes().to_vec();
                        // Alpaca sends JSON arrays of message objects.
                        let messages: Vec<AlpacaMessage> = match serde_json::from_str(&text) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!(error = %e, "failed to parse Alpaca message array");
                                let norm_err = NormalizeError::Deserialize(e.to_string());
                                if let Err(qe) =
                                    quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                                {
                                    warn!(error = %qe, "quarantine publish failed");
                                }
                                continue;
                            }
                        };

                        for am in &messages {
                            match am.msg_type.as_str() {
                                "t" => {
                                    // Trade event.
                                    seq += 1;
                                    if let Some(gap) = gap_detector.check(seq) {
                                        warn!(
                                            instrument_id = %gap.instrument_id,
                                            lane = %gap.lane,
                                            expected = gap.expected,
                                            got = gap.got,
                                            "sequence gap detected"
                                        );
                                    }
                                    let result = self.normalize(am, &raw, seq);
                                    crate::normalizer::quarantine_or_publish(
                                        result,
                                        &raw,
                                        &self.symbol,
                                        SOURCE,
                                        &publisher,
                                        &quarantine,
                                    )
                                    .await;
                                }
                                "success" => {
                                    info!(
                                        symbol = %self.symbol,
                                        msg = ?am.msg,
                                        "Alpaca WS: connected/authenticated"
                                    );
                                }
                                "error" => {
                                    warn!(
                                        symbol = %self.symbol,
                                        msg = ?am.msg,
                                        "Alpaca WS error"
                                    );
                                }
                                _ => {} // subscription confirmations and other bookkeeping
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = ws_stream.send(Message::Pong(data)).await {
                            warn!(error = %e, "failed to send pong");
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!(symbol = %self.symbol, "Alpaca WS closed by server");
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }

            warn!(
                symbol = %self.symbol,
                attempt = policy.attempt(),
                "reconnecting to Alpaca WS"
            );
            policy.wait().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_valid_trade() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = AlpacaMessage {
            msg_type: "t".into(),
            msg: None,
            symbol: Some("AAPL".into()),
            price: Some(150.25),
            size: Some(100.0),
            timestamp: Some("2024-01-15T14:30:00Z".into()),
            trade_id: Some(999),
        };
        let result = collector.normalize(&msg, &[], 1);
        assert!(result.is_ok());
        let envelope = result.unwrap();
        assert_eq!(envelope.instrument_id, "AAPL");
        assert_eq!(envelope.source, SOURCE);
        assert_eq!(envelope.trust_tier, TrustTier::Regulated);
    }

    #[test]
    fn normalize_missing_price_returns_error() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = AlpacaMessage {
            msg_type: "t".into(),
            msg: None,
            symbol: Some("AAPL".into()),
            price: None,
            size: Some(100.0),
            timestamp: Some("2024-01-15T14:30:00Z".into()),
            trade_id: Some(999),
        };
        let result = collector.normalize(&msg, &[], 1);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), NormalizeError::MissingField { .. }));
    }

    #[test]
    fn trust_tier_is_regulated() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = AlpacaMessage {
            msg_type: "t".into(),
            msg: None,
            symbol: Some("AAPL".into()),
            price: Some(150.0),
            size: Some(50.0),
            timestamp: Some("2024-01-15T14:30:00Z".into()),
            trade_id: None,
        };
        let envelope = collector.normalize(&msg, &[], 1).unwrap();
        // Equity events must carry Regulated trust tier (higher than CentralizedExchange).
        assert_eq!(envelope.trust_tier, TrustTier::Regulated);
    }
}
