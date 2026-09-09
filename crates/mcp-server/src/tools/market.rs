//! Market-data tools: `get_bars`, `get_instrument`, `list_asset_classes`.
//!
//! `get_bars` is the agent's eyes on the data: real OHLCV history for any
//! instrument/interval, size-capped so a months-long window can't blow out the
//! context. Coarser intervals are aggregated server-side from stored 1m bars.

use serde_json::{json, Value};

use crate::ApiClient;

/// Hard cap on bars returned to the model in one call.
const MAX_BARS_CAP: usize = 2000;
const DEFAULT_MAX_BARS: usize = 300;

fn f64_of(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_str())
        .and_then(|s| s.parse::<f64>().ok())
}

/// `get_bars` — fetch OHLCV bars with summary stats and tail-truncation.
pub async fn get_bars(api: &ApiClient, params: &Value) -> Value {
    let instrument = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if instrument.is_empty() {
        return json!({ "error": "missing_field", "field": "instrument_id" });
    }
    let start = params.get("start").and_then(|v| v.as_str()).unwrap_or("");
    let end = params.get("end").and_then(|v| v.as_str()).unwrap_or("");
    if start.is_empty() || end.is_empty() {
        return json!({ "error": "missing_field", "field": "start/end (RFC3339)" });
    }
    let interval = params
        .get("interval_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600)
        .clamp(60, 86_400);
    let max_bars = params
        .get("max_bars")
        .and_then(|v| v.as_i64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_MAX_BARS)
        .clamp(10, MAX_BARS_CAP);

    let path = format!(
        "/assets/chart/bars?symbol={}&start={}&end={}&interval_seconds={}",
        urlencode(instrument),
        urlencode(start),
        urlencode(end),
        interval
    );
    let resp = match api.get(&path).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let bars = resp
        .get("bars")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Summary stats over the FULL window, before truncation.
    let closes: Vec<f64> = bars.iter().filter_map(|b| f64_of(b.get("c"))).collect();
    let highs: Vec<f64> = bars.iter().filter_map(|b| f64_of(b.get("h"))).collect();
    let lows: Vec<f64> = bars.iter().filter_map(|b| f64_of(b.get("l"))).collect();
    let summary = if closes.is_empty() {
        json!({ "bar_count": 0 })
    } else {
        let first = closes.first().copied().unwrap_or(0.0);
        let last = closes.last().copied().unwrap_or(0.0);
        json!({
            "bar_count": bars.len(),
            "first_close": first,
            "last_close": last,
            "return_pct": if first != 0.0 { (last - first) / first * 100.0 } else { 0.0 },
            "high": highs.iter().cloned().fold(f64::MIN, f64::max),
            "low": lows.iter().cloned().fold(f64::MAX, f64::min),
        })
    };

    let omitted = bars.len().saturating_sub(max_bars);
    let tail: Vec<Value> = bars.iter().skip(omitted).cloned().collect();

    json!({
        "instrument_id": instrument,
        "interval_seconds": interval,
        "summary": summary,
        "bars": tail,
        "omitted_earlier_bars": omitted,
        "note": if omitted > 0 {
            "bars is the most recent tail; summary covers the full window — narrow the range or raise interval_seconds for detail"
        } else { "" },
    })
}

/// `get_instrument` — one instrument's registry row.
pub async fn get_instrument(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("instrument_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "instrument_id" });
    }
    match api
        .get(&format!("/api/instruments/{}", urlencode(id)))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `list_asset_classes` — asset classes the platform knows.
pub async fn list_asset_classes(api: &ApiClient) -> Value {
    match api.get("/api/assets").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// Minimal percent-encoding for query/path values (alnum plus -_.~ kept).
pub(crate) fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
