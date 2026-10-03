//! Authoring tools for `tbot.strategy/3` (Set U U-7.4, U-7.6; ADR-0045
//! decision 4; ADR-0022).
//!
//! Every tool here is a thin client of one route in `/api/v3/…` — the routes
//! the strategy editor calls. The module computes nothing: it does not import
//! the language, the compiler or the engine, and a test reads this file to keep
//! it that way. So a person and the agent get the same answer to the same
//! question, because there is only one place that answers it.

use serde_json::{json, Value};

use crate::ApiClient;

/// The strategy fields a request may carry, forwarded as given.
const STRATEGY_FIELDS: [&str; 6] = [
    "text",
    "graph",
    "strategy_ref",
    "ops",
    "params",
    "window_bars",
];

fn strategy_body(params: &Value) -> Value {
    let mut body = serde_json::Map::new();
    for f in STRATEGY_FIELDS {
        if let Some(v) = params.get(f) {
            body.insert(f.into(), v.clone());
        }
    }
    Value::Object(body)
}

async fn post(api: &ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `validate_strategy_v3` → `POST /api/v3/authoring/validate`
pub async fn validate(api: &ApiClient, params: &Value) -> Value {
    post(api, "/api/v3/authoring/validate", strategy_body(params)).await
}

/// `compile_strategy_v3` → `POST /api/v3/authoring/compile`
pub async fn compile(api: &ApiClient, params: &Value) -> Value {
    post(api, "/api/v3/authoring/compile", strategy_body(params)).await
}

/// `explain_strategy_v3` → `POST /api/v3/authoring/explain`
pub async fn explain(api: &ApiClient, params: &Value) -> Value {
    post(api, "/api/v3/authoring/explain", strategy_body(params)).await
}

/// `build_strategy_v3` → `POST /api/v3/authoring/build`
pub async fn build(api: &ApiClient, params: &Value) -> Value {
    post(api, "/api/v3/authoring/build", strategy_body(params)).await
}

/// `save_strategy_v3` → `POST /api/v3/strategies`
pub async fn save(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = params.get("strategy_id").and_then(Value::as_str) else {
        return json!({"error": "missing_field", "field": "strategy_id"});
    };
    let mut body = strategy_body(params);
    body["strategy_id"] = json!(id);
    if let Some(p) = params.get("parent_strategy_id") {
        body["parent_strategy_id"] = p.clone();
    }
    post(api, "/api/v3/strategies", body).await
}

/// `get_strategy_v3` → `GET /api/v3/strategies/{id}`
pub async fn get(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = params
        .get("strategy_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return json!({"error": "missing_field", "field": "strategy_id"});
    };
    let encoded: String = id
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    match api.get(&format!("/api/v3/strategies/{encoded}")).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_strategy_catalogue` → `GET /api/v3/catalogue`
pub async fn catalogue(api: &ApiClient, _params: &Value) -> Value {
    match api.get("/api/v3/catalogue").await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

fn strategy_schema(extra: &[(&str, Value)], required: &[&str]) -> Value {
    let mut props = serde_json::Map::new();
    props.insert(
        "text".into(),
        json!({"type": "string", "description": "SLv3 text"}),
    );
    props.insert("graph".into(), json!({"type": "object"}));
    props.insert(
        "strategy_ref".into(),
        json!({"type": "string", "description": "A saved strategy's id"}),
    );
    props.insert(
        "ops".into(),
        json!({"type": "array", "description": "Builder steps"}),
    );
    props.insert("params".into(), json!({"type": "object"}));
    props.insert("window_bars".into(), json!({"type": "integer"}));
    for (k, v) in extra {
        props.insert((*k).into(), v.clone());
    }
    json!({"type": "object", "required": required, "properties": props})
}

/// Declarations for the v3 authoring tools.
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::ToolSpec;
    use crate::taxonomy::Risk;
    vec![
        ToolSpec::new(
            "validate_strategy_v3",
            "builder",
            Risk::Read,
            "Check a v3 strategy — SLv3 text, graph JSON or builder ops — and get its graph, \
             canonical text and revision, or every finding with its fix.",
            strategy_schema(&[], &[]),
        )
        .internal()
        .evidence(&["revision"])
        .notes("validate_strategy_v3"),
        ToolSpec::new(
            "compile_strategy_v3",
            "strategy",
            Risk::Read,
            "What a valid v3 strategy compiles to: its inputs and indicators, warm-up and the \
             model versions it pins.",
            strategy_schema(&[], &[]),
        )
        .internal()
        .evidence(&["revision"]),
        ToolSpec::new(
            "explain_strategy_v3",
            "builder",
            Risk::Read,
            "A v3 strategy in plain words, one line per node and rule, generated from the graph.",
            strategy_schema(&[], &[]),
        )
        .internal(),
        ToolSpec::new(
            "build_strategy_v3",
            "builder",
            Risk::Read,
            "Build a v3 strategy step by step with builder ops (new, add_node, connect, set_arg, \
             add_rule, …); returns the graph so far and what is still wrong with it.",
            strategy_schema(&[], &["ops"]),
        )
        .internal()
        .notes("build_strategy_v3"),
        ToolSpec::new(
            "save_strategy_v3",
            "builder",
            Risk::Write,
            "Validate and save a v3 strategy as a new revision of `strategy_id` — the same call \
             the strategy editor makes.",
            strategy_schema(
                &[
                    ("strategy_id", json!({"type": "string"})),
                    ("parent_strategy_id", json!({"type": "string"})),
                ],
                &["strategy_id"],
            ),
        )
        .internal()
        .evidence(&["revision_hash"]),
        ToolSpec::new(
            "get_strategy_v3",
            "builder",
            Risk::Read,
            "A saved v3 strategy's head revision, as graph and SLv3 text.",
            json!({"type": "object", "required": ["strategy_id"],
                   "properties": {"strategy_id": {"type": "string"}}}),
        )
        .internal()
        .evidence(&["revision"]),
        ToolSpec::new(
            "get_strategy_catalogue",
            "strategy",
            Risk::Read,
            "Every operator, feature family and callable model a v3 strategy can use, read from \
             the registries the compiler checks against.",
            json!({"type": "object", "properties": {}}),
        )
        .internal(),
    ]
}

#[cfg(test)]
mod tests {
    /// U-7.4's acceptance, by source scan: this module is a thin client. It
    /// reaches the platform only through `api.get` / `api.post`, and imports
    /// nothing that could answer a strategy question itself.
    #[test]
    fn every_v3_strategy_tool_is_a_thin_client_of_an_api_route() {
        let src = include_str!("strategy_v3.rs");
        let body = src.split("#[cfg(test)]").next().unwrap_or(src);
        for forbidden in [
            "strategy_lang",
            "strategy_engine",
            "backtest::",
            "domain::",
            "sqlx",
            "features::",
        ] {
            assert!(
                !body.contains(forbidden),
                "the v3 tools must not use `{forbidden}`"
            );
        }
        for route in [
            "/api/v3/authoring/validate",
            "/api/v3/authoring/compile",
            "/api/v3/authoring/explain",
            "/api/v3/authoring/build",
            "/api/v3/strategies",
            "/api/v3/catalogue",
        ] {
            assert!(body.contains(route), "{route}");
        }
        // And every route it calls is mounted.
        let mounted = include_str!("../../../api/src/routes/mod.rs");
        for route in [
            "/api/v3/authoring/validate",
            "/api/v3/authoring/build",
            "/api/v3/strategies/{id}",
            "/api/v3/catalogue",
        ] {
            assert!(
                mounted.contains(&format!("\"{route}\"")),
                "{route} is not mounted"
            );
        }
    }
}
