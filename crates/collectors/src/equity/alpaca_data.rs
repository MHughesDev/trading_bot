//! Alpaca WS data feed collector for equities.
//!
//! Connects to `wss://stream.data.alpaca.markets/v2/iex`, authenticates with
//! `ALPACA_API_KEY_ID` / `ALPACA_API_SECRET_KEY`, subscribes to real-time trade
//! events, and normalizes them into [`domain::EventEnvelope`] on the same
//! `market.trades` lane as the Kraken crypto collector.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
use std::str::FromStr;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use xxhash_rust::xxh3::xxh3_64;

use crate::gap::GapDetector;
use crate::reconnect::ReconnectPolicy;
use crate::{Collector, CollectorError};

/// Free-tier IEX feed.  Use `stream.data.alpaca.markets/v2/sip` for the SIP feed.
const ALPACA_WS_URL: &str = "wss://stream.data.alpaca.markets/v2/iex";
const VENUE_ID: &str = "alpaca";
const SOURCE: &str = "alpaca_ws";

// ── Dedup identity ───────────────────────────────────────────────────────────

/// xxh3-64 dedup key: 8 bytes, no heap allocation, no SHA-1.
/// Hashes the 24-byte concatenation of timestamp_ns || price_raw || size_raw
/// (all little-endian) where price_raw and size_raw are the lower 8 bytes of
/// the serialized Decimal representation.
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
                               // bytes[0..4] = flags, [4..8] = hi, [8..12] = lo, [12..16] = mid
                               // Use lo + mid as the 64-bit fingerprint (covers the significant mantissa bits).
    let lo = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let mid = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    (u64::from(mid) << 32) | u64::from(lo)
}

// ── Alpaca message shapes ────────────────────────────────────────────────────

/// Alpaca streaming message — deserialized borrowing directly from the WS frame
/// bytes (zero-copy for `&'a str` fields; no heap allocation per tick).
#[derive(Debug, Deserialize)]
struct AlpacaMessage<'a> {
    #[serde(rename = "T", borrow)]
    msg_type: &'a str,
    #[serde(default, borrow)]
    msg: Option<&'a str>,
    #[allow(dead_code)]
    #[serde(rename = "S", default, borrow)]
    symbol: Option<&'a str>,
    /// Raw price string — parsed directly to Decimal (no f64 intermediate).
    #[serde(rename = "p", default, borrow)]
    price: Option<&'a str>,
    /// Raw size string — parsed directly to Decimal (no f64 intermediate).
    #[serde(rename = "s", default, borrow)]
    size: Option<&'a str>,
    #[serde(rename = "t", default, borrow)]
    timestamp: Option<&'a str>,
    /// Trade ID from Alpaca (not always present on IEX).
    #[serde(rename = "i", default)]
    trade_id: Option<u64>,
    /// Taker side: "B" = buy, "S" = sell, absent = unknown.
    #[serde(rename = "tks", default, borrow)]
    taker_side: &'a str,
}

// ── AlpacaDataCollector ──────────────────────────────────────────────────────

/// Per-instrument price state for tick-test side inference.
///
/// Stores `(prev_price, last_inferred_side)` keyed by instrument symbol.
/// When the venue does not provide a taker side field (`tks` absent), we infer:
/// * `price > prev_price` → `Side::Buy`
/// * `price < prev_price` → `Side::Sell`
/// * `price == prev_price` → carry forward the last inferred direction
///
/// Using `Mutex<HashMap>` for interior mutability so `normalize` can update
/// state while taking `&self` (required by the `Collector` trait bounds).
type TickTestState = Mutex<HashMap<String, (Decimal, TradeSide)>>;

/// Alpaca equity data feed connector.
pub struct AlpacaDataCollector {
    /// Symbol in Alpaca/domain format, e.g. `"AAPL"`.
    pub symbol: String,
    pub venue_id: String,
    /// Per-instrument `(prev_price, last_side)` for tick-test side inference.
    tick_state: TickTestState,
}

impl AlpacaDataCollector {
    pub fn new(symbol: impl Into<String>) -> Self {
        let symbol = symbol.into();
        Self {
            symbol,
            venue_id: VENUE_ID.to_owned(),
            tick_state: Mutex::new(HashMap::new()),
        }
    }

    /// Infer the trade side using the tick-test rule and update per-instrument state.
    ///
    /// Called only when the venue does not supply an explicit taker side.
    fn infer_side(&self, instrument: &str, price: Decimal) -> TradeSide {
        let mut state = self.tick_state.lock().unwrap_or_else(|e| e.into_inner());
        let entry = state
            .entry(instrument.to_owned())
            .or_insert((price, TradeSide::Unknown));
        let (prev_price, last_side) = *entry;
        let inferred = if price > prev_price {
            TradeSide::Buy
        } else if price < prev_price {
            TradeSide::Sell
        } else {
            // Equal price — carry forward last inferred direction.
            last_side
        };
        *entry = (price, inferred);
        inferred
    }

    fn normalize(
        &self,
        msg: &AlpacaMessage<'_>,
        raw: &[u8],
        seq: u64,
    ) -> Result<EventEnvelope, NormalizeError> {
        let price_str = msg.price.ok_or_else(|| NormalizeError::MissingField {
            field: "p".to_owned(),
        })?;
        let size_str = msg.size.ok_or_else(|| NormalizeError::MissingField {
            field: "s".to_owned(),
        })?;
        let ts_str = msg.timestamp.unwrap_or("");

        let price = Decimal::from_str(price_str)
            .map(Price::from_decimal)
            .map_err(|e| NormalizeError::InvalidPrice {
                field: "p".to_owned(),
                reason: e.to_string(),
            })?;

        let size = Decimal::from_str(size_str)
            .map(Size::from_decimal)
            .map_err(|e| NormalizeError::InvalidSize {
                field: "s".to_owned(),
                reason: e.to_string(),
            })?;

        let side = match msg.taker_side {
            "B" | "b" => TradeSide::Buy,
            "S" | "s" => TradeSide::Sell,
            // No explicit side from venue — use tick-test inference.
            _ => self.infer_side(&self.symbol, price.0),
        };

        let exchange_trade_id = msg
            .trade_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| seq.to_string());

        let timestamp_ns = chrono::DateTime::parse_from_rfc3339(ts_str)
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
            domain::intern_instrument(&self.symbol),
            domain::intern_venue(VENUE_ID),
            domain::intern_source(SOURCE),
            seq,
            timestamp_ns,
            payload_bytes,
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

            let auth_msg = serde_json::json!({
                "action": "auth",
                "key": api_key,
                "secret": api_secret,
            });
            if let Err(e) = ws_stream.send(Message::Text(auth_msg.to_string())).await {
                warn!(error = %e, "Alpaca WS auth send failed");
                policy.wait().await;
                continue;
            }

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
                        // Borrow directly from `text` — no per-field String allocations.
                        let messages: Vec<AlpacaMessage<'_>> = match serde_json::from_str(&text) {
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
                            match am.msg_type {
                                "t" => {
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
                                        MARKET_TRADES,
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
                                _ => {}
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

    fn make_msg<'a>(
        price: Option<&'a str>,
        size: Option<&'a str>,
        taker_side: &'a str,
    ) -> AlpacaMessage<'a> {
        AlpacaMessage {
            msg_type: "t",
            msg: None,
            symbol: Some("AAPL"),
            price,
            size,
            timestamp: Some("2024-01-15T14:30:00Z"),
            trade_id: Some(999),
            taker_side,
        }
    }

    #[test]
    fn normalize_valid_trade() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = make_msg(Some("150.25"), Some("100.0"), "");
        let result = collector.normalize(&msg, &[], 1);
        assert!(result.is_ok());
        let envelope = result.unwrap();
        assert_eq!(
            domain::instrument_name(envelope.instrument_id).as_deref(),
            Some("AAPL")
        );
        assert_eq!(
            domain::source_name(envelope.source_id).as_deref(),
            Some(SOURCE)
        );
    }

    #[test]
    fn normalize_missing_price_returns_error() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = make_msg(None, Some("100.0"), "");
        let result = collector.normalize(&msg, &[], 1);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            NormalizeError::MissingField { .. }
        ));
    }

    #[test]
    fn payload_is_valid_rkyv() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = AlpacaMessage {
            msg_type: "t",
            msg: None,
            symbol: Some("AAPL"),
            price: Some("150.0"),
            size: Some("50.0"),
            timestamp: Some("2024-01-15T14:30:00Z"),
            trade_id: None,
            taker_side: "",
        };
        let envelope = collector.normalize(&msg, &[], 1).unwrap();
        let trade: TradePayload = envelope.decode_payload().unwrap();
        assert_eq!(trade.price.to_string(), "150.0");
    }

    #[test]
    fn test_side_inference_buy() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = make_msg(Some("100.0"), Some("10.0"), "B");
        let envelope = collector.normalize(&msg, &[], 1).unwrap();
        let trade: TradePayload = envelope.decode_payload().unwrap();
        assert_eq!(trade.side, TradeSide::Buy);
    }

    #[test]
    fn test_side_inference_sell() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = make_msg(Some("100.0"), Some("10.0"), "S");
        let envelope = collector.normalize(&msg, &[], 1).unwrap();
        let trade: TradePayload = envelope.decode_payload().unwrap();
        assert_eq!(trade.side, TradeSide::Sell);
    }

    #[test]
    fn test_side_inference_unknown() {
        let collector = AlpacaDataCollector::new("AAPL");
        let msg = make_msg(Some("100.0"), Some("10.0"), "");
        let envelope = collector.normalize(&msg, &[], 1).unwrap();
        let trade: TradePayload = envelope.decode_payload().unwrap();
        // First trade with no prior history → Unknown
        assert_eq!(trade.side, TradeSide::Unknown);
    }

    /// Issue #47: tick-test side inference.
    ///
    /// Given a sequence of trades with ascending then descending prices (no
    /// taker-side field), the collector must infer Buy for the rising leg and
    /// Sell for the falling leg.
    #[test]
    fn tick_test_ascending_then_descending() {
        let collector = AlpacaDataCollector::new("AAPL");

        // First trade: 100 — no history, Unknown
        let t1 = make_msg(Some("100.0"), Some("10.0"), "");
        let e1 = collector.normalize(&t1, &[], 1).unwrap();
        let p1: TradePayload = e1.decode_payload().unwrap();
        assert_eq!(p1.side, TradeSide::Unknown, "first trade has no history");

        // Second trade: 101 > 100 → Buy
        let t2 = make_msg(Some("101.0"), Some("10.0"), "");
        let e2 = collector.normalize(&t2, &[], 2).unwrap();
        let p2: TradePayload = e2.decode_payload().unwrap();
        assert_eq!(p2.side, TradeSide::Buy, "ascending price → Buy");

        // Third trade: 102 > 101 → Buy
        let t3 = make_msg(Some("102.0"), Some("10.0"), "");
        let e3 = collector.normalize(&t3, &[], 3).unwrap();
        let p3: TradePayload = e3.decode_payload().unwrap();
        assert_eq!(p3.side, TradeSide::Buy, "still ascending → Buy");

        // Fourth trade: 101 < 102 → Sell
        let t4 = make_msg(Some("101.0"), Some("10.0"), "");
        let e4 = collector.normalize(&t4, &[], 4).unwrap();
        let p4: TradePayload = e4.decode_payload().unwrap();
        assert_eq!(p4.side, TradeSide::Sell, "descending price → Sell");

        // Fifth trade: 100 < 101 → Sell
        let t5 = make_msg(Some("100.0"), Some("10.0"), "");
        let e5 = collector.normalize(&t5, &[], 5).unwrap();
        let p5: TradePayload = e5.decode_payload().unwrap();
        assert_eq!(p5.side, TradeSide::Sell, "still descending → Sell");

        // Sixth trade: same price → carry forward Sell
        let t6 = make_msg(Some("100.0"), Some("10.0"), "");
        let e6 = collector.normalize(&t6, &[], 6).unwrap();
        let p6: TradePayload = e6.decode_payload().unwrap();
        assert_eq!(
            p6.side,
            TradeSide::Sell,
            "equal price carries forward last direction"
        );
    }
}
