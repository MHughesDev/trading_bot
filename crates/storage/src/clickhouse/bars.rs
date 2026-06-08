//! ClickHouse inserts for the `market_bars` table.

use clickhouse::{Client, Row};
use domain::payloads::bar::BarPayload;
use domain::EventEnvelope;
use serde::Serialize;

use super::ChError;

#[derive(Row, Serialize)]
pub struct BarRow {
    pub instrument_id: String,
    pub venue_id: String,
    /// Microseconds since Unix epoch.
    pub event_time_us: i64,
    pub available_time_us: i64,
    pub timeframe: String,
    /// OHLCV as strings (Decimal-safe).
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    pub volume: String,
    pub trade_count: u64,
    /// `0` for the first publish; incremented on each late-data revision.
    pub revision: u32,
    pub event_id: String,
}

impl BarRow {
    pub fn from_envelope(
        env: &EventEnvelope<BarPayload>,
        instrument_id: &str,
        venue_id: &str,
    ) -> Self {
        let event_time_us = env
            .event_time
            .unwrap_or(env.ingested_time)
            .timestamp_micros();
        let available_time_us = env.available_time.timestamp_micros();
        let timeframe = format!("{:?}", env.payload.timeframe).to_lowercase();

        Self {
            instrument_id: instrument_id.to_owned(),
            venue_id: venue_id.to_owned(),
            event_time_us,
            available_time_us,
            timeframe,
            open: env.payload.open.to_string(),
            high: env.payload.high.to_string(),
            low: env.payload.low.to_string(),
            close: env.payload.close.to_string(),
            volume: env.payload.volume.to_string(),
            trade_count: env.payload.trade_count,
            revision: env.payload.revision,
            event_id: env.event_id.to_string(),
        }
    }
}

pub async fn insert_batch(client: &Client, rows: &[BarRow]) -> Result<(), ChError> {
    let mut insert = client
        .insert("market_bars")
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
