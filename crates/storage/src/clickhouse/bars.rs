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
}

impl BarRow {
    pub fn from_envelope(env: &EventEnvelope) -> Option<Self> {
        let bar = env.decode_payload::<BarPayload>().ok()?;
        let instrument_id = domain::instrument_name(env.instrument_id).unwrap_or_default();
        let venue_id = domain::venue_name(env.venue_id).unwrap_or_default();
        let event_time_us = env.timestamp_ns / 1_000;
        let timeframe = format!("{:?}", bar.timeframe).to_lowercase();

        Some(Self {
            instrument_id,
            venue_id,
            event_time_us,
            timeframe,
            open: bar.open.to_string(),
            high: bar.high.to_string(),
            low: bar.low.to_string(),
            close: bar.close.to_string(),
            volume: bar.volume.to_string(),
            trade_count: bar.trade_count,
            revision: bar.revision,
        })
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
