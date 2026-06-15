//! Kraken WebSocket v2 trade collector.
//!
//! Connects to `wss://ws.kraken.com/v2`, subscribes to the `trade` channel for
//! the configured symbol, and normalizes each trade update into a
//! [`domain::EventEnvelope`] with a rkyv-encoded [`domain::payloads::trade::TradePayload`].

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use domain::{
    lanes::MARKET_TRADES,
    money::{Price, Size},
    payloads::trade::{TradePayload, TradeSide},
    EventEnvelope, NormalizeError,
};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::str::FromStr;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;

use crate::gap::GapDetector;
use crate::reconnect::ReconnectPolicy;
use crate::{Collector, CollectorError};

const KRAKEN_WS_URL: &str = "wss://ws.kraken.com/v2";
const VENUE_ID: &str = "kraken";
const SOURCE: &str = "kraken_ws";

// ── Dedup identity ───────────────────────────────────────────────────────────

/// xxh3-64 dedup key: 8 bytes, no heap allocation, no SHA-1.
fn trade_dedup_key(timestamp_ns: i64, price_raw: u64, size_raw: u64) -> u64 {
    let mut buf = [0u8; 24];
    buf[0..8].copy_from_slice(&timestamp_ns.to_le_bytes());
    buf[8..16].copy_from_slice(&price_raw.to_le_bytes());
    buf[16..24].copy_from_slice(&size_raw.to_le_bytes());
    xxh3_64(&buf)
}

/// Extract a stable u64 fingerprint from a `Decimal` using its raw serialized
/// bytes (lo 32 bits + mid 32 bits → u64 little-endian).
fn decimal_to_raw_u64(d: rust_decimal::Decimal) -> u64 {
    let bytes = d.serialize(); // [u8; 16]: flags, hi, lo, mid (each 4 bytes LE)
    let lo = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let mid = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    (u64::from(mid) << 32) | u64::from(lo)
}

// ── Kraken message shapes ─────────────────────────────────────────────────────

/// Kraken trade message wrapper — borrows directly from the WS frame bytes.
#[derive(Debug, Deserialize)]
struct KrakenMessage<'a> {
    #[serde(borrow)]
    channel: Option<&'a str>,
    #[serde(rename = "type", borrow)]
    msg_type: Option<&'a str>,
    data: Option<Vec<KrakenTrade<'a>>>,
}

/// A single trade entry inside a Kraken `trade.update` message.
/// String fields borrow directly from the WS frame (no per-field allocation).
#[derive(Debug, Deserialize)]
struct KrakenTrade<'a> {
    #[allow(dead_code)]
    #[serde(borrow)]
    symbol: &'a str,
    #[serde(borrow)]
    side: &'a str,
    /// Raw price token from the WS frame.  Kraken WS **v2** sends price/qty as
    /// JSON *numbers* (e.g. `64449.5`), not the v1 strings.  Borrowing the raw
    /// JSON text and parsing it straight to `Decimal` handles either form with
    /// no f64 intermediate — preserving the repo-wide no-float convention.
    #[serde(borrow)]
    price: &'a RawValue,
    /// Raw qty token — parsed directly to Decimal (no f64 intermediate).
    #[serde(borrow)]
    qty: &'a RawValue,
    trade_id: u64,
    #[serde(borrow)]
    timestamp: &'a str,
    #[serde(default, borrow)]
    #[allow(dead_code)]
    ord_type: &'a str,
}

/// Parse a raw JSON token (number `64449.5` or quoted string `"64449.5"`) into a
/// `Decimal` without an f64 step.  Quotes are stripped so both Kraken WS v1
/// (string) and v2 (number) encodings are accepted.
fn decimal_from_raw(raw: &RawValue) -> Result<Decimal, rust_decimal::Error> {
    Decimal::from_str(raw.get().trim_matches('"'))
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

    /// Normalize a [`KrakenTrade`] into a binary [`EventEnvelope`].
    fn normalize(
        &self,
        trade: &KrakenTrade<'_>,
        raw: &[u8],
    ) -> Result<EventEnvelope, NormalizeError> {
        let price = decimal_from_raw(trade.price)
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "price".to_owned(),
                reason: e.to_string(),
            })?;

        let size = decimal_from_raw(trade.qty)
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "qty".to_owned(),
                reason: e.to_string(),
            })?;

        let side = match trade.side {
            "buy" => TradeSide::Buy,
            "sell" => TradeSide::Sell,
            _ => TradeSide::Unknown,
        };

        let exchange_trade_id = trade.trade_id.to_string();

        let timestamp_ns = chrono::DateTime::parse_from_rfc3339(trade.timestamp)
            .map(|dt| dt.timestamp_nanos_opt().unwrap_or(0))
            .unwrap_or_else(|_| Utc::now().timestamp_nanos_opt().unwrap_or(0));

        // xxh3-64 dedup key: no UUID v5, no SHA-1, no heap allocation.
        let dedup_key = trade_dedup_key(
            timestamp_ns,
            decimal_to_raw_u64(price.0),
            decimal_to_raw_u64(size.0),
        );

        let payload =
            TradePayload::with_dedup_key(price, size, side, &exchange_trade_id, dedup_key);

        let payload_bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&payload)
            .map_err(|e| NormalizeError::Deserialize(e.to_string()))?
            .into_vec();

        let envelope = EventEnvelope::new(
            domain::intern_instrument(&self.instrument_id),
            domain::intern_venue(VENUE_ID),
            domain::intern_source(SOURCE),
            trade.trade_id,
            timestamp_ns,
            payload_bytes,
        );

        let _ = raw;
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
        mut raw_prod: rtrb::Producer<domain::EventEnvelope>,
        tee_tx: tokio::sync::mpsc::UnboundedSender<domain::EventEnvelope>,
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
                        // Borrow directly from `text` — no per-field String allocations.
                        let parsed: Result<KrakenMessage<'_>, _> = serde_json::from_str(&text);
                        match parsed {
                            Err(e) => {
                                warn!(error = %e, "failed to parse Kraken message (in-process)");
                                let _ = raw;
                            }
                            Ok(km) => {
                                let is_trade_update =
                                    km.channel == Some("trade") && km.msg_type == Some("update");

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
                                            if raw_prod.push(envelope.clone()).is_err() {
                                                tracing::warn!("ring_raw full — tick dropped");
                                            }
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
                        // Borrow directly from `text` — no per-field String allocations.
                        let parsed: Result<KrakenMessage<'_>, _> = serde_json::from_str(&text);
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
                                let is_trade_update =
                                    km.channel == Some("trade") && km.msg_type == Some("update");

                                if !is_trade_update {
                                    continue;
                                }

                                let trades = km.data.unwrap_or_default();
                                for trade in &trades {
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
                                        MARKET_TRADES,
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

            warn!(
                symbol = %self.symbol,
                attempt = policy.attempt(),
                "reconnecting to Kraken WS"
            );
            policy.wait().await;
        }
    }
}
