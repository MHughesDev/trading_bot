//! The `core` namespace: the protocol tools every profile exposes.
//!
//! `get_authoring_guide` is the escape hatch that makes AGENT-006 P1 hold — the
//! catalogue may be any size, the prompt may not, so the prose a tool used to
//! carry in its description lives in the guide and is fetched a section at a
//! time. The handler is in `lib.rs`; this module holds only the declaration.
//!
//! `estimate_cost` is the other half of planning: the guide says what a tool is
//! for, and this says what calling it would cost. A model that can ask before
//! committing can plan inside a budget rather than discovering the cost at the
//! wall (§2.4b, Set S task S-3.19).

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;
use crate::ApiClient;

/// `estimate_cost` — what a call would cost, before making it.
pub async fn estimate_cost(api: &ApiClient, params: &Value) -> Value {
    let Some(tool) = params
        .get("tool")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return crate::tool_error::missing_field("tool");
    };
    let body = json!({
        "tool": tool,
        "arguments": params.get("arguments").cloned().unwrap_or_else(|| json!({})),
    });
    match api.post("/api/agent/cost/estimate", body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "get_authoring_guide",
            "core",
            Risk::Read,
            "Return the strategy-authoring protocol: definition JSON format, expression grammar, \
             supported features, backtest workflow, and result interpretation. Call it once before \
             authoring anything.",
            json!({ "type": "object", "properties": {} }),
        )
        .internal(),
        ToolSpec::new(
            "estimate_cost",
            "core",
            Risk::Read,
            "What another tool would cost if you called it with these arguments: trials, seconds \
             and rows. It spends nothing, so ask before committing to an expensive call rather \
             than discovering the cost at the wall.",
            json!({
                "type": "object",
                "required": ["tool"],
                "properties": {
                    "tool": {"type": "string", "description": "the tool you are considering"},
                    "arguments": {
                        "type": "object",
                        "description": "the arguments you would pass it"
                    }
                }
            }),
        )
        .internal()
        // Bounded because it is cheap enough to loop on. Twenty is a planning
        // budget, not a browsing one.
        .max_calls_per_run(20)
        .summary(
            "`{tool}` would spend {trials} trial(s) and about {est_seconds} seconds ({confidence})",
            &[
                ("tool", ""),
                ("trials", "trials"),
                ("est_seconds", "s"),
                ("confidence", ""),
            ],
        )
        .notes("estimate_cost"),
    ]
}
