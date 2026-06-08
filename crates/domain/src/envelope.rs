//! `EventEnvelope<T>` — the universal wrapper for every event on the bus.
//!
//! The envelope is **immutable once published**.  Late data produces a *new*
//! envelope (with a new `event_id`, new `available_time`) on the revised lane;
//! it never mutates an already-published envelope.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::payloads::Payload;
use crate::trust::TrustTier;

/// The universal event wrapper.
///
/// `T` is the concrete payload type; use `AnyPayload` when the type is not
/// known at compile time.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(bound = "T: serde::Serialize + serde::de::DeserializeOwned")]
pub struct EventEnvelope<T: Payload> {
    /// Deterministic UUID derived from the dedup key (UUID v5).
    pub event_id: Uuid,
    /// Dotted event-type string (from `T::event_type()`).
    pub event_type: String,
    /// Payload schema version (from `T::schema_version()`).
    pub schema_version: String,
    /// NATS lane this event was published on.
    pub lane: String,
    pub instrument_id: String,
    pub venue_id: String,
    /// Identifier for the data source / collector (e.g. `"kraken_ws"`).
    pub source: String,
    pub trust_tier: TrustTier,

    /// When the source says this event happened (optional).
    pub event_time: Option<DateTime<Utc>>,
    /// When the collector received the raw bytes.
    pub observed_time: DateTime<Utc>,
    /// When the normalized envelope entered the NATS bus.
    pub ingested_time: DateTime<Utc>,
    /// When downstream consumers (strategies, features) are allowed to use this.
    /// This is the **replay sort key**.
    pub available_time: DateTime<Utc>,

    /// Monotonically increasing per-lane sequence number from the source.
    pub sequence: u64,
    /// Links this event to a causal chain (e.g. the order that triggered a fill).
    pub correlation_id: Option<Uuid>,
    /// The event that directly caused this one.
    pub causation_id: Option<Uuid>,

    pub payload: T,
}

impl<T: Payload> EventEnvelope<T> {
    /// Convenience constructor.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: Uuid,
        lane: impl Into<String>,
        instrument_id: impl Into<String>,
        venue_id: impl Into<String>,
        source: impl Into<String>,
        trust_tier: TrustTier,
        event_time: Option<DateTime<Utc>>,
        observed_time: DateTime<Utc>,
        ingested_time: DateTime<Utc>,
        available_time: DateTime<Utc>,
        sequence: u64,
        payload: T,
    ) -> Self {
        Self {
            event_id,
            event_type: T::event_type().into(),
            schema_version: T::schema_version().into(),
            lane: lane.into(),
            instrument_id: instrument_id.into(),
            venue_id: venue_id.into(),
            source: source.into(),
            trust_tier,
            event_time,
            observed_time,
            ingested_time,
            available_time,
            sequence,
            correlation_id: None,
            causation_id: None,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::{Price, Size};
    use crate::payloads::trade::{TradePayload, TradeSide};
    use chrono::Utc;
    use std::str::FromStr;

    #[test]
    fn serde_round_trip_trade_envelope() {
        let payload = TradePayload::new(
            Price::from_str("50000.00").unwrap(),
            Size::from_str("0.01").unwrap(),
            TradeSide::Buy,
            "trade-001",
        );
        let now = Utc::now();
        let env = EventEnvelope::new(
            Uuid::new_v4(),
            "market.trades",
            "BTC-USDT",
            "coinbase",
            "kraken_ws",
            TrustTier::CentralizedExchange,
            Some(now),
            now,
            now,
            now,
            1,
            payload,
        );

        let json = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope<TradePayload> = serde_json::from_str(&json).unwrap();
        assert_eq!(env.event_id, back.event_id);
        assert_eq!(env.instrument_id, back.instrument_id);
        assert_eq!(
            env.payload.exchange_trade_id,
            back.payload.exchange_trade_id
        );
    }
}
