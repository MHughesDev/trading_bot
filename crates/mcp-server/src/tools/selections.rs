//! Selection tools: `run_selection`, `list_selection_runs` (Set U U-6.3).
//!
//! A selection strategy answers "which assets should we be in?". These call
//! the same route the web screener calls — `POST /api/selections/run` — so the
//! agent and a person asking about the same strategy at the same instant get
//! the same list. The tool forwards exactly the two things that decide the
//! answer, the strategy and the instant; it adds nothing of its own.

use serde_json::{json, Value};

use crate::ApiClient;

/// The request body for a run: the strategy, and the instant when one is
/// given. Nothing else reaches the route.
fn run_body(params: &Value) -> Result<Value, Value> {
    let strategy = params
        .get("strategy")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| json!({ "error": "missing_field", "field": "strategy" }))?;
    let mut body = json!({ "strategy": strategy });
    if let Some(as_of) = params.get("as_of").and_then(Value::as_str) {
        body["as_of"] = json!(as_of);
    }
    Ok(body)
}

/// `run_selection` — run a selection strategy now or as of a past instant.
pub async fn run_selection(api: &ApiClient, params: &Value) -> Value {
    let body = match run_body(params) {
        Ok(b) => b,
        Err(e) => return e,
    };
    match api.post("/api/selections/run", body).await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// Percent-encode a query value: ids and hashes pass through unchanged.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `list_selection_runs` — the journal, newest first.
pub async fn list_selection_runs(api: &ApiClient, params: &Value) -> Value {
    let mut path = String::from("/api/selections/runs");
    let mut q = Vec::new();
    if let Some(s) = params.get("strategy").and_then(Value::as_str) {
        q.push(format!("strategy={}", encode(s)));
    }
    if let Some(n) = params.get("limit").and_then(Value::as_i64) {
        q.push(format!("limit={}", n.clamp(1, 500)));
    }
    if !q.is_empty() {
        path.push('?');
        path.push_str(&q.join("&"));
    }
    match api.get(&path).await {
        Ok(resp) => resp,
        Err(e) => e.to_tool_error(),
    }
}

/// Declarations for the `selection` tools.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![
        ToolSpec::new(
            "run_selection",
            "strategy",
            Risk::Read,
            "Run a selection strategy (a screener) now, or as of a past instant, and get the \
             ranked list it picks — every universe member selected with its score, or excluded \
             with the reason. Spends no trial; the run is journaled.",
            json!({
                "type": "object",
                "required": ["strategy"],
                "properties": {
                    "strategy": { "type": "string", "description": "A selection strategy's revision hash, or its strategy id (its head revision)" },
                    "as_of": { "type": "string", "description": "RFC 3339 instant to run as of; membership and data are as they stood then. Default: now" }
                }
            }),
        )
        .internal()
        .evidence(&["run_id", "digest"]),
        ToolSpec::new(
            "list_selection_runs",
            "strategy",
            Risk::Read,
            "List journaled selection runs, newest first, optionally of one strategy.",
            json!({
                "type": "object",
                "properties": {
                    "strategy": { "type": "string", "description": "Revision hash or strategy id" },
                    "limit": { "type": "integer", "description": "Default 50, at most 500" }
                }
            }),
        )
        .internal()
        .evidence(&["run_id"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_forwards_only_the_strategy_and_the_instant() {
        let b = run_body(&json!({"strategy": "s1", "as_of": "2025-03-01T00:00:00Z", "top_n": 3}))
            .unwrap();
        assert_eq!(
            b,
            json!({"strategy": "s1", "as_of": "2025-03-01T00:00:00Z"})
        );
        assert_eq!(
            run_body(&json!({"strategy": "s1"})).unwrap(),
            json!({"strategy": "s1"})
        );
        assert!(run_body(&json!({})).is_err());
    }
}
