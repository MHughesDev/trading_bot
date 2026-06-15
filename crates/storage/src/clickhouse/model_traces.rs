//! ClickHouse inserts for `model_predictions` and `model_traces`.
//! Batch insert pattern mirrors `features.rs`.

use clickhouse::{Client, Row};
use serde::Serialize;
use uuid::Uuid;

use super::ChError;

#[derive(Row, Serialize)]
pub struct PredictionRow {
    pub model_id: String,
    pub version: u32,
    pub instrument_id: String,
    pub event_time_us: i64,
    pub produced_time_us: i64,
    pub direction: String,
    /// Decimal-safe string (ADR-0002).
    pub magnitude_str: String,
    /// Calibration metric 0..1 — not money.
    pub confidence: f64,
    pub horizon: String,
}

pub async fn insert_predictions(client: &Client, rows: &[PredictionRow]) -> Result<(), ChError> {
    let mut insert = client
        .insert("model_predictions")
        .map_err(|e| ChError::Client(e.to_string()))?;
    for row in rows {
        insert
            .write(row)
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))
}

#[derive(Row, Serialize)]
pub struct TraceRow {
    pub trace_id: Uuid,
    pub model_id: String,
    pub version: u32,
    pub kind: String,
    pub latency_ms: u64,
    /// Decimal-safe string (ADR-0002).
    pub cost_usd_str: String,
    pub input_hash: String,
    pub output_hash: String,
    pub status: String,
    pub ts_us: i64,
}

pub async fn insert_traces(client: &Client, rows: &[TraceRow]) -> Result<(), ChError> {
    let mut insert = client
        .insert("model_traces")
        .map_err(|e| ChError::Client(e.to_string()))?;
    for row in rows {
        insert
            .write(row)
            .await
            .map_err(|e| ChError::Insert(e.to_string()))?;
    }
    insert.end().await.map_err(|e| ChError::Insert(e.to_string()))
}
