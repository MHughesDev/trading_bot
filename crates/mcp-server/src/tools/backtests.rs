//! Backtest tools: `get_backtest`, `wait_for_backtest`,
//! `list_backtests`, `stop_backtest`.
//!
//! All calls go through the platform API (`/api/backtests`), so runs are
//! user-scoped, visible in the UI, and driven by the platform's single
//! `BacktestManager` (ADR-0010, ADR-0014).

use std::time::Duration;

use serde_json::{json, Value};


use crate::{ApiClient, ProgressUpdate};

/// Array length above which summary detail truncates (equity curves, trade
/// lists — the full document is available with `detail: "full"`).
const SUMMARY_MAX_ARRAY: usize = 50;

fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

fn snapshot_status(snapshot: &Value) -> String {
    snapshot
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Recursively truncate long arrays so summary responses stay small.
fn summarize_json(value: &Value) -> Value {
    match value {
        Value::Array(arr) if arr.len() > SUMMARY_MAX_ARRAY => {
            let mut kept: Vec<Value> = arr
                .iter()
                .take(SUMMARY_MAX_ARRAY)
                .map(summarize_json)
                .collect();
            kept.push(json!({
                "truncated": true,
                "omitted": arr.len() - SUMMARY_MAX_ARRAY,
                "hint": "use get_backtest with detail:'full' for the complete array",
            }));
            Value::Array(kept)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(summarize_json).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), summarize_json(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn shape_snapshot(mut snapshot: Value, detail: &str) -> Value {
    // Surface partial coverage prominently: a "completed" run that simulated a
    // fraction of the requested window is a silent trap for agents reading
    // only status + headline metrics.
    let missing = snapshot
        .pointer("/coverage/missing_ranges")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    if missing > 0 {
        let expected = snapshot
            .pointer("/coverage/expected_bars")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let present = snapshot
            .pointer("/coverage/present_bars")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert(
                "coverage_warning".into(),
                json!(format!(
                    "requested window NOT fully covered: {present} of {expected} expected bars \
                     present, {missing} missing range(s) — results reflect only the covered \
                     stretch; see coverage.missing_ranges"
                )),
            );
        }
    }
    // The window this run covered, in the shape every other evidence-producing
    // tool uses (S-4.2, ADR-S4-04).
    //
    // The snapshot has carried `start` and `end` at the top level all along, and
    // that is not the same thing: a finding's `data_window` is derived by looking
    // for `window: {start, end}`, and a bare `start` beside twenty other fields
    // is not something a reader can safely treat as the period under study.
    // Without this a finding citing a backtest — the most important evidence a
    // finding can cite — had no derivable window and could not be stored.
    //
    // These are the **requested** ends. A run whose coverage was partial says so
    // loudly in `coverage_warning` above, which is the right place for it: the
    // window is what the claim is about, and the coverage is how well it was
    // measured.
    let window = match (snapshot.get("start").cloned(), snapshot.get("end").cloned()) {
        (Some(start), Some(end)) => Some(json!({ "start": start, "end": end })),
        _ => None,
    };
    if let (Some(w), Some(obj)) = (window, snapshot.as_object_mut()) {
        obj.insert("window".into(), w);
    }

    if detail == "full" {
        snapshot
    } else {
        summarize_json(&snapshot)
    }
}

// `create_backtest` was removed: dispatching compute without a registered
// trial row is INV-16. The sanctioned path is create_experiment → run_sweep,
// which registers, propensity-logs and counts every member before it runs.

/// `get_backtest` — one-shot snapshot via `GET /api/backtests/{id}`.
pub async fn get_backtest(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("backtest_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "backtest_id" });
    }
    let detail = params
        .get("detail")
        .and_then(|v| v.as_str())
        .unwrap_or("summary");
    match api.get(&format!("/api/backtests/{id}")).await {
        Ok(snapshot) => shape_snapshot(snapshot, detail),
        Err(e) => e.to_tool_error(),
    }
}

/// `wait_for_backtest` — poll server-side until terminal or timeout.
///
/// Sends `ProgressUpdate`s (forwarded to streaming clients as MCP progress
/// notifications) so long waits keep the connection alive.
pub async fn wait_for_backtest(
    api: &ApiClient,
    params: &Value,
    progress: Option<tokio::sync::mpsc::Sender<ProgressUpdate>>,
) -> Value {
    let id = params
        .get("backtest_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "backtest_id" });
    }
    let timeout_secs = params
        .get("timeout_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(300)
        .clamp(10, 600) as u64;
    let poll_secs = params
        .get("poll_seconds")
        .and_then(|v| v.as_i64())
        .unwrap_or(5)
        .clamp(2, 60) as u64;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut last_snapshot = Value::Null;

    loop {
        match api.get(&format!("/api/backtests/{id}")).await {
            Ok(snapshot) => {
                let status = snapshot_status(&snapshot);
                let pct = snapshot
                    .get("progress")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if let Some(tx) = &progress {
                    let _ = tx
                        .send(ProgressUpdate {
                            progress: pct,
                            message: format!("backtest {status} — {pct:.0}%"),
                        })
                        .await;
                }
                if is_terminal(&status) {
                    return shape_snapshot(snapshot, "summary");
                }
                last_snapshot = snapshot;
            }
            Err(e) => {
                // Transient API errors during a wait shouldn't abort the wait;
                // hard 4xx (bad id) should.
                if matches!(e.status, Some(s) if s < 500) {
                    return e.to_tool_error();
                }
            }
        }

        if tokio::time::Instant::now() + Duration::from_secs(poll_secs) > deadline {
            return json!({
                "timed_out": true,
                "backtest_id": id,
                "status": snapshot_status(&last_snapshot),
                "progress": last_snapshot.get("progress"),
                "hint": "still running — call wait_for_backtest again",
            });
        }
        tokio::time::sleep(Duration::from_secs(poll_secs)).await;
    }
}

/// `list_backtests` — recent runs, compact rows.
pub async fn list_backtests(api: &ApiClient, params: &Value) -> Value {
    let limit = params
        .get("limit")
        .and_then(|v| v.as_i64())
        .unwrap_or(20)
        .clamp(1, 100);
    match api.get(&format!("/api/backtests?limit={limit}")).await {
        Ok(resp) => {
            let rows = resp
                .get("backtests")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let compact: Vec<Value> = rows
                .iter()
                .map(|s| {
                    json!({
                        "backtest_id": s.get("id"),
                        "name": s.get("name"),
                        "strategy_id": s.get("strategy_slug"),
                        "instrument_id": s.get("instrument_id"),
                        "timeframe": s.get("timeframe"),
                        "status": s.get("status"),
                        "progress": s.get("progress"),
                        "error": s.get("error"),
                        "created_at": s.get("created_at"),
                        "finished_at": s.get("finished_at"),
                    })
                })
                .collect();
            json!({ "backtests": compact, "total": resp.get("total") })
        }
        Err(e) => e.to_tool_error(),
    }
}

/// `stop_backtest` — cancel a running job.
pub async fn stop_backtest(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("backtest_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "backtest_id" });
    }
    match api
        .post(&format!("/api/backtests/{id}/stop"), json!({}))
        .await
    {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// `rerun_backtest` — fresh run with the same spec; returns the new id.
pub async fn rerun_backtest(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("backtest_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "backtest_id" });
    }
    match api
        .post(&format!("/api/backtests/{id}/rerun"), json!({}))
        .await
    {
        Ok(resp) => json!({
            "backtest_id": resp.get("id"),
            "status": "queued",
            "hint": "call wait_for_backtest on the new id",
        }),
        Err(e) => e.to_tool_error(),
    }
}

/// `delete_backtest` — remove a finished run (terminal states only).
pub async fn delete_backtest(api: &ApiClient, params: &Value) -> Value {
    let id = params
        .get("backtest_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id.is_empty() {
        return json!({ "error": "missing_field", "field": "backtest_id" });
    }
    match api.delete(&format!("/api/backtests/{id}")).await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// Result keys worth surfacing in a side-by-side comparison.
const COMPARE_RESULT_KEYS: &[&str] = &[
    "summary",
    "stats_returns",
    "stats_general",
    "total_orders",
    "total_positions",
];

/// `compare_backtests` — side-by-side headline metrics for 2–5 runs.
pub async fn compare_backtests(api: &ApiClient, params: &Value) -> Value {
    let ids: Vec<String> = params
        .get("backtest_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if ids.len() < 2 || ids.len() > 5 {
        return json!({
            "error": "invalid_request",
            "hint": "pass backtest_ids as an array of 2-5 run UUIDs",
        });
    }

    let mut rows = Vec::with_capacity(ids.len());
    for id in &ids {
        match api.get(&format!("/api/backtests/{id}")).await {
            Ok(snap) => {
                let mut metrics = serde_json::Map::new();
                if let Some(result) = snap.get("result").filter(|r| !r.is_null()) {
                    for key in COMPARE_RESULT_KEYS {
                        if let Some(v) = result.get(*key) {
                            metrics.insert((*key).to_string(), v.clone());
                        }
                    }
                }
                rows.push(json!({
                    "backtest_id": id,
                    "name": snap.get("name"),
                    "strategy_id": snap.get("strategy_slug"),
                    "instrument_id": snap.get("instrument_id"),
                    "timeframe": snap.get("timeframe"),
                    "start": snap.get("start"),
                    "end": snap.get("end"),
                    "status": snap.get("status"),
                    "error": snap.get("error"),
                    "metrics": metrics,
                }));
            }
            Err(e) => rows.push(json!({ "backtest_id": id, "error": e.to_tool_error() })),
        }
    }
    json!({ "comparison": rows })
}

/// Declarations for the seven read/manage `backtest` tools (AGENT-006 §2.3).
///
/// There is deliberately no `create_backtest`. Compute may not be dispatched
/// without a registered trial row (INV-16), and a raw backtest has no Experiment
/// to register against — so the only path that runs anything is
/// `create_experiment` → `run_sweep`, where every member is registered,
/// propensity-logged and counted before it executes. `backtest_strategy` is
/// declared in `research.rs` with the rest of that path.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{ToolSpec, TrialKind};
    use crate::taxonomy::Risk;
    let by_id = |name: &'static str, risk, description, what: &'static str| {
        ToolSpec::new(
            name,
            "backtest",
            risk,
            description,
            json!({
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": what }
                }
            }),
        )
    };
    vec![
        ToolSpec::new(
            "get_backtest",
            "backtest",
            Risk::Read,
            "READ an existing backtest by id — it runs nothing. Status, progress, error detail \
             and, when completed, the result document.",
            json!({
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID of an existing run (from run_sweep)" },
                    "detail": { "type": "string", "enum": ["summary", "full"], "description": "Default summary" }
                }
            }),
        )
        .internal()
        .evidence(&["backtest_id"])
        .notes("get_backtest"),
        ToolSpec::new(
            "wait_for_backtest",
            "backtest",
            Risk::Read,
            "Block server-side until a backtest reaches a terminal state or timeout_seconds \
             elapses. On timeout it returns {timed_out:true, status, progress}; call it again.",
            json!({
                "type": "object",
                "required": ["backtest_id"],
                "properties": {
                    "backtest_id": { "type": "string", "description": "UUID of an existing run (from run_sweep)" },
                    "timeout_seconds": { "type": "integer", "description": "Max seconds this call blocks (default 300, clamp 10-600)" },
                    "poll_seconds": { "type": "integer", "description": "Seconds between status polls (default 5, clamp 2-60)" }
                }
            }),
        )
        .internal()
        .notes("wait_for_backtest"),
        ToolSpec::new(
            "list_backtests",
            "backtest",
            Risk::Read,
            "List recent backtest runs (newest first) with status, progress and identifying \
             fields.",
            json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max runs to return (default 20, max 100)" }
                }
            }),
        )
        .internal(),
        by_id(
            "stop_backtest",
            Risk::Write,
            "Cancel a running backtest.",
            "UUID of the run to cancel",
        ),
        // Re-running dispatches the simulator again. The trial that pays for it
        // is the one the original run registered; see the note in `spec.rs`
        // about declaration versus dispatch-time enforcement (§2.5).
        by_id(
            "rerun_backtest",
            Risk::Write,
            "Start a fresh run with the same spec as an existing backtest; returns the new \
             backtest_id.",
            "UUID of the run to copy",
        )
        .spends_trial(TrialKind::Search)
        .evidence(&["backtest_id"]),
        // The evidence behind a result disappears. If a report cited it, the
        // citation now points at nothing.
        by_id(
            "delete_backtest",
            Risk::Destructive,
            "Delete a finished backtest run (terminal states only). Anything that cited it now \
             cites nothing.",
            "UUID of the run to delete",
        ),
        ToolSpec::new(
            "compare_backtests",
            "backtest",
            Risk::Read,
            "Side-by-side comparison of 2-5 backtest runs: identifying fields plus headline \
             result metrics.",
            json!({
                "type": "object",
                "required": ["backtest_ids"],
                "properties": {
                    "backtest_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "2-5 backtest UUIDs"
                    }
                }
            }),
        )
        .internal(),
    ]
}

#[cfg(test)]
mod window_tests {
    use super::*;

    /// A backtest result says what period it covered, in the shape a finding's
    /// window is derived from (S-4.2, ADR-S4-04).
    ///
    /// The snapshot has carried `start` and `end` at the top level all along,
    /// and that is not the same thing: `memory::window::from_evidence` looks for
    /// `window: {start, end}`, and a bare `start` among twenty other fields is
    /// not something a reader can safely treat as the period under study. Until
    /// this, a finding citing a backtest had no derivable window and could not
    /// be stored — and a backtest is the most important evidence a finding can
    /// cite.
    #[test]
    fn a_shaped_snapshot_carries_the_window_it_covered() {
        let snap = json!({
            "id": "11112222-0000-0000-0000-000000000000",
            "status": "completed",
            "start": "2026-04-01T00:00:00Z",
            "end": "2026-06-30T00:00:00Z",
            "result": { "metrics": { "sharpe": 1.4 } },
        });
        for detail in ["summary", "full"] {
            let out = shape_snapshot(snap.clone(), detail);
            assert_eq!(
                out.pointer("/window/start").and_then(Value::as_str),
                Some("2026-04-01T00:00:00Z"),
                "`{detail}` lost the window"
            );
            assert_eq!(
                out.pointer("/window/end").and_then(Value::as_str),
                Some("2026-06-30T00:00:00Z"),
                "`{detail}` lost the window"
            );
        }
    }

    /// A snapshot with no ends grows no window rather than an empty one.
    ///
    /// A `window` whose fields are null reads as "this ran over no period",
    /// which is a claim. Absent reads as "this does not say", which is true.
    #[test]
    fn a_snapshot_without_ends_grows_no_window() {
        let out = shape_snapshot(json!({ "id": "x", "status": "queued" }), "summary");
        assert!(out.get("window").is_none());
    }

    /// The coverage warning is untouched: the window is what the claim is
    /// about, and the coverage is how well it was measured.
    #[test]
    fn partial_coverage_still_warns_beside_the_window() {
        let out = shape_snapshot(
            json!({
                "start": "2026-04-01T00:00:00Z",
                "end": "2026-06-30T00:00:00Z",
                "coverage": {
                    "missing_ranges": [{"from": "2026-05-01", "to": "2026-05-09"}],
                    "expected_bars": 2160,
                    "present_bars": 1944,
                },
            }),
            "full",
        );
        assert!(out.get("window").is_some(), "the window is still stamped");
        assert!(
            out.get("coverage_warning").is_some(),
            "and the partial-coverage warning still fires — a run that covered less than it \
             asked for must not look complete because it now has a window"
        );
    }
}
