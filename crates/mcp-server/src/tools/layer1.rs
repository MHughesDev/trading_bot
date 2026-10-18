//! `run_research_strategy` (Set M M-1.8): submit a Layer 1 research-strategy file.
//!
//! A thin client of `POST /api/jobs` with `kind = layer1_run` (ADR-0010). The job runs the
//! function and its Gate-0 truncation re-runs, then dispatches the one counted evaluation
//! (`crates/api/src/layer1_worker.rs`). The tool schema has no field a P&L could ride in
//! (`additionalProperties: false`), and the manifest type refuses one again server-side.
//!
//! Pre-flight is a better error message, not a boundary: the file must declare its parameters
//! (the declared params are the only free parameters, and a strategy with none cannot be swept
//! by the platform sampler), and the harness refuses it again as `meta.no_params`.

use serde_json::{json, Value};

use crate::ApiClient;

fn s(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The params array `[{name, value}]` as the manifest object.
fn params_object(params: &Value) -> Result<Value, Value> {
    let mut out = serde_json::Map::new();
    for item in params
        .get("params")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(value)) = (
            item.get("name").and_then(Value::as_str),
            item.get("value").filter(|v| v.is_number()),
        ) else {
            return Err(crate::tool_error::refuse(
                "bad_param",
                "each entry of `params` needs a string `name` and a numeric `value`.",
                "pass params as [{\"name\": \"lookback\", \"value\": 20}]",
            ));
        };
        out.insert(name.to_string(), value.clone());
    }
    Ok(Value::Object(out))
}

/// `run_research_strategy`.
pub async fn run_research_strategy(api: &ApiClient, params: &Value) -> Value {
    for f in [
        "experiment_id",
        "source",
        "instrument",
        "timeframe",
        "start",
        "end",
    ] {
        if s(params, f).is_none() {
            return crate::tool_error::missing_field(f);
        }
    }
    let source = s(params, "source").unwrap_or_default();
    if !source.contains("research_strategy") || !source.contains("Param.") {
        return crate::tool_error::refuse(
            "no_params_declared",
            "the file declares no parameters: it needs @research_strategy(..., params={\"name\": Param.int(lo, hi)}).",
            "declare every free parameter with Param.int or Param.float; they are the only free parameters and the platform sampler sweeps them",
        );
    }
    let (Some(fee), Some(half)) = (
        params.get("fee_bps").and_then(Value::as_f64),
        params.get("half_spread_bps").and_then(Value::as_f64),
    ) else {
        return crate::tool_error::refuse(
            "costs_required",
            "fee_bps and half_spread_bps are required: a run with no stated costs is not run.",
            "state the venue taker fee and an estimated half-spread, in basis points",
        );
    };
    let manifest_params = match params_object(params) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut manifest = json!({
        "experiment_id": s(params, "experiment_id"),
        "source": source,
        "instrument": s(params, "instrument"),
        "timeframe": s(params, "timeframe"),
        "start": s(params, "start"),
        "end": s(params, "end"),
        // The read horizon is the platform clock at submission, never the caller's.
        "as_of": chrono::Utc::now().to_rfc3339(),
        "params": manifest_params,
        "seed": params.get("seed").and_then(Value::as_u64).unwrap_or(0),
        "fee_bps": fee,
        "half_spread_bps": half,
    });
    if let Some(n) = params.get("sweep_runs").and_then(Value::as_u64) {
        // A count, never the points: the platform sampler draws them and the result is sealed.
        manifest["sweep"] = json!({ "max_runs": n });
    }
    let body = json!({
        "kind": "layer1_run",
        "manifest": manifest,
        "experiment_id": s(params, "experiment_id"),
    });
    match api.post("/api/jobs", body).await {
        Ok(v) => json!({
            "summary_fields": { "job_id": v["job_id"], "state": v["state"] },
            "detail": {
                "job": v,
                "next": "poll the job: its result carries the run id, the trial id, the truncation verdict and any Gate-0 flag",
            },
            "ids": { "job_id": v["job_id"] },
        }),
        Err(e) => e.to_tool_error(),
    }
}

#[must_use]
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{ToolSpec, TrialKind};
    use crate::taxonomy::Risk;
    vec![ToolSpec::new(
        "run_research_strategy",
        "backtest",
        Risk::Write,
        "RUN A LAYER 1 RESEARCH STRATEGY: a Python function returning target positions, run under the platform costs and fills. Re-run on truncated data first (Gate 0) and rejected if its earlier answers change; then one counted trial. Declare every parameter. sweep_runs=N samples N points (sealed). Never supply P&L.",
        json!({
            "type": "object",
            "required": ["experiment_id", "source", "instrument", "timeframe", "start", "end", "fee_bps", "half_spread_bps"],
            "additionalProperties": false,
            "properties": {
                "experiment_id": { "type": "string" },
                "source": { "type": "string", "description": "Python defining positions(data, p) under @research_strategy" },
                "instrument": { "type": "string" },
                "timeframe": { "type": "string" },
                "start": { "type": "string" },
                "end": { "type": "string" },
                "params": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["name", "value"],
                        "additionalProperties": false,
                        "properties": { "name": { "type": "string" }, "value": { "type": "number" } }
                    }
                },
                "seed": { "type": "integer" },
                "sweep_runs": { "type": "integer" },
                "fee_bps": { "type": "number" },
                "half_spread_bps": { "type": "number" }
            }
        }),
    )
    .internal()
    .spends_trial(TrialKind::Search)
    .evidence(&["job_id"])
    .max_calls_per_run(20)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_has_no_place_to_put_a_pnl_and_requires_stated_costs() {
        let s = &specs()[0];
        assert_eq!(s.input_schema["additionalProperties"], json!(false));
        let props = s.input_schema["properties"].as_object().unwrap();
        for forbidden in ["pnl", "equity", "returns", "equity_curve"] {
            assert!(!props.contains_key(forbidden));
        }
        let req = s.input_schema["required"].as_array().unwrap();
        assert!(req.contains(&json!("fee_bps")) && req.contains(&json!("half_spread_bps")));
    }

    #[test]
    fn params_become_the_manifest_object_and_a_malformed_one_is_refused() {
        let ok = params_object(
            &json!({"params": [{"name": "lb", "value": 5}, {"name": "k", "value": 1.5}]}),
        )
        .unwrap();
        assert_eq!(ok, json!({"lb": 5, "k": 1.5}));
        assert!(params_object(&json!({"params": [{"name": "lb", "value": "five"}]})).is_err());
        assert_eq!(params_object(&json!({})).unwrap(), json!({}));
    }

    #[test]
    fn a_description_is_inside_the_descriptor_budget() {
        let s = &specs()[0];
        let bytes = serde_json::to_string(
            &json!({"name": s.name, "description": s.description, "inputSchema": s.input_schema}),
        )
        .unwrap()
        .len();
        assert!(bytes <= 1_200, "{bytes}");
    }
}
