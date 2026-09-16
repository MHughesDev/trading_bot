//! The feature catalogue, exposed (AGENT-011 §4; Set S task S-5.2).
//!
//! # Why this is the highest-leverage tool in the phase
//!
//! `crates/features` has been a real feature engine for some time — around
//! fifty families, and `crates/backtest/src/sim.rs` resolves every one of them
//! through `features::runtime::feature`. Verified by construction, not by
//! reading: `atr_14`, `parkinson_vol_20`, `rolling_std_30`, `donchian_upper_20`,
//! `obv` and `hour_sin` all resolve and run.
//!
//! The agent could not see any of it. The MCP server's own `initialize`
//! instructions said "backtests support only ema_N and rsi_N features" — a
//! sentence every model read at the start of every session, and which had become
//! false. A capability ceiling written in prose is still a ceiling: a model that
//! is told it has two indicators will use two indicators.
//!
//! So this serves the registry the engine already answers to. The registry is
//! the single source — `list` here and `runtime::feature` in the simulator read
//! the same `FAMILIES` table, so a family that is listed is a family that
//! resolves, and there is no second catalogue to drift.
//!
//! # Answered locally
//!
//! Static reference data with no tenant and no row, like `list_lanes` and
//! `get_venue_constraints`. It costs no round trip, and a family is added by
//! editing the registry the simulator reads rather than by editing a list beside
//! it.

use serde_json::{json, Value};

use crate::ApiClient;

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;

/// How many families a single answer may carry.
///
/// The whole catalogue rendered at once is a few thousand tokens, which on a
/// flat tier is most of a context window spent on a menu. The filters are the
/// intended path and the cap is what makes ignoring them survivable.
const MAX_FAMILIES: usize = 40;

fn cost_of(c: features::CostClass) -> &'static str {
    match c {
        features::CostClass::Cheap => "cheap",
        features::CostClass::Rolling => "rolling",
        features::CostClass::Model => "model",
    }
}

fn phase_of(p: features::Phase) -> &'static str {
    match p {
        features::Phase::F1 => "f1",
        features::Phase::F2 => "f2",
    }
}

fn render(f: &'static features::Family) -> Value {
    json!({
        "family": f.name,
        // The name a caller actually writes. A family with parameters is not
        // usable from its bare name, and an example is worth more than a grammar.
        "example": f.example(),
        "parameters": f.params.iter().map(|p| json!({
            "name": p.name,
            "min": p.min,
            "max": p.max,
        })).collect::<Vec<_>>(),
        "use_when": f.use_when,
        "cost": cost_of(f.cost),
        "phase": phase_of(f.phase),
        "warmup_bars": f.lookback_bars(&f.example_params()),
        // An alias is rewritten to its canonical family before anything is
        // constructed (INV-14), so it is reported rather than hidden: a caller
        // reading old code needs to know the name still works.
        "alias_of": f.alias_of,
    })
}

/// `list_features` — what the engine can actually compute.
#[must_use]
pub fn list_features(params: &Value) -> Value {
    let text = |k: &str| {
        params
            .get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };

    let cost = match text("cost") {
        Some("cheap") => Some(features::CostClass::Cheap),
        Some("rolling") => Some(features::CostClass::Rolling),
        Some("model") => Some(features::CostClass::Model),
        Some(other) => {
            return json!({
                "error": "unknown_cost",
                "cost": other,
                "known": ["cheap", "rolling", "model"],
            })
        }
        None => None,
    };

    let all = features::feature_families().len();
    let matched = features::list_features(text("family"), cost, text("query"));
    let shown: Vec<Value> = matched.iter().take(MAX_FAMILIES).map(|f| render(f)).collect();
    let truncated = matched.len().saturating_sub(shown.len());

    json!({
        "summary_fields": {
            "n_shown": shown.len(),
            "n_matched": matched.len(),
            "n_total": all,
        },
        "detail": {
            "families": shown,
            "n_shown": shown.len(),
            "n_matched": matched.len(),
            "n_total": all,
            "n_truncated": truncated,
            "note": "every family listed here is one the backtest engine resolves — this reads \
                     the same registry `features::runtime::feature` does, so a name that \
                     appears here is a name a strategy may use. Write the `example` form: a \
                     family with parameters does not resolve from its bare name.",
        },
        "ids": {},
    })
}

/// `preview_signals` — what a definition would have done, before buying a trial.
///
/// Routed rather than answered here: it reads bars, and every path to bars goes
/// through the API's project resolution and point-in-time store. A local
/// shortcut would be a second read path, which is the one thing the data layer
/// does not have.
pub async fn preview_signals(api: &ApiClient, params: &Value) -> Value {
    for field in ["definition", "instrument_id", "start", "end"] {
        if params.get(field).is_none() {
            return json!({ "error": "missing_field", "field": field });
        }
    }
    match api.post("/api/agent/strategy/preview", params.clone()).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

pub fn specs() -> Vec<ToolSpec> {
    vec![
    ToolSpec::new(
        "list_features",
        "strategy",
        Risk::Read,
        "Every feature a strategy may use, with an example name, when to reach for it and its \
         warm-up cost. Call it before writing a definition — the engine computes far more than \
         moving averages.",
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "match on name or use_when"},
                "family": {"type": "string", "description": "one exact family name"},
                "cost": {
                    "type": "string",
                    "enum": ["cheap", "rolling", "model"],
                    "description": "cheap is O(1) per bar; rolling scans a window"
                }
            }
        }),
    )
    .internal()
    .summary(
        "{n_shown} of {n_matched} matching families ({n_total} in the engine)",
        &[("n_shown", ""), ("n_matched", ""), ("n_total", "")],
    )
    .notes("list_features"),
    ToolSpec::new(
        "preview_signals",
        "strategy",
        Risk::Read,
        "What a strategy definition would have done on a window: how many entries, on which          bars, with the feature values that caused them — and, when it never fires, which of          the several possible reasons it was. Spends no trial. Use it to check a draft before          paying for a backtest; it reports signals only, never P&L.",
        json!({
            "type": "object",
            "required": ["definition", "instrument_id", "start", "end"],
            "properties": {
                "definition": {
                    "type": "object",
                    "description": "the strategy definition to preview — a draft, not a saved id"
                },
                "instrument_id": {"type": "string"},
                "timeframe": {"type": "string", "default": "1m"},
                "start": {"type": "string", "description": "RFC3339"},
                "end": {
                    "type": "string",
                    "description": "RFC3339. A window reaching into the sealed holdout is                                     refused by name rather than shortened."
                },
                "project_id": {"type": "string", "description": "defaults to your Desk"}
            }
        }),
    )
    // Stated rather than left to the default. This is the whole reason the tool
    // exists: the cheapest question an author asks used to cost a trial, and a
    // reader of this spec should be able to see that it no longer does.
    .spends_trial(crate::spec::TrialKind::None)
    .summary(
        "{entries} entries over {bars_evaluated} evaluated bars",
        &[("entries", ""), ("bars_evaluated", "")],
    )
    .notes("preview_signals"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim the tool exists to correct: the engine is not two indicators.
    #[test]
    fn the_catalogue_is_far_more_than_ema_and_rsi() {
        let out = list_features(&json!({}));
        let total = out["detail"]["n_total"].as_u64().unwrap_or(0);
        assert!(
            total > 20,
            "the registry reports {total} families; `server_instructions` used to tell every \
             model there were two"
        );
    }

    /// Listed and resolvable are the same set, because they are the same table.
    ///
    /// This is the property that makes the tool safe to trust: a model that picks
    /// a name from here and puts it in a strategy must not then be told the
    /// feature is unknown.
    #[test]
    fn every_example_name_resolves_in_the_engine() {
        for f in features::feature_families() {
            let example = f.example();
            assert!(
                features::resolve_feature(&example).is_some(),
                "`{}` advertises the example `{example}`, which the registry does not resolve",
                f.name
            );
        }
    }

    #[test]
    fn an_unknown_cost_class_is_named_rather_than_ignored() {
        let out = list_features(&json!({ "cost": "free" }));
        assert_eq!(out["error"], "unknown_cost");
    }

    /// A filter narrows, and the answer says how much it left out.
    #[test]
    fn the_answer_reports_what_it_truncated() {
        let out = list_features(&json!({}));
        let shown = out["detail"]["n_shown"].as_u64().unwrap_or(0);
        let matched = out["detail"]["n_matched"].as_u64().unwrap_or(0);
        let truncated = out["detail"]["n_truncated"].as_u64().unwrap_or(0);
        assert_eq!(shown + truncated, matched);
        assert!(shown <= MAX_FAMILIES as u64);
    }
}
