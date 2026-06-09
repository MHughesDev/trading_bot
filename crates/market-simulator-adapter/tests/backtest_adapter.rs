//! Proves Arrow IPC export matches market_simulator's data contract schema and
//! that the round-trip (export → contract validation) succeeds.

use std::str::FromStr;

use arrow::ipc::reader::StreamReader;
use chrono::Utc;
use domain::money::{Price, Size};
use domain::payloads::bar::{BarPayload, Timeframe};
use market_simulator_adapter::{
    bars_to_ipc_bytes, ohlcv_schema, ExportError, TimedBar, OHLCV_COLUMNS,
};

fn make_bar(close_str: &str) -> BarPayload {
    BarPayload::new(
        Timeframe::Minutes1,
        Price::from_str("100").unwrap(),
        Price::from_str("110").unwrap(),
        Price::from_str("95").unwrap(),
        Price::from_str(close_str).unwrap(),
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

fn read_ipc(bytes: &[u8]) -> Vec<arrow::record_batch::RecordBatch> {
    let cursor = std::io::Cursor::new(bytes);
    let reader = StreamReader::try_new(cursor, None).expect("valid IPC stream");
    reader.map(|r| r.expect("valid batch")).collect()
}

// ── Schema conformance ────────────────────────────────────────────────────────

#[test]
fn exported_schema_has_correct_column_names() {
    let schema = ohlcv_schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    for expected in OHLCV_COLUMNS {
        assert!(
            names.contains(expected),
            "schema missing column '{expected}'"
        );
    }
}

#[test]
fn exported_schema_has_six_columns() {
    assert_eq!(ohlcv_schema().fields().len(), 6);
}

// ── Export round-trip ─────────────────────────────────────────────────────────

#[test]
fn single_bar_round_trips() {
    let bars = vec![timed(make_bar("105"))];
    let bytes = bars_to_ipc_bytes(&bars).expect("export should succeed");
    let batches = read_ipc(&bytes);
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
    assert_eq!(batches[0].num_columns(), 6);
}

#[test]
fn multiple_bars_all_present() {
    let bars: Vec<TimedBar> = (0..10)
        .map(|i| timed(make_bar(&format!("{}", 100 + i))))
        .collect();
    let bytes = bars_to_ipc_bytes(&bars).expect("export should succeed");
    let batches = read_ipc(&bytes);
    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 10);
}

#[test]
fn exported_schema_matches_expected() {
    let bars = vec![timed(make_bar("105"))];
    let bytes = bars_to_ipc_bytes(&bars).unwrap();
    let batches = read_ipc(&bytes);
    let schema = batches[0].schema();

    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(
        names, OHLCV_COLUMNS,
        "exported column order must match contract"
    );
}

// ── Rejection cases ───────────────────────────────────────────────────────────

#[test]
fn adapter_rejects_empty_slice() {
    assert!(matches!(bars_to_ipc_bytes(&[]), Err(ExportError::NoBars)));
}

#[test]
fn adapter_rejects_non_1m_timeframe() {
    let bar = BarPayload::new(
        Timeframe::Hours1,
        Price::from_str("100").unwrap(),
        Price::from_str("110").unwrap(),
        Price::from_str("95").unwrap(),
        Price::from_str("105").unwrap(),
        Size::from_str("500").unwrap(),
        200,
    );
    let result = bars_to_ipc_bytes(&[timed(bar)]);
    assert!(
        matches!(result, Err(ExportError::UnsupportedTimeframe(_))),
        "non-1m timeframe must be rejected"
    );
}
