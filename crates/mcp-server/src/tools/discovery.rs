//! Discovery tools: `list_lanes` and `list_instruments`.
//!
//! `list_lanes` returns canonical lane names from domain constants (pure).
//! `list_instruments` calls the platform API's bar-coverage endpoint and
//! groups rows per instrument so an agent can pick backtest windows.

use domain::lanes::ALL_LANES;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ApiClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct LaneInfo {
    pub lane: String,
}

/// `list_lanes` — return canonical data lanes from the shared domain constant.
pub fn list_lanes() -> Vec<LaneInfo> {
    ALL_LANES
        .iter()
        .map(|&lane| LaneInfo {
            lane: lane.to_owned(),
        })
        .collect()
}

fn ms_to_rfc3339(ms: i64) -> Value {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|dt| json!(dt.to_rfc3339()))
        .unwrap_or(Value::Null)
}

/// `list_instruments` — instruments with stored bar history, grouped with
/// per-timeframe coverage (bar count, first/last timestamp).
pub async fn list_instruments(api: &ApiClient) -> Value {
    let resp = match api.get("/api/market/instruments").await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };

    // Rows arrive flat (one per instrument×timeframe); group per instrument.
    let rows = resp
        .get("instruments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut grouped: Vec<(String, Vec<Value>)> = Vec::new();
    for row in rows {
        let instrument = row
            .get("instrument_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let entry = json!({
            "timeframe": row.get("timeframe"),
            "bars": row.get("bars"),
            "first": row.get("first_ms").and_then(|v| v.as_i64()).map(ms_to_rfc3339),
            "last": row.get("last_ms").and_then(|v| v.as_i64()).map(ms_to_rfc3339),
        });
        match grouped.iter_mut().find(|(id, _)| *id == instrument) {
            Some((_, list)) => list.push(entry),
            None => grouped.push((instrument, vec![entry])),
        }
    }

    let instruments: Vec<Value> = grouped
        .into_iter()
        .map(|(instrument_id, coverage)| {
            json!({ "instrument_id": instrument_id, "coverage": coverage })
        })
        .collect();

    json!({ "instruments": instruments })
}
