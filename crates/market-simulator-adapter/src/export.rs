//! Convert `BarPayload` slices to Arrow IPC bytes matching market_simulator's
//! OHLCV data contract.
//!
//! Only 1-minute OHLCV bars are supported for MVP; the adapter rejects other
//! granularities (market_simulator Engine A requires 1m data).

use std::sync::Arc;

use arrow::array::{Float64Array, Int64Array};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, Utc};
use domain::payloads::bar::{BarPayload, Timeframe};
use rust_decimal::prelude::ToPrimitive;
use thiserror::Error;

use crate::contract::OHLCV_SCHEMA_VERSION;

/// Errors produced during export.
#[derive(Debug, Error)]
pub enum ExportError {
    #[error("no bars provided — at least one bar is required")]
    NoBars,

    #[error("unsupported timeframe {0:?} — only Minutes1 is supported for MVP")]
    UnsupportedTimeframe(Timeframe),

    #[error("Arrow error: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

/// A bar record paired with its `available_time`.
pub struct TimedBar {
    pub available_time: DateTime<Utc>,
    pub bar: BarPayload,
}

/// Serialise `bars` to Arrow IPC stream bytes.
///
/// Schema: `timestamp_ns i64, open f64, high f64, low f64, close f64, volume f64`.
/// Only `Timeframe::Minutes1` bars are accepted for MVP.
pub fn bars_to_ipc_bytes(bars: &[TimedBar]) -> Result<Vec<u8>, ExportError> {
    if bars.is_empty() {
        return Err(ExportError::NoBars);
    }

    for tb in bars {
        if tb.bar.timeframe != Timeframe::Minutes1 {
            return Err(ExportError::UnsupportedTimeframe(tb.bar.timeframe));
        }
    }

    let schema = ohlcv_schema();

    let timestamps: Int64Array = bars
        .iter()
        .map(|tb| tb.available_time.timestamp_nanos_opt().unwrap_or(0))
        .collect::<Vec<i64>>()
        .into();

    let opens: Float64Array = bars
        .iter()
        .map(|tb| tb.bar.open.inner().to_f64().unwrap_or(0.0))
        .collect::<Vec<f64>>()
        .into();

    let highs: Float64Array = bars
        .iter()
        .map(|tb| tb.bar.high.inner().to_f64().unwrap_or(0.0))
        .collect::<Vec<f64>>()
        .into();

    let lows: Float64Array = bars
        .iter()
        .map(|tb| tb.bar.low.inner().to_f64().unwrap_or(0.0))
        .collect::<Vec<f64>>()
        .into();

    let closes: Float64Array = bars
        .iter()
        .map(|tb| tb.bar.close.inner().to_f64().unwrap_or(0.0))
        .collect::<Vec<f64>>()
        .into();

    let volumes: Float64Array = bars
        .iter()
        .map(|tb| tb.bar.volume.inner().to_f64().unwrap_or(0.0))
        .collect::<Vec<f64>>()
        .into();

    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(timestamps),
            Arc::new(opens),
            Arc::new(highs),
            Arc::new(lows),
            Arc::new(closes),
            Arc::new(volumes),
        ],
    )?;

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = StreamWriter::try_new(&mut buf, &schema)?;
        writer.write(&batch)?;
        writer.finish()?;
    }

    Ok(buf.into_inner())
}

/// Returns the canonical OHLCV Arrow schema for market_simulator Engine A.
pub fn ohlcv_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("timestamp_ns", DataType::Int64, false),
        Field::new("open", DataType::Float64, false),
        Field::new("high", DataType::Float64, false),
        Field::new("low", DataType::Float64, false),
        Field::new("close", DataType::Float64, false),
        Field::new("volume", DataType::Float64, false),
    ]))
}

/// Schema version string embedded in all exports.
pub fn schema_version() -> &'static str {
    OHLCV_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::money::{Price, Size};
    use std::str::FromStr;

    fn make_bar(close: &str) -> BarPayload {
        BarPayload::new(
            Timeframe::Minutes1,
            Price::from_str("100").unwrap(),
            Price::from_str("110").unwrap(),
            Price::from_str("95").unwrap(),
            Price::from_str(close).unwrap(),
            Size::from_str("500").unwrap(),
            200,
        )
    }

    fn timed(bar: BarPayload) -> TimedBar {
        TimedBar {
            available_time: Utc::now(),
            bar,
        }
    }

    #[test]
    fn empty_slice_returns_error() {
        assert!(matches!(bars_to_ipc_bytes(&[]), Err(ExportError::NoBars)));
    }

    #[test]
    fn single_bar_produces_non_empty_bytes() {
        let bytes = bars_to_ipc_bytes(&[timed(make_bar("105"))]).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn schema_has_six_columns() {
        let schema = ohlcv_schema();
        assert_eq!(schema.fields().len(), 6);
    }
}
