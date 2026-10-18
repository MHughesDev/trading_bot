//! The knowledge model's agent-facing tools (Set R-b RB-1.2, RB-1.3, RB-1.7, RB-3.2,
//! RB-4.3; AGENT-003 §3 to §7).
//!
//! Thin clients of `/api/knowledge/*`. Every one is a *question*: is this move vetoed,
//! what are this window's analogs, what regime is this instrument in, what do families do
//! in each regime, how does my campaign compare with random search, and where did this
//! hypothesis come from. None of them records a measurement. The routes that record what
//! a move did, what a study measured or what a trial scored need `knowledge.write`, which
//! no agent session holds, and `no_knowledge_tool_reaches_a_platform_write_route` pins
//! that over this file's source.

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;
use crate::tools::market::urlencode;
use crate::ApiClient;

fn missing(field: &str) -> Value {
    json!({ "error": "missing_field", "field": field })
}

fn s<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty())
}

/// `check_move_history` — has this shape of move a record of failure?
pub async fn check_move_history(api: &ApiClient, params: &Value) -> Value {
    let Some(candidate) = params.get("candidate").filter(|c| c.is_object()) else {
        return missing("candidate");
    };
    match api.post("/api/knowledge/moves/veto", json!({ "candidate": candidate })).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `find_analog_windows` — the nearest past windows by explicit fingerprint.
pub async fn find_analog_windows(api: &ApiClient, params: &Value) -> Value {
    let (Some(iid), Some(vid)) = (
        params.get("instrument_id").and_then(Value::as_i64),
        params.get("venue_id").and_then(Value::as_i64),
    ) else {
        return json!({ "error": "missing_field", "fields": ["instrument_id", "venue_id"] });
    };
    let mut url = format!("/api/knowledge/analogs?instrument_id={iid}&venue_id={vid}");
    if let Some(k) = params.get("k").and_then(Value::as_u64) {
        url.push_str(&format!("&k={k}"));
    }
    if let Some(a) = s(params, "as_of") {
        url.push_str(&format!("&as_of={}", urlencode(a)));
    }
    match api.get(&url).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `tag_hypothesis_source` — say where a hypothesis came from.
pub async fn tag_hypothesis_source(api: &ApiClient, params: &Value) -> Value {
    for f in ["hypothesis_id", "source", "instrument", "window_start", "window_end"] {
        if params.get(f).is_none() {
            return missing(f);
        }
    }
    let body = json!({
        "hypothesis_id": params["hypothesis_id"],
        "source": params["source"],
        "instrument": params["instrument"],
        "window_start": params["window_start"],
        "window_end": params["window_end"],
        "evidence_ids": params.get("evidence_ids").cloned().unwrap_or_else(|| json!([])),
    });
    match api.post("/api/knowledge/hypothesis-sources", body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_regime_profile` — occupancy, transitions and the current label.
pub async fn get_regime_profile(api: &ApiClient, params: &Value) -> Value {
    let (Some(inst), Some(class)) = (s(params, "instrument"), s(params, "asset_class")) else {
        return json!({ "error": "missing_field", "fields": ["instrument", "asset_class"] });
    };
    match api
        .get(&format!(
            "/api/knowledge/regimes/profile?instrument={}&asset_class={}",
            urlencode(inst),
            urlencode(class)
        ))
        .await
    {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `get_regime_prior` — the families that did best in each regime.
pub async fn get_regime_prior(api: &ApiClient, params: &Value) -> Value {
    let (Some(inst), Some(class)) = (s(params, "instrument"), s(params, "asset_class")) else {
        return json!({ "error": "missing_field", "fields": ["instrument", "asset_class"] });
    };
    let mut url = format!(
        "/api/knowledge/regimes/prior?instrument={}&asset_class={}",
        urlencode(inst),
        urlencode(class)
    );
    if let Some(r) = s(params, "regime") {
        url.push_str(&format!("&regime={}", urlencode(r)));
    }
    match api.get(&url).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `compare_to_random` — a campaign against its matched random-search arm.
pub async fn compare_to_random(api: &ApiClient, params: &Value) -> Value {
    let Some(id) = s(params, "campaign_id") else {
        return missing("campaign_id");
    };
    match api.get(&format!("/api/knowledge/campaigns/{}/compare-to-random", urlencode(id))).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec::new(
            "check_move_history",
            "memory",
            Risk::Read,
            "Ask whether a structural move you are about to try has a record of failure. It warns below a \
             stated confidence, blocks above it, and cites the campaigns that failed. Run it before spending a trial on a move.",
            json!({
                "type": "object",
                "required": ["candidate"],
                "additionalProperties": false,
                "properties": {
                    "candidate": {
                        "type": "object",
                        "description": "the move exactly as you would submit it, e.g. {\"move\": \"change_exit\", \"mode\": \"trail\"}",
                        "properties": { "move": { "type": "string", "enum": [
                            "add_condition_filter", "remove_node", "swap_feature", "change_exit",
                            "add_regime_gate", "change_sizing", "simplify", "combine"
                        ] } },
                        "required": ["move"]
                    }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "find_analog_windows",
            "memory",
            Risk::Read,
            "Find past windows whose explicit fingerprint is closest to an instrument's latest one. The same \
             instrument's neighbouring windows are excluded for one window length, so an analog is never the same episode.",
            json!({
                "type": "object",
                "required": ["instrument_id", "venue_id"],
                "additionalProperties": false,
                "properties": {
                    "instrument_id": { "type": "integer" },
                    "venue_id": { "type": "integer" },
                    "k": { "type": "integer", "minimum": 1, "maximum": 50, "description": "how many analogs, default 5" },
                    "as_of": { "type": "string", "description": "RFC 3339; only fingerprints known by then are used" }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "tag_hypothesis_source",
            "memory",
            Risk::Write,
            "Say where a hypothesis came from, as `memory:<finding id>` when a stored finding suggested it. A repeat of an \
             earlier hypothesis on the same instrument and window is flagged as a repeated trial. Findings are refused as evidence.",
            json!({
                "type": "object",
                "required": ["hypothesis_id", "source", "instrument", "window_start", "window_end"],
                "additionalProperties": false,
                "properties": {
                    "hypothesis_id": { "type": "string" },
                    "source": { "type": "string", "description": "memory:<finding id>, or a plain description" },
                    "instrument": { "type": "string" },
                    "window_start": { "type": "string", "description": "YYYY-MM-DD" },
                    "window_end": { "type": "string", "description": "YYYY-MM-DD" },
                    "evidence_ids": { "type": "array", "items": { "type": "string" }, "description": "what the experiment rests on; never findings" }
                }
            }),
        )
        .internal()
        .max_calls_per_run(20),
        ToolSpec::new(
            "get_regime_profile",
            "memory",
            Risk::Read,
            "The regime history of an instrument's market: how much time it spends in each state, how states follow one another, \
             how long a stay lasts and the current label. An instrument with no labels is a refusal, not an empty profile.",
            json!({
                "type": "object",
                "required": ["instrument", "asset_class"],
                "additionalProperties": false,
                "properties": {
                    "instrument": { "type": "string" },
                    "asset_class": { "type": "string" }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "get_regime_prior",
            "memory",
            Risk::Read,
            "Which strategy families did best in each regime for an instrument, each with the number of observations it rests on. \
             A thin count is a hint to explore, not a verdict.",
            json!({
                "type": "object",
                "required": ["instrument", "asset_class"],
                "additionalProperties": false,
                "properties": {
                    "instrument": { "type": "string" },
                    "asset_class": { "type": "string" },
                    "regime": { "type": "string", "enum": ["low_vol", "mid_vol", "high_vol"] }
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "compare_to_random",
            "memory",
            Risk::Read,
            "Compare a campaign with the matched random-search arm created beside it: both arms' sealed-metric distributions and where \
             your best sits among random search's best-of-budget outcomes. A campaign with no completed arm refuses.",
            json!({
                "type": "object",
                "required": ["campaign_id"],
                "additionalProperties": false,
                "properties": { "campaign_id": { "type": "string" } }
            }),
        )
        .internal(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_knowledge_tool_reaches_a_platform_write_route() {
        let src = include_str!("knowledge_model.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "/moves/records",
            "/moves/overrides",
            "regimes/prior\", json",
            "/random-arm",
            "/scores",
        ] {
            assert!(!prod.contains(forbidden), "a knowledge tool mentions {forbidden}, a platform-write route");
        }
        // The only POSTs are the veto question and the source tag.
        assert_eq!(prod.matches(".post(").count(), 2);
    }

    #[test]
    fn six_tools_in_the_memory_namespace_and_every_schema_spells_its_fields() {
        let sp = specs();
        assert_eq!(sp.len(), 6);
        for t in &sp {
            assert_eq!(t.namespace, "memory");
            assert!(t.input_schema.get("properties").and_then(Value::as_object).is_some_and(|p| !p.is_empty()), "{}", t.name);
        }
    }
}
