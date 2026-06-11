//! Kraken WebSocket v2 trade collector.
//!
//! Connects to `wss://ws.kraken.com/v2`, subscribes to the `trade` channel for
//! the configured symbol, and normalizes each trade update into a
//! [`domain::payloads::TradePayload`] wrapped in an [`domain::EventEnvelope`].

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

const KRAKEN_WS_URL: &str = "wss://ws.kraken.com/v2";
const VENUE_ID: &str = "kraken";
const SOURCE: &str = "kraken_ws";

/// Kraken trade message wrapper.
#[derive(Debug, Deserialize)]
struct KrakenMessage {
    channel: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    data: Option<Vec<KrakenTrade>>,
}

/// A single trade entry inside a Kraken `trade.update` message.
#[derive(Debug, Deserialize)]
struct KrakenTrade {
    #[allow(dead_code)]
    symbol: String,
    side: String,
    price: String,
    qty: String,
    trade_id: u64,
    timestamp: String,
    #[serde(default)]
    #[allow(dead_code)]
    ord_type: String,
}

/// Kraken WS v2 connector for trade events.
pub struct KrakenCollector {
    /// Symbol in Kraken format, e.g. `"BTC/USD"`.
    pub symbol: String,
    /// Symbol in domain format, e.g. `"BTC-USD"`.
    pub instrument_id: String,
    /// Always `"kraken"`.
    pub venue_id: String,
}

impl KrakenCollector {
    /// Create a new collector.  `symbol` is the Kraken symbol (e.g. `"BTC/USD"`).
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        let instrument_id = symbol.replace('/', "-");
        Self {
            symbol,
            instrument_id,
            venue_id: VENUE_ID.to_owned(),
        }
    }

    /// Normalize a [`KrakenTrade`] into a domain `EventEnvelope<TradePayload>`.
    fn normalize(
        &self,
        trade: &KrakenTrade,
        raw: &[u8],
    ) -> Result<EventEnvelope<TradePayload>, NormalizeError> {
        let price = Decimal::from_str(&trade.price)
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "price".to_owned(),
                reason: e.to_string(),
            })?;

        let size = Decimal::from_str(&trade.qty)
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "qty".to_owned(),
                reason: e.to_string(),
            })?;

        let side = match trade.side.as_str() {
            "buy" => TradeSide::Buy,
            "sell" => TradeSide::Sell,
            _ => TradeSide::Unknown,
        };

        let exchange_trade_id = trade.trade_id.to_string();
        let payload = TradePayload::new(price, size, side, &exchange_trade_id);

        let dedup = trade_key(VENUE_ID, &exchange_trade_id);
        let event_id = event_id_from_key(&dedup);

        let event_time = chrono::DateTime::parse_from_rfc3339(&trade.timestamp)
            .ok()
            .map(|dt| dt.with_timezone(&Utc));

        let now = Utc::now();

        let envelope = EventEnvelope::new(
            event_id,
            MARKET_TRADES,
            &self.instrument_id,
            VENUE_ID,
            SOURCE,
            TrustTier::CentralizedExchange,
            event_time,
            now,
            now,
            now,
            trade.trade_id,
            payload,
        );

        let _ = raw; // raw bytes passed through for quarantine if needed
        Ok(envelope)
    }
}

impl KrakenCollector {
    /// Run the collector in-process, writing normalized ticks into an rtrb ring.
    ///
    /// Each tick is also forwarded to `tee_tx` so the tee task can persist it
    /// to JetStream asynchronously without blocking the hot path.
    pub async fn run_in_process(
        &self,
        mut raw_prod: rtrb::Producer<domain::EventEnvelope<domain::payloads::trade::TradePayload>>,
        tee_tx: tokio::sync::mpsc::UnboundedSender<
            domain::EventEnvelope<domain::payloads::trade::TradePayload>,
        >,
    ) -> Result<(), CollectorError> {
        let mut policy = ReconnectPolicy::default();
        let mut gap_detector = GapDetector::new(&self.instrument_id, MARKET_TRADES);

        loop {
            info!(symbol = %self.symbol, "connecting to Kraken WS (in-process)");

            let ws_result = connect_async(KRAKEN_WS_URL).await;
            let (mut ws_stream, _) = match ws_result {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "Kraken WS connect failed, retrying");
                    policy.wait().await;
                    continue;
                }
            };

            let subscribe_msg = serde_json::json!({
                "method": "subscribe",
                "params": { "channel": "trade", "symbol": [self.symbol] },
                "req_id": 1
            });
            if let Err(e) = ws_stream
                .send(Message::Text(subscribe_msg.to_string()))
                .await
            {
                warn!(error = %e, "failed to send subscribe message");
                policy.wait().await;
                continue;
            }

            policy.reset();
            gap_detector.reset();
            info!(symbol = %self.symbol, "subscribed to Kraken trade channel (in-process)");

            loop {
                let msg = ws_stream.next().await;
                match msg {
                    None => {
                        warn!(symbol = %self.symbol, "Kraken WS stream ended");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "Kraken WS read error");
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        let raw = text.as_bytes().to_vec();
                        let parsed: Result<KrakenMessage, _> = serde_json::from_str(&text);
                        match parsed {
                            Err(e) => {
                                warn!(error = %e, "failed to parse Kraken message (in-process)");
                                let _ = raw;
                            }
                            Ok(km) => {
                                let is_trade_update = km.channel.as_deref() == Some("trade")
                                    && km.msg_type.as_deref() == Some("update");

                                if !is_trade_update {
                                    continue;
                                }

                                for trade in km.data.unwrap_or_default() {
                                    if let Some(gap) = gap_detector.check(trade.trade_id) {
                                        warn!(
                                            instrument_id = %gap.instrument_id,
                                            lane = %gap.lane,
                                            expected = gap.expected,
                                            got = gap.got,
                                            "sequence gap detected"
                                        );
                                    }

                                    match self.normalize(&trade, &raw) {
                                        Ok(envelope) => {
                                            // Push to hot-path ring; drop on full rather than block.
                                            if raw_prod.push(envelope.clone()).is_err() {
                                                tracing::warn!("ring_raw full — tick dropped");
                                            }
                                            // Best-effort tee to JetStream.
                                            let _ = tee_tx.send(envelope);
                                        }
                                        Err(e) => {
                                            warn!(error = %e, "normalize failed (in-process)");
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = ws_stream.send(Message::Pong(data)).await {
                            warn!(error = %e, "failed to send pong");
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!(symbol = %self.symbol, "Kraken WS closed by server (in-process)");
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }

            warn!(
                symbol = %self.symbol,
                attempt = policy.attempt(),
                "reconnecting to Kraken WS (in-process)"
            );
            policy.wait().await;
        }
    }
}

#[async_trait]
impl Collector for KrakenCollector {
    async fn run(
        &self,
        publisher: Arc<event_bus::Publisher>,
        quarantine: Arc<event_bus::QuarantinePublisher>,
    ) -> Result<(), CollectorError> {
        let mut policy = ReconnectPolicy::default();
        let mut gap_detector = GapDetector::new(&self.instrument_id, MARKET_TRADES);

        loop {
            info!(symbol = %self.symbol, "connecting to Kraken WS");

            let ws_result = connect_async(KRAKEN_WS_URL).await;
            let (mut ws_stream, _) = match ws_result {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(error = %e, "Kraken WS connect failed, retrying");
                    policy.wait().await;
                    continue;
                }
            };

            // Send subscribe message.
            let subscribe_msg = serde_json::json!({
                "method": "subscribe",
                "params": {
                    "channel": "trade",
                    "symbol": [self.symbol]
                },
                "req_id": 1
            });
            let subscribe_text = subscribe_msg.to_string();
            if let Err(e) = ws_stream.send(Message::Text(subscribe_text)).await {
                warn!(error = %e, "failed to send subscribe message");
                policy.wait().await;
                continue;
            }

            policy.reset();
            gap_detector.reset();
            info!(symbol = %self.symbol, "subscribed to Kraken trade channel");

            // Read messages.
            loop {
                let msg = ws_stream.next().await;
                match msg {
                    None => {
                        warn!(symbol = %self.symbol, "Kraken WS stream ended");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(error = %e, "Kraken WS read error");
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        let raw = text.as_bytes().to_vec();
                        let parsed: Result<KrakenMessage, _> = serde_json::from_str(&text);
                        match parsed {
                            Err(e) => {
                                warn!(error = %e, "failed to parse Kraken message");
                                let norm_err = NormalizeError::Deserialize(e.to_string());
                                if let Err(qe) =
                                    quarantine.publish_failure(&raw, &norm_err, SOURCE).await
                                {
                                    warn!(error = %qe, "quarantine publish failed");
                                }
                            }
                            Ok(km) => {
                                let is_trade_update = km.channel.as_deref() == Some("trade")
                                    && km.msg_type.as_deref() == Some("update");

                                if !is_trade_update {
                                    continue;
                                }

                                let trades = km.data.unwrap_or_default();
                                for trade in &trades {
                                    // Gap detection using trade_id as sequence.
                                    if let Some(gap) = gap_detector.check(trade.trade_id) {
                                        warn!(
                                            instrument_id = %gap.instrument_id,
                                            lane = %gap.lane,
                                            expected = gap.expected,
                                            got = gap.got,
                                            "sequence gap detected"
                                        );
                                    }

                                    let result = self.normalize(trade, &raw);
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
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = ws_stream.send(Message::Pong(data)).await {
                            warn!(error = %e, "failed to send pong");
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!(symbol = %self.symbol, "Kraken WS closed by server");
                        break;
                    }
                    Some(Ok(_)) => {}
                }
            }

            // Reconnect with backoff.
            warn!(
                symbol = %self.symbol,
                attempt = policy.attempt(),
                "reconnecting to Kraken WS"
            );
            policy.wait().await;
        }
    }
}
