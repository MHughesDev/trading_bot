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
    pub available_time_us: i64,
    /// `Decimal` serialized as a string.
    pub price: String,
    pub size: String,
    pub side: String,
    pub trade_id: String,
    pub event_id: String,
}

impl TradeRow {
    pub fn from_envelope(
        env: &EventEnvelope<TradePayload>,
        instrument_id: &str,
        venue_id: &str,
    ) -> Self {
        let event_time_us = env
            .event_time
            .unwrap_or(env.ingested_time)
            .timestamp_micros();
        let available_time_us = env.available_time.timestamp_micros();
        let side = format!("{:?}", env.payload.side).to_lowercase();

        Self {
            instrument_id: instrument_id.to_owned(),
            venue_id: venue_id.to_owned(),
            event_time_us,
            available_time_us,
            price: env.payload.price.to_string(),
            size: env.payload.size.to_string(),
            side,
            trade_id: env.payload.exchange_trade_id.clone(),
            event_id: env.event_id.to_string(),
        }
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
