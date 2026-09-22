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

use domain::strategy_def::StrategyDefinition;

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

// ── SLv2: the language, exposed (S-5.8, S-5.10) ──────────────────────────────
//
// `crates/strategy-lang` is thirteen thousand lines with, until this task, no
// caller anywhere in the workspace — the shape this repository keeps finding:
// complete, tested code nothing reaches. It parses, compiles, decompiles, diffs
// and explains SLv2, and a model could not call any of it.
//
// Answered locally for the reason `list_features` is: the crate is a pure
// function of one document. No tenant, no row, no bars. Routing it through the
// API would add a round trip to a call that cannot fail for any reason the
// platform knows about.

/// The two ways a caller may hand over a strategy, and exactly one of them.
///
/// A single free-form field taking "text or JSON" would be a value nothing
/// checks: the two are different languages, and telling them apart by sniffing
/// the first character is how a `{` inside a comment becomes a parse error
/// about JSON.
enum Source {
    /// SLv2 text.
    Text(String),
    /// The canonical stored form.
    Definition(Box<StrategyDefinition>),
}

fn read_source(params: &Value) -> Result<Source, Value> {
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let definition = params.get("definition").filter(|v| !v.is_null());
    match (text, definition) {
        (Some(t), None) => Ok(Source::Text(t.to_string())),
        (None, Some(d)) => match serde_json::from_value::<StrategyDefinition>(d.clone()) {
            Ok(def) => Ok(Source::Definition(Box::new(def))),
            Err(e) => Err(json!({
                "error": "invalid_definition",
                "message": e.to_string(),
                "fix": "send the object `get_strategy` returned, or send SLv2 in `text` instead",
            })),
        },
        (None, None) => Err(json!({
            "error": "missing_field",
            "field": "text",
            "fix": "send SLv2 in `text`, or a stored definition object in `definition`",
        })),
        (Some(_), Some(_)) => Err(json!({
            "error": "ambiguous_source",
            "fix": "send `text` or `definition`, not both — they are different languages and \
                    this tool will not guess which one you meant",
        })),
    }
}

/// A source, as the canonical definition plus the SLv2 that reads it back.
fn resolve(params: &Value) -> Result<(StrategyDefinition, String), Value> {
    match read_source(params)? {
        Source::Text(t) => match strategy_lang::text_to_definition(&t) {
            Ok(def) => {
                // The text a reader is shown is the canonical one, not the one
                // that was typed: two documents that compile to the same
                // definition must explain identically, or `explain_strategy`
                // becomes a report on the author's whitespace.
                let canonical = strategy_lang::decompile(&def).map_err(one_diagnostic)?;
                Ok((def, canonical))
            }
            Err(ds) => Err(diagnostics(&ds)),
        },
        Source::Definition(def) => {
            let text = strategy_lang::decompile(&def).map_err(one_diagnostic)?;
            Ok((*def, text))
        }
    }
}

fn render_diagnostic(d: &strategy_lang::Diagnostic) -> Value {
    json!({
        "code": d.code.as_str(),
        "line": d.span.line,
        "col": d.span.col,
        "message": d.message,
        "fix": d.fix,
        "see": d.see,
    })
}

fn diagnostics(ds: &[strategy_lang::Diagnostic]) -> Value {
    json!({
        "error": "compile_failed",
        "n_diagnostics": ds.len(),
        "diagnostics": ds.iter().map(render_diagnostic).collect::<Vec<_>>(),
    })
}

/// A refusal the language raised about a *stored* definition: it holds something
/// SLv2 has no text for. Its own code, because the fix is different — nothing
/// the caller writes will help, and the answer names the mechanism that is
/// missing rather than the clause.
fn one_diagnostic(d: strategy_lang::Diagnostic) -> Value {
    json!({
        "error": "not_expressible_in_slv2",
        "diagnostic": render_diagnostic(&d),
    })
}

/// `compile_strategy` — SLv2 text to the canonical definition. Persists nothing.
#[must_use]
pub fn compile_strategy(params: &Value) -> Value {
    let Some(text) = params.get("text").and_then(Value::as_str) else {
        return json!({ "error": "missing_field", "field": "text" });
    };
    let def = match strategy_lang::text_to_definition(text) {
        Ok(d) => d,
        Err(ds) => return diagnostics(&ds),
    };
    let Ok(definition) = serde_json::to_value(&def) else {
        return json!({ "error": "serialization_error" });
    };
    json!({
        "summary_fields": { "strategy_id": def.strategy_id, "n_diagnostics": 0 },
        "detail": {
            "definition": definition,
            "note": "nothing was persisted. Pass this object to `create_strategy` when you want \
                     it saved, or to `preview_signals` to see what it would have done.",
        },
        "ids": {},
    })
}

/// `explain_strategy` — what a strategy says, clause by clause, with its
/// warm-up cost.
///
/// # Warm-up is refused as a whole when one feature is unknown
///
/// The longest lookback among the declared features is what a caller needs, and
/// a `max` over the ones that happened to resolve would be a smaller number
/// wearing the same name. A feature the engine does not know contributes no
/// lookback, and that is not zero: the aggregate is `null` and the unresolved
/// names are listed beside it.
#[must_use]
pub fn explain_strategy(params: &Value) -> Value {
    let (def, text) = match resolve(params) {
        Ok(v) => v,
        Err(e) => return e,
    };

    let declared: Vec<String> = def
        .inputs
        .iter()
        .flat_map(|i| i.features.iter().cloned())
        .collect();
    let mut unresolved: Vec<&str> = Vec::new();
    let mut inputs: Vec<Value> = Vec::new();
    let mut longest: Option<u32> = None;
    for name in &declared {
        match features::runtime::lookback_bars(name) {
            Some(bars) => {
                longest = Some(longest.map_or(bars, |m: u32| m.max(bars)));
                inputs.push(json!({ "feature": name, "warmup_bars": bars }));
            }
            None => {
                unresolved.push(name);
                // No `warmup_bars` key at all rather than a null or a zero: the
                // engine was asked and had no answer, and a row carrying the
                // number 0 would be read as "this one is free".
                inputs.push(json!({ "feature": name, "unknown_to_the_engine": true }));
            }
        }
    }
    let warmup_bars = if unresolved.is_empty() { longest } else { None };

    let entries = match strategy_lang::entry_clauses(&def) {
        Ok(e) => e,
        Err(d) => return one_diagnostic(d),
    };
    let entries_json: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "path": e.path,
                "side": e.side.as_str(),
                "clauses": e.clauses.iter().map(|c| json!({
                    "path": c.path,
                    "reads": c.text,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let n_clauses: usize = entries.iter().map(|e| e.clauses.len()).sum();

    // The exits and the size rule are read off the canonical text rather than
    // off the stored form: `decompile` is the one renderer, and a second one
    // here would disagree with it the first time an exit kind changed shape.
    let clause = |kw: &str| -> Vec<String> {
        text.lines()
            .filter(|l| l.split_whitespace().next() == Some(kw))
            .map(str::to_string)
            .collect()
    };
    let exits = clause("exit");
    let size = clause("size");

    json!({
        "summary_fields": {
            "strategy_id": def.strategy_id,
            "n_entries": entries.len(),
            "n_clauses": n_clauses,
            "n_exits": exits.len(),
        },
        "detail": {
            "strategy_id": def.strategy_id,
            "text": text,
            "inputs": inputs,
            "entries": entries_json,
            "exits": exits,
            "size": size,
            // `null` is the honest answer when a declared feature did not
            // resolve, and `warmup_is_unknown_because` says which.
            "warmup_bars": warmup_bars,
            "warmup_is_unknown_because": (!unresolved.is_empty()).then(|| json!({
                "unresolved_features": unresolved,
                "fix": "call `list_features` and write the `example` form — a family with \
                        parameters does not resolve from its bare name",
            })),
            "note": "warm-up is the longest lookback among the features this definition \
                     declares. `preview_signals` reports the warm-up the executor derives, \
                     which is the number a backtest will use.",
        },
        "ids": {},
    })
}

/// `diff_strategies` — what changed, in the FEAT-004 §6 move vocabulary.
#[must_use]
pub fn diff_strategies(params: &Value) -> Value {
    let read = |field: &str| -> Result<StrategyDefinition, Value> {
        let Some(v) = params.get(field).filter(|v| !v.is_null()) else {
            return Err(json!({ "error": "missing_field", "field": field }));
        };
        serde_json::from_value(v.clone()).map_err(|e| {
            json!({ "error": "invalid_definition", "field": field, "message": e.to_string() })
        })
    };
    let before = match read("before") {
        Ok(d) => d,
        Err(e) => return e,
    };
    let after = match read("after") {
        Ok(d) => d,
        Err(e) => return e,
    };
    let d = match strategy_lang::diff(&before, &after) {
        Ok(d) => d,
        Err(diag) => return one_diagnostic(diag),
    };
    let changes: Vec<Value> = d
        .changes()
        .iter()
        .map(|c| {
            json!({
                "kind": c.kind.as_str(),
                "path": c.path,
                "before": c.before,
                "after": c.after,
                "says": c.words,
            })
        })
        .collect();
    json!({
        "summary_fields": {
            "n_moves": d.n_moves(),
            "is_single_change": d.is_single_change(),
        },
        "detail": {
            "n_moves": d.n_moves(),
            "is_single_change": d.is_single_change(),
            "changes": changes,
            "renamed": d.renamed().map(|(a, b)| json!({ "from": a, "to": b })),
            "reads_as": d.render(),
            "note": "a version carrying more than one move cannot be attributed to either of \
                     them. The research protocol asks for one change at a time, and \
                     `is_single_change` is how that becomes checkable rather than exhorted.",
        },
        "ids": {},
    })
}

/// `apply_move` — one typed edit in, one new version out.
///
/// It never persists. A version that has not been previewed or backtested is a
/// draft, and `create_strategy` is where a draft becomes something evidence can
/// cite — under the *same* slug, because saving appends a revision whose parent
/// is the version this move edited. That chain is the lineage; a new slug throws
/// it away (ADR-D-21).
#[must_use]
pub fn apply_move(params: &Value) -> Value {
    let Some(v) = params.get("definition").filter(|v| !v.is_null()) else {
        return json!({ "error": "missing_field", "field": "definition" });
    };
    let def: StrategyDefinition = match serde_json::from_value(v.clone()) {
        Ok(d) => d,
        Err(e) => {
            return json!({
                "error": "invalid_definition",
                "message": e.to_string(),
            })
        }
    };
    let Some(raw) = params.get("edit").filter(|v| !v.is_null()) else {
        return json!({ "error": "missing_field", "field": "edit" });
    };
    // The edit is a closed enum with typed clause fills, so an unreadable one
    // is refused here with serde's own message rather than becoming a
    // half-applied version. A free-form "what to change" string is the defect
    // class this repository keeps finding.
    let edit: strategy_lang::Edit = match serde_json::from_value(raw.clone()) {
        Ok(e) => e,
        Err(e) => {
            return json!({
                "error": "unreadable_edit",
                "message": e.to_string(),
                "fix": "an edit names its kind in `edit` and carries that kind's fields; clause \
                        addresses are the ones `explain_strategy` prints, such as \
                        `entry.long[0].clause[1]`",
            })
        }
    };

    let applied = match strategy_lang::apply_move(&def, &edit) {
        Ok(a) => a,
        Err(d) => {
            return json!({
                "error": d.code.as_str(),
                "message": d.message,
                "fix": d.fix,
                "see": d.see,
            })
        }
    };
    let kind = applied.kind();
    let says = applied.words();
    let text = applied.text().to_string();
    let new_id = params
        .get("strategy_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let out = match new_id {
        Some(id) => applied.into_version(id),
        None => applied.into_definition(),
    };
    let Ok(definition) = serde_json::to_value(&out) else {
        return json!({ "error": "serialization_error" });
    };
    json!({
        "summary_fields": { "kind": kind.as_str(), "says": says },
        "detail": {
            "kind": kind.as_str(),
            "says": says,
            "definition": definition,
            "text": text,
            "note": "one move, checked by running the differ over the result — an edit that \
                     changed nothing or changed two things is refused rather than returned. \
                     Nothing was persisted. `create_strategy` appends this as the next revision \
                     of whatever slug it carries, so leaving `strategy_id` alone keeps the \
                     lineage; passing a new one starts a separate history.",
        },
        "ids": {},
    })
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
    ToolSpec::new(
        "compile_strategy",
        "strategy",
        Risk::Read,
        "Turn SLv2 text into the canonical strategy definition, or into every reason it is not \
         one. Persists nothing, spends nothing; the object it returns is what create_strategy \
         and preview_signals take.",
        json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {
                    "type": "string",
                    "description": "SLv2: one clause per line, keywords first"
                }
            }
        }),
    )
    .internal()
    .summary(
        "compiled `{strategy_id}` with {n_diagnostics} diagnostics",
        &[("strategy_id", ""), ("n_diagnostics", "")],
    ),
    ToolSpec::new(
        "explain_strategy",
        "strategy",
        Risk::Read,
        "Read a strategy back in words: every input with its warm-up, every entry clause by \
         clause, the exits and the size rule. Send SLv2 in `text` or a stored definition in \
         `definition`.",
        json!({
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "SLv2 — exactly one of this or `definition`"},
                "definition": {"type": "object", "description": "a stored definition object"}
            }
        }),
    )
    .internal()
    .summary(
        "`{strategy_id}`: {n_entries} entries over {n_clauses} clauses, {n_exits} exits",
        &[("strategy_id", ""), ("n_entries", ""), ("n_clauses", ""), ("n_exits", "")],
    ),
    ToolSpec::new(
        "diff_strategies",
        "strategy",
        Risk::Read,
        "What changed between two strategy definitions, classified into the move vocabulary and \
         said in words. `is_single_change` is how \"change one thing at a time\" becomes \
         checkable.",
        json!({
            "type": "object",
            "required": ["before", "after"],
            "properties": {
                "before": {"type": "object", "description": "the earlier definition"},
                "after": {"type": "object", "description": "the later definition"}
            }
        }),
    )
    .internal()
    .summary(
        "{n_moves} moves; single change: {is_single_change}",
        &[("n_moves", ""), ("is_single_change", "")],
    ),
    ToolSpec::new(
        "apply_move",
        "strategy",
        Risk::Read,
        "Make one typed structural edit to a strategy and get the new version back, with the \
         move it turned out to be. An edit that changes nothing, or two things, is refused. \
         Persists nothing.",
        json!({
            "type": "object",
            "required": ["definition", "edit"],
            "properties": {
                "definition": {"type": "object", "description": "the version to edit"},
                "edit": {
                    "type": "object",
                    "description": "one typed edit: {edit: threshold|add_filter|remove_filter|replace_filter|timeframe|sizing|add_exit|replace_exit|remove_exit, …}. Clause addresses are the ones explain_strategy prints."
                },
                "strategy_id": {
                    "type": "string",
                    "description": "OPTIONAL slug override. Leave it unset to keep the same slug: create_strategy appends the edited version as a new revision of that slug's history and the previous one stays readable, so your evidence still resolves. Set it only to start a separate lineage."
                }
            }
        }),
    )
    .internal()
    .summary("{kind}: {says}", &[("kind", ""), ("says", "")]),
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

    // ── SLv2 (S-5.8, S-5.10) ────────────────────────────────────────────────

    const TREND: &str = "\
strategy trend_v1 on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21)
size fixed 0.01
exit stop 2 % from entry
exit time 48 bars
";

    fn compiled(text: &str) -> Value {
        let out = compile_strategy(&json!({ "text": text }));
        assert!(
            out.get("error").is_none(),
            "expected a compile, got {out:#}"
        );
        out["detail"]["definition"].clone()
    }

    /// The front door: text a model can write becomes the object every other
    /// tool already takes, and nothing is saved on the way through.
    #[test]
    fn compile_strategy_turns_text_into_the_stored_form_and_persists_nothing() {
        let out = compile_strategy(&json!({ "text": TREND }));
        assert_eq!(out["summary_fields"]["strategy_id"], "trend_v1");
        assert_eq!(out["detail"]["definition"]["strategy_id"], "trend_v1");
        assert!(
            out["detail"]["note"]
                .as_str()
                .is_some_and(|n| n.contains("nothing was persisted")),
            "the tool must say it saved nothing, or a model will call create_strategy twice"
        );
        let def: domain::strategy_def::StrategyDefinition =
            serde_json::from_value(out["detail"]["definition"].clone()).expect("a definition");
        assert_eq!(def.definition_version, "1.3", "the exits block is a 1.3 document");
    }

    /// A refusal names the clause and carries a fix, because the whole argument
    /// of AGENT-011 §5 is that a model iterates against the diagnostics.
    #[test]
    fn a_compile_failure_comes_back_as_diagnostics_with_fixes() {
        let out = compile_strategy(&json!({
            "text": "strategy s on $bound_at_init timeframe 1h\n\
                     entry long when nope(close, 14) < 30\n\
                     size fixed 1\n\
                     exit none\n"
        }));
        assert_eq!(out["error"], "compile_failed");
        let ds = out["diagnostics"].as_array().expect("diagnostics");
        assert!(!ds.is_empty());
        assert!(
            ds.iter().all(|d| d["fix"].is_string()),
            "a diagnostic with no fix leaves a small model guessing: {ds:#?}"
        );
        assert!(ds.iter().any(|d| d["code"] == "unknown_feature"));
    }

    /// S-5.8's acceptance: every input, entry, exit, size rule and the warm-up.
    #[test]
    fn explain_strategy_names_every_input_entry_exit_size_and_the_warmup() {
        let out = explain_strategy(&json!({ "text": TREND }));
        assert!(out.get("error").is_none(), "{out:#}");
        let d = &out["detail"];
        assert_eq!(d["strategy_id"], "trend_v1");

        let inputs = d["inputs"].as_array().expect("inputs");
        assert_eq!(inputs.len(), 2, "two EMAs: {inputs:#?}");
        assert!(inputs.iter().all(|i| i["warmup_bars"].is_u64()));

        let entries = d["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "entry.long[0]");
        assert_eq!(entries[0]["clauses"].as_array().expect("clauses").len(), 1);

        assert_eq!(d["exits"].as_array().expect("exits").len(), 2);
        assert_eq!(d["size"].as_array().expect("size").len(), 1);
        assert!(d["size"][0].as_str().is_some_and(|s| s.starts_with("size ")));

        // `ema_21` declares a 105-bar warm-up (5N), and the longest wins. The
        // number matters less than that it is the engine's own, not a second
        // arithmetic here.
        let longest = d["warmup_bars"].as_u64().expect("a warm-up");
        assert_eq!(longest, features::runtime::lookback_bars("ema_21").expect("ema_21 resolves") as u64);
        assert!(d["warmup_is_unknown_because"].is_null());
    }

    /// **An absent measurement is never a zero.**
    ///
    /// A definition reading `feature('rsi14')` is the exact shape S-5.3 found in
    /// the wild: the validator passed it and `features::runtime::feature` failed
    /// during the run, after a trial had been spent. The engine has no lookback
    /// for that name, so the longest warm-up is *unknown* — not the longest of
    /// the ones that happened to resolve, and not zero.
    #[test]
    fn a_feature_the_engine_cannot_resolve_makes_the_warmup_unknown_rather_than_zero() {
        let def = compiled(TREND);
        // Rename one declared-and-used feature to a spelling the registry does
        // not know. Both sides move together, because a definition whose
        // declared and used features disagree is refused before it gets here.
        let mutated: Value = serde_json::from_str(
            &serde_json::to_string(&def)
                .expect("serialises")
                .replace("ema_21", "ema21"),
        )
        .expect("still json");
        assert!(
            features::runtime::lookback_bars("ema21").is_none(),
            "the fixture depends on `ema21` being unknown to the engine"
        );

        let out = explain_strategy(&json!({ "definition": mutated }));
        assert!(out.get("error").is_none(), "{out:#}");
        let d = &out["detail"];
        assert!(
            d["warmup_bars"].is_null(),
            "the longest warm-up is unknown, and reporting `ema_7`'s 35 bars would be a \
             smaller number wearing the same name: {d:#}"
        );
        let why = &d["warmup_is_unknown_because"];
        assert_eq!(why["unresolved_features"][0], "ema21");

        // And the row for the unknown feature carries no warm-up at all rather
        // than a zero a reader would take for "free".
        let row = d["inputs"]
            .as_array()
            .expect("inputs")
            .iter()
            .find(|i| i["feature"] == "ema21")
            .expect("the unknown feature is still listed");
        assert!(row.get("warmup_bars").is_none(), "{row:#}");
        assert_eq!(row["unknown_to_the_engine"], true);
    }

    /// Adversarial: two languages, and the tool refuses to guess which was meant.
    #[test]
    fn explain_strategy_refuses_two_sources_rather_than_picking_one() {
        let out = explain_strategy(&json!({ "text": TREND, "definition": compiled(TREND) }));
        assert_eq!(out["error"], "ambiguous_source");

        let none = explain_strategy(&json!({}));
        assert_eq!(none["error"], "missing_field");
    }

    /// S-5.10's acceptance, the half a differ can prove on its own: a one-move
    /// edit is one move, at the address the language uses.
    #[test]
    fn a_one_move_edit_diffs_as_one_move() {
        let before = compiled(TREND);
        let after = compiled(&TREND.replace("exit time 48 bars", "exit time 96 bars"));
        let out = diff_strategies(&json!({ "before": before, "after": after }));
        assert!(out.get("error").is_none(), "{out:#}");
        assert_eq!(out["summary_fields"]["n_moves"], 1);
        assert_eq!(out["summary_fields"]["is_single_change"], true);
        let c = &out["detail"]["changes"][0];
        // `change_threshold`, not `change_exit`: one number moved and nothing
        // else did, which is the more specific of the two true statements and
        // the one a reader can act on.
        assert_eq!(c["kind"], "change_threshold", "{:#}", out["detail"]);
        assert!(
            c["says"].as_str().is_some_and(|w| !w.contains("nodes")),
            "the words a reader sees must be about the strategy, not about its JSON: {c:#}"
        );
    }

    /// And the direction that makes the rule worth having: a version carrying
    /// two moves cannot be attributed to either of them, and says so.
    #[test]
    fn two_moves_in_one_version_are_not_a_single_change() {
        let before = compiled(TREND);
        let after = compiled(
            &TREND
                .replace("exit time 48 bars", "exit time 96 bars")
                .replace("size fixed 0.01", "size risk_per_trade 0.01"),
        );
        let out = diff_strategies(&json!({ "before": before, "after": after }));
        assert_eq!(out["summary_fields"]["is_single_change"], false);
        assert_eq!(out["summary_fields"]["n_moves"], 2);
    }


    /// S-5.10's acceptance across the tool boundary: a move applied by
    /// `apply_move` comes back from `diff_strategies` as that move, with the
    /// two speaking JSON to each other the way a model would.
    #[test]
    fn a_move_applied_by_the_tool_diffs_back_as_that_move() {
        let before = compiled(TREND);
        let applied = apply_move(&json!({
            "definition": before,
            "strategy_id": "trend_v2",
            "edit": {
                "edit": "add_filter",
                "entry": "entry.long[0]",
                "filter": {
                    "filter": "compare",
                    "lhs": {"operand": "feature", "family": "rsi", "args": [14]},
                    "op": "lt",
                    "rhs": {"operand": "number", "value": "60"}
                }
            }
        }));
        assert!(applied.get("error").is_none(), "{applied:#}");
        assert_eq!(applied["summary_fields"]["kind"], "add_filter");
        let after = applied["detail"]["definition"].clone();
        assert_eq!(after["strategy_id"], "trend_v2");

        let d = diff_strategies(&json!({ "before": before, "after": after }));
        assert_eq!(d["summary_fields"]["is_single_change"], true);
        assert_eq!(d["detail"]["changes"][0]["kind"], "add_filter");
        // The rename is not a move, or every iteration would read as two.
        assert_eq!(d["detail"]["renamed"]["to"], "trend_v2");
    }

    /// An edit that changes nothing is refused with a routable code, not
    /// returned as a version identical to the one it came from.
    #[test]
    fn an_edit_that_moves_nothing_is_refused_by_name() {
        let out = apply_move(&json!({
            "definition": compiled(TREND),
            "edit": { "edit": "timeframe", "to": "1h" }
        }));
        assert_eq!(out["error"], "nothing_to_move");
        assert!(out["fix"].is_string());
    }

    /// Adversarial: the address is a typed value, so a string that is not one
    /// is refused at the boundary rather than defaulted into an edit of clause
    /// zero.
    #[test]
    fn an_address_that_is_not_one_is_refused_rather_than_defaulted() {
        let out = apply_move(&json!({
            "definition": compiled(TREND),
            "edit": { "edit": "threshold", "clause": "entry.long.0", "to": "45" }
        }));
        assert_eq!(out["error"], "unreadable_edit");
        assert!(
            out["fix"].as_str().is_some_and(|f| f.contains("entry.long[0].clause[1]")),
            "the refusal must show the shape that works: {out:#}"
        );
    }

    /// The new tools are declared, and the namespace still routes.
    #[test]
    fn the_language_is_in_the_catalogue() {
        for name in ["compile_strategy", "explain_strategy", "diff_strategies", "apply_move"] {
            let spec = crate::spec::find(name)
                .unwrap_or_else(|| panic!("`{name}` is not declared, so nothing can call it"));
            assert_eq!(spec.namespace, "strategy");
            assert_eq!(spec.spends_trial, crate::spec::TrialKind::None);
        }
    }
}
