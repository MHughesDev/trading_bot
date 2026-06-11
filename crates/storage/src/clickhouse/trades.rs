//! ClickHouse inserts for the `market_trades` table.

use clickhouse::{Client, Row};
use domain::payloads::trade::TradePayload;
use domain::EventEnvelope;
use serde::Serialize;

use super::ChError;

#[derive(Row, Serialize)]
pub struct TradeRow {
    pub instrument_id: String,
    pub venue_id: String,
    /// Microseconds since Unix epoch.
    pub event_time_us: i64,
    /// `Decimal` serialized as a string.
    pub price: String,
    pub size: String,
    pub side: String,
    pub trade_id: String,
}

impl TradeRow {
    pub fn from_envelope(env: &EventEnvelope) -> Option<Self> {
        let trade = env.decode_payload::<TradePayload>().ok()?;
        let instrument_id = domain::instrument_name(env.instrument_id).unwrap_or_default();
        let venue_id = domain::venue_name(env.venue_id).unwrap_or_default();
        let event_time_us = env.timestamp_ns / 1_000;
        let side = format!("{:?}", trade.side).to_lowercase();

        Some(Self {
            instrument_id,
            venue_id,
            event_time_us,
            price: trade.price.to_string(),
            size: trade.size.to_string(),
            side,
            trade_id: trade.exchange_trade_id.clone(),
        })
    }
}

pub async fn insert_batch(client: &Client, rows: &[TradeRow]) -> Result<(), ChError> {
    let mut insert = client
        .insert("market_trades")
        .map_err(|e| ChError::Client(e.to_string()))?;
    for row in rows {
        insert
            .write(row)
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert
        .end()
        .await
        .map_err(|e| ChError::Insert(e.to_string()))
}
