//! ClickHouse inserts for the `instrument_features` table.
//!
//! Features are flattened: one row per (instrument, time, feature_name).

use clickhouse::{Client, Row};
use serde::Serialize;
use std::collections::HashMap;

use super::ChError;

#[derive(Row, Serialize)]
pub struct FeatureRow {
    pub instrument_id: String,
    pub venue_id: String,
    /// Microseconds since Unix epoch.
    pub event_time_us: i64,
    pub available_time_us: i64,
    pub feature_name: String,
    /// Decimal-safe string value.
    pub value: String,
}

impl FeatureRow {
    /// Expand a map of feature_name → value into one `FeatureRow` per entry.
    pub fn from_map(
        instrument_id: &str,
        venue_id: &str,
        event_time_us: i64,
        available_time_us: i64,
        features: &HashMap<String, String>,
    ) -> Vec<Self> {
        features
            .iter()
            .map(|(name, val)| Self {
                instrument_id: instrument_id.to_owned(),
                venue_id: venue_id.to_owned(),
                event_time_us,
                available_time_us,
                feature_name: name.clone(),
                value: val.clone(),
            })
            .collect()
    }
}

pub async fn insert_batch(client: &Client, rows: &[FeatureRow]) -> Result<(), ChError> {
    let mut insert = client
        .insert("instrument_features")
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
