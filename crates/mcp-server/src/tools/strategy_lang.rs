//! The SLv2 tools that need more than one pure call (Set S phase 5: S-5.8,
//! S-5.11, S-5.12, S-5.13, S-5.15).
//!
//! `strategy.rs` answers the tools that are a pure function of one document
//! (`compile_strategy`, `explain_strategy`, `diff_strategies`, `apply_move`).
//! These are the ones that either carry a text door into a tool that used to
//! take only JSON (`validate_strategy`, `create_strategy`), serve the language's
//! own reference (`strategy_skeleton`, `get_slv2_guide`), or measure a strategy
//! against bars (`blocking_profile`, `suggest_moves`).
//!
//! # The measuring tools do not walk a bar
//!
//! `strategy_lang::probes` turns "which clause blocked the entry?" into a few
//! previews of the same definition with one expression swapped in, and the
//! platform's own preview route — the same one `preview_signals` calls, with the
//! same holdout refusal — answers each. The tool only asks and folds. A second
//! evaluator here would be a second set of answers about when a clause is true
//! (Set U's "no second evaluator"), and the preview already reports the one
//! number every probe needs: how many bars a signal was true on.
//!
//! Nothing here spends a trial: every read is a preview.
//!
//! # SLv2 and SLv3
//!
//! `text` here is **SLv2**, the text of v1 definitions. SLv3 (Set U) has its own
//! tools (`*_strategy_v3`) and its own grammar, and a v3 header handed to an SLv2
//! tool is refused by name with the pointer, not parsed into a confusing
//! diagnostic about a clause the author did not write.

use std::collections::BTreeMap;
use std::future::Future;

use domain::strategy_def::StrategyDefinition;
use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;
use crate::tools::authoring::{self, ValidationErrorItem};
use crate::ApiClient;

/// The most previews one measuring call may spend.
///
/// A one-clause entry costs 1, a two-clause entry 3, a three-clause entry 7 and
/// each clause beyond adds two more. Reads, not trials — but each walks a window
/// of bars, so an entry with many clauses is refused by name rather than run
/// for minutes.
pub const MAX_PREVIEWS: usize = 24;

// ── the text door ────────────────────────────────────────────────────────────

/// Whether `text` opens with an SLv3 header (`strategy <name> kind <…>`).
///
/// The SLv2 header is `strategy <name> on <instrument> timeframe <tf>`; the two
/// differ at the third word, which is what this reads. Reading the header and
/// not guessing from the first character is deliberate: a `{` inside a comment
/// must not change which language a document is in.
#[must_use]
pub fn looks_like_slv3(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .is_some_and(|l| {
            let mut words = l.split_whitespace();
            words.next() == Some("strategy") && words.nth(1) == Some("kind")
        })
}

fn slv3_refusal() -> Value {
    crate::tool_error::refuse_extra(
        "this_is_slv3",
        "this text opens with an SLv3 header (`strategy <name> kind …`); the tools that take SLv2 `text` do not read it.",
        "call `validate_strategy_v3`, `compile_strategy_v3` or `build_strategy_v3`, which take SLv3, or write the SLv2 header `strategy <name> on $bound_at_init timeframe 1h`",
        json!({ "see": "guide#grammar.header" }),
    )
}

fn diagnostics_as_errors(ds: &[strategy_lang::Diagnostic]) -> Vec<ValidationErrorItem> {
    ds.iter()
        .map(|d| ValidationErrorItem {
            code: d.code.as_str().to_string(),
            path: format!("{}:{}", d.span.line, d.span.col),
            message: d.message.clone(),
            fix: d.fix.clone(),
            see: d.see.clone(),
            example: None,
        })
        .collect()
}

/// What a caller handed over: SLv2 text, or a definition object/string.
enum Door {
    Text(String),
    Json(String),
}

fn door(params: &Value, definition_json: &str) -> Result<Door, Value> {
    // A strategy written as clause fills is assembled into SLv2 and goes through
    // the text door: one compiler (S-5.9).
    if let Some(form) = params.get("form").filter(|f| !f.is_null()) {
        if params.get("text").is_some() || !definition_json.is_empty() {
            return Err(crate::tool_error::refuse(
                "ambiguous_source",
                "`form` was sent together with `text` or `definition_json`.",
                "send exactly one of `form`, `text`, `definition_json`",
            ));
        }
        return match serde_json::from_value::<strategy_lang::FormStrategy>(form.clone()) {
            Ok(f) => Ok(Door::Text(f.to_text())),
            Err(e) => Err(crate::tool_error::refuse_extra(
                "unreadable_form",
                format!("`form` could not be read: {e}"),
                "fill header, entries, size and exits; the grammar is get_slv2_guide(section=\"form.schema\")",
                json!({ "see": "guide#form" }),
            )),
        };
    }
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (text, definition_json.is_empty()) {
        (Some(t), true) => {
            if looks_like_slv3(t) {
                Err(slv3_refusal())
            } else {
                Ok(Door::Text(t.to_string()))
            }
        }
        (None, false) => Ok(Door::Json(definition_json.to_string())),
        (None, true) => Err(crate::tool_error::missing_field_with(
            "definition_json",
            "send the definition object in `definition_json`, or SLv2 in `text`",
        )),
        (Some(_), false) => Err(crate::tool_error::refuse(
            "ambiguous_source",
            "both `text` and `definition_json` were sent; they are different languages and this tool will not guess.",
            "send `text` or `definition_json`, not both",
        )),
    }
}

/// The features a definition declares, the longest warm-up among them and the
/// dearest cost class — AGENT-011 §4's `validate_strategy` row.
fn facts(def: &StrategyDefinition) -> Value {
    let mut used: Vec<String> = Vec::new();
    for i in &def.inputs {
        for f in &i.features {
            if !used.contains(f) {
                used.push(f.clone());
            }
        }
    }
    let mut warmup: Option<u32> = None;
    let mut unresolved = false;
    let mut cost = features::CostClass::Cheap;
    for name in &used {
        match (
            features::runtime::lookback_bars(name),
            features::registry::resolve(name),
        ) {
            (Some(bars), Some(r)) => {
                warmup = Some(warmup.map_or(bars, |w| w.max(bars)));
                cost = cost.max(r.family.cost);
            }
            // A parameterised name (`ema_{{fast}}`) does not resolve until
            // parameters take values, and an unknown one never does. Neither is
            // a warm-up of zero, so the aggregate is withheld.
            _ => unresolved = true,
        }
    }
    json!({
        "features_used": used,
        "warmup_bars": if unresolved { Value::Null } else { json!(warmup) },
        "warmup_is_unknown_because": unresolved.then_some(
            "a declared feature name carries a `{{parameter}}` or is not in the registry, so its \
             lookback is not known until parameters take values — `explain_strategy` and \
             `preview_signals` report it for the materialised definition"
        ),
        "estimated_cost_class": cost.as_str(),
    })
}

/// `validate_strategy` — now with a text door.
///
/// Both doors end at `strategy_validator::validate`. Text is compiled first, and
/// a compile failure is reported as the same `errors` list (code, where, message,
/// fix, see) the validator uses, so a caller has one shape to parse.
pub fn validate_strategy(params: &Value, definition_json: &str) -> Value {
    match door(params, definition_json) {
        Err(e) => e,
        Ok(Door::Json(j)) => {
            let result = authoring::validate_strategy(&j);
            let mut out = serde_json::to_value(&result).unwrap_or_else(|_| json!({}));
            if result.valid {
                if let Ok(def) = serde_json::from_str::<StrategyDefinition>(&j) {
                    merge(&mut out, facts(&def));
                }
            }
            out
        }
        Ok(Door::Text(t)) => match strategy_lang::text_to_definition(&t) {
            Err(ds) => json!({ "valid": false, "errors": diagnostics_as_errors(&ds) }),
            Ok(def) => {
                let materialised =
                    domain::strategy_def::params::materialize(&def, &BTreeMap::new())
                        .unwrap_or_else(|_| def.clone());
                match strategy_validator::validate(&materialised) {
                    Ok(_) => {
                        let mut out = json!({
                            "valid": true,
                            "errors": [],
                            "definition": def,
                        });
                        // Facts are read off the materialised copy: a swept
                        // feature name resolves only once parameters have values.
                        merge(&mut out, facts(&materialised));
                        out
                    }
                    Err(es) => json!({
                        "valid": false,
                        "errors": es.into_iter().map(ValidationErrorItem::from).collect::<Vec<_>>(),
                    }),
                }
            }
        },
    }
}

fn merge(into: &mut Value, extra: Value) {
    if let (Some(a), Value::Object(b)) = (into.as_object_mut(), extra) {
        a.extend(b);
    }
}

/// `create_strategy` — now with a text door.
///
/// Reusing the slug is the lineage mechanism (ADR-D-21): the route derives
/// `parent_hash` from the head of the same `strategy_id`, and the response says
/// whether it found one. Text names its slug in its header, so iterating on a
/// strategy is writing the same header again.
pub async fn create_strategy(api: &ApiClient, params: &Value, definition_json: &str) -> Value {
    let parent = params.get("parent_strategy_id").and_then(Value::as_str);
    match door(params, definition_json) {
        Err(e) => e,
        Ok(Door::Json(j)) => authoring::create_strategy(api, &j, parent).await,
        Ok(Door::Text(t)) => {
            let def = match strategy_lang::text_to_definition(&t) {
                Ok(d) => d,
                Err(ds) => {
                    return crate::tool_error::refuse_extra(
                        "validation_failed",
                        format!("the SLv2 text has {} diagnostic(s); see `errors`.", ds.len()),
                        "apply the `fix` on each entry in `errors`, then call again",
                        json!({ "errors": diagnostics_as_errors(&ds) }),
                    )
                }
            };
            let materialised = domain::strategy_def::params::materialize(&def, &BTreeMap::new())
                .unwrap_or_else(|_| def.clone());
            if let Err(es) = strategy_validator::validate(&materialised) {
                let errors = es.into_iter().map(ValidationErrorItem::from).collect::<Vec<_>>();
                return crate::tool_error::refuse_extra(
                    "validation_failed",
                    format!("the definition has {} validation error(s); see `errors`.", errors.len()),
                    "fix each entry in `errors`, then call again",
                    json!({ "errors": errors }),
                );
            }
            let mut out = authoring::create_strategy_from_def(api, def, parent).await;
            if out.get("error").is_none() {
                let stored = store_comments(api, &t, out.get("revision_hash")).await;
                if let Some(o) = out.as_object_mut() {
                    o.insert(
                        "compiled_from".into(),
                        json!("slv2 text; the author's comments are kept beside the revision, not inside the definition"),
                    );
                    o.insert("comments".into(), stored);
                }
            }
            out
        }
    }
}

/// Keep the author's comments beside the revision just created (S-5.15).
///
/// They go to `strategy_revision_notes`, keyed by the revision hash of the
/// canonical definition — which has no comment in it, so storing, rewording or
/// dropping a note cannot change a revision's identity or count as a trial
/// (ADR-S5-07 refused a field on the definition for exactly that reason; this
/// is the alternative that does not touch the hash). A failure to store them
/// does not fail the save: the strategy exists, and the answer says the notes
/// did not.
async fn store_comments(api: &ApiClient, text: &str, revision: Option<&Value>) -> Value {
    let notes = match strategy_lang::intent::notes_of_text(text) {
        Ok(n) => n,
        Err(_) => return json!({ "stored": 0 }),
    };
    if notes.is_empty() {
        return json!({ "stored": 0 });
    }
    let Some(hash) = revision.and_then(Value::as_str) else {
        return json!({ "stored": 0, "not_stored_because": "the save returned no revision hash to key them by" });
    };
    let body = json!({
        "revision_hash": hash,
        "notes": notes.iter().map(|n| json!({ "clause": n.clause, "lines": n.lines })).collect::<Vec<_>>(),
    });
    match api.post("/api/strategy-notes", body).await {
        Ok(r) => json!({ "stored": r.get("stored").cloned().unwrap_or(json!(notes.len())) }),
        Err(e) => json!({ "stored": 0, "not_stored_because": e.to_tool_error() }),
    }
}

/// The notes stored beside a revision, as `explain_strategy` shows them.
async fn stored_comments(api: &ApiClient, def: &StrategyDefinition) -> Option<Vec<Value>> {
    let hash = domain::strategy_def::revision::RevisionHash::of(def).ok()?;
    let r = api
        .get(&format!("/api/strategy-notes/{}", hash.as_str()))
        .await
        .ok()?;
    let notes = r.get("notes")?.as_array()?;
    if notes.is_empty() {
        return None;
    }
    Some(
        notes
            .iter()
            .map(|n| json!({ "clause": n["clause"], "says": n["lines"] }))
            .collect(),
    )
}

/// `explain_strategy`, reading the notes stored with the revision when the
/// caller passes a stored definition rather than the text (S-5.15).
pub async fn explain_strategy_with_stored_notes(api: &ApiClient, params: &Value) -> Value {
    let mut out = explain_strategy(params);
    if out.get("error").is_some() || params.get("text").is_some() {
        return out;
    }
    let Ok(def) = definition_of(params) else {
        return out;
    };
    if let Some(comments) = stored_comments(api, &def).await {
        if let Some(d) = out.get_mut("detail").and_then(Value::as_object_mut) {
            d.insert("comments".into(), Value::Array(comments));
            d.remove("comments_are_not_stored");
            d.insert(
                "comments_from".into(),
                json!("the notes stored beside this revision when its author saved the text"),
            );
        }
    }
    out
}

// ── explain, with the author's intent ───────────────────────────────────────

/// `explain_strategy`, plus the comments the author wrote (S-5.15).
///
/// The base answer is `strategy.rs`'s, unchanged. What this adds is the one
/// thing it cannot know: the comments. They are on the syntax tree, not in the
/// stored definition (the domain has no field for them and a good reason not to
/// grow one — see `strategy_lang::intent`), so they are shown when the caller
/// hands over the **text** and are reported absent, by name, when it hands over a
/// stored definition. A reader who sees no comments on a stored strategy is told
/// why, not left to think nobody wrote any.
#[must_use]
pub fn explain_strategy(params: &Value) -> Value {
    let mut out = crate::tools::strategy::explain_strategy(params);
    if out.get("error").is_some() {
        return out;
    }
    warm_up_at_defaults(params, &mut out);
    let comments = match params.get("text").and_then(Value::as_str) {
        Some(text) => match strategy_lang::intent::notes_of_text(text) {
            Ok(notes) => json!({
                "comments": notes.iter().map(|n| json!({
                    "clause": n.clause,
                    "says": n.lines,
                })).collect::<Vec<_>>(),
            }),
            // The base answer compiled the text, so this cannot happen; if it
            // does, say nothing rather than something wrong.
            Err(_) => return out,
        },
        None => json!({
            "comments": [],
            "comments_are_not_stored": "no notes are stored beside this revision, and the canonical form has no field for them (they would change its hash): either its author wrote none, or it was saved as JSON. Pass the original text as `text` to see any it carries.",
        }),
    };
    if let Some(detail) = out.get_mut("detail").and_then(Value::as_object_mut) {
        if let Value::Object(extra) = comments {
            detail.extend(extra);
        }
    }
    out
}

/// A swept strategy's warm-up, at its parameter defaults.
///
/// `explain_strategy` reads the features a definition declares, and a swept one
/// declares `ema_{{fast}}`, which the engine cannot resolve until `fast` has a
/// value — so for every strategy a sweep can vary (every skeleton) the base answer
/// withholds the warm-up. AGENT-011 §4 asks it to name the warm-up bars, so this
/// fills it in from the materialised copy and says that it did: the number is the
/// warm-up *at the defaults*, and a sweep that raises `slow` raises it.
fn warm_up_at_defaults(params: &Value, out: &mut Value) {
    let withheld = out["detail"]["warmup_bars"].is_null()
        && out["detail"]["warmup_is_unknown_because"].is_object();
    if !withheld {
        return;
    }
    let Ok(def) = definition_of(params) else {
        return;
    };
    if def.parameters.is_empty() {
        return;
    }
    let Ok(materialised) = domain::strategy_def::params::materialize(&def, &BTreeMap::new())
    else {
        return;
    };
    let facts = facts(&materialised);
    if facts["warmup_bars"].is_null() {
        return;
    }
    let inputs: Vec<Value> = facts["features_used"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .map(|name| json!({
            "feature": name,
            "warmup_bars": features::runtime::lookback_bars(name),
        }))
        .collect();
    if let Some(d) = out.get_mut("detail").and_then(Value::as_object_mut) {
        d.insert("inputs".into(), json!(inputs));
        d.insert("warmup_bars".into(), facts["warmup_bars"].clone());
        d.insert("warmup_is_unknown_because".into(), Value::Null);
        d.insert(
            "warmup_evaluated_at".into(),
            json!("the parameter defaults; a sweep that raises a lookback raises this"),
        );
    }
}

// ── preview, for a strategy a sweep can vary ────────────────────────────────

/// `params` with a v1 draft's parameters at their defaults.
///
/// The preview route (and `backtest::preview`, and `derive_requirements` under
/// it) read feature names as written, so a swept strategy — `ema_{{fast}}`, which
/// is every skeleton and every strategy the compiler produces from an
/// `input x = ema(close, fast)` — reaches the engine unresolved and is refused as
/// an unknown feature. A backtest materialises first (`sim_executor`); the preview
/// did not, so the tool a model uses to check a first draft failed on the first
/// draft the platform itself had handed it. A v3 graph, or anything that is not a
/// v1 definition with parameters, passes through untouched.
#[must_use]
pub fn with_materialised_draft(params: &Value) -> Value {
    let Some(raw) = params.get("definition") else {
        return params.clone();
    };
    let Ok(def) = serde_json::from_value::<StrategyDefinition>(raw.clone()) else {
        return params.clone();
    };
    if def.parameters.is_empty() {
        return params.clone();
    }
    let Ok(materialised) = materialised_json(&def) else {
        // Defaults that do not materialise are the caller's to hear about from
        // the platform, in the platform's words.
        return params.clone();
    };
    let mut out = params.clone();
    out["definition"] = materialised;
    out
}

/// `preview_signals`, after [`with_materialised_draft`] — and, when the draft
/// never fires, naming the clause that stopped it (S-5.15).
///
/// `never_fires_because` says that something is wrong, not what, so the next move
/// is a guess that costs a round trip. When the preview finds no entry and every
/// feature has a value (`never_fires`, as opposed to a warm-up problem), this asks
/// the platform for the blocking profile over the same window and attaches it as
/// `blocking_profile`. It costs extra previews only in the case that needs them,
/// spends no trial, and a profile that cannot be measured is reported as
/// `blocking_profile_unavailable` beside the unchanged preview rather than
/// failing it.
pub async fn preview_signals(api: &ApiClient, params: &Value) -> Value {
    let mut out =
        crate::tools::strategy::preview_signals(api, &with_materialised_draft(params)).await;
    let never_fires = out.get("error").is_none()
        && out["entries"] == 0
        && out["never_fires_code"] == "never_fires";
    if !never_fires {
        return out;
    }
    let Some(def) = params
        .get("definition")
        .and_then(|d| serde_json::from_value::<StrategyDefinition>(d.clone()).ok())
    else {
        return out;
    };
    let attached = match profile_over_api(api, params, &def).await {
        Ok(profile) => ("blocking_profile", profile_json(&profile)),
        Err(e) => ("blocking_profile_unavailable", e),
    };
    out[attached.0] = attached.1;
    out
}

// ── the clause grammar, as a tool ───────────────────────────────────────────

/// `compile_strategy_form` — a strategy written as clause fills.
///
/// The arguments **are** the form (`strategy_lang::form::FormStrategy`), and the
/// tool declares the form's JSON Schema as its input schema. That is the whole of
/// how a `select_fill` tier reaches the grammar: such a tier decodes a tool's
/// arguments under the tool's own schema, so a strategy it writes through this
/// tool cannot name a feature the engine does not serve, pass the wrong number of
/// parameters or leave a range. Before this the form had no caller outside its own
/// tests — a grammar for a tier nothing could hand it to.
///
/// The fills are assembled into SLv2 and compiled through the same parser and
/// compiler as free text; there is no second compiler.
#[must_use]
pub fn compile_strategy_form(params: &Value) -> Value {
    let form: strategy_lang::FormStrategy = match serde_json::from_value(params.clone()) {
        Ok(f) => f,
        Err(e) => {
            return crate::tool_error::refuse_extra(
                "unreadable_form",
                format!("the form could not be read: {e}"),
                "fill the clauses the schema names: header, entries, size and exits are required; params, constraints, inputs and risk are optional",
                json!({ "see": "guide#form" }),
            )
        }
    };
    let text = form.to_text();
    let fills = form.fills();
    match form.compile() {
        Ok(def) => json!({
            "summary_fields": { "strategy_id": def.strategy_id, "n_fills": fills },
            "detail": {
                "definition": serde_json::to_value(&def).unwrap_or(Value::Null),
                "text": text,
                "fills": fills,
                "note": "nothing was persisted. Pass `definition` to `create_strategy`, or the text to `validate_strategy`, to save it.",
            },
            "ids": {},
        }),
        Err(ds) => crate::tool_error::refuse_extra(
            "compile_failed",
            format!("the form compiles to SLv2 with {} diagnostic(s); see `diagnostics`.", ds.len()),
            "apply the `fix` on each entry in `diagnostics`, then call again",
            json!({
                "fills": fills,
                "text": text,
                "n_diagnostics": ds.len(),
                "diagnostics": ds.iter().map(|d| json!({
                    "code": d.code.as_str(),
                    "line": d.span.line,
                    "col": d.span.col,
                    "message": d.message,
                    "fix": d.fix,
                    "see": d.see,
                })).collect::<Vec<_>>(),
            }),
        ),
    }
}

// ── skeletons and the guide ──────────────────────────────────────────────────

/// `strategy_skeleton` — a correct first version per hypothesis family.
#[must_use]
pub fn strategy_skeleton(params: &Value) -> Value {
    let Some(name) = params
        .get("hypothesis_family")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return crate::tool_error::refuse_extra(
            "missing_field",
            "`hypothesis_family` is required and was not given.",
            "add `hypothesis_family`: one of `known`",
            json!({
                "field": "hypothesis_family",
                "known": strategy_lang::HypothesisFamily::ALL.map(strategy_lang::HypothesisFamily::as_str),
            }),
        );
    };
    let Some(family) = strategy_lang::HypothesisFamily::parse(name) else {
        return crate::tool_error::refuse_extra(
            "unknown_family",
            format!("hypothesis family `{name}` is not one this tool knows."),
            "set `hypothesis_family` to one of `known`",
            json!({
                "hypothesis_family": name,
                "known": strategy_lang::HypothesisFamily::ALL.map(strategy_lang::HypothesisFamily::as_str),
            }),
        );
    };
    match strategy_lang::skeleton(family) {
        Err(d) => crate::tool_error::refuse_extra(
            "not_writable_yet",
            d.message.as_str(),
            d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX),
            json!({ "hypothesis_family": name, "see": d.see }),
        ),
        Ok(s) => {
            let params_json = s
                .params()
                .ok()
                .and_then(|p| serde_json::to_value(p).ok())
                .unwrap_or(Value::Null);
            json!({
                "summary_fields": { "family": family.as_str(), "n_params": params_json.as_object().map_or(0, serde_json::Map::len) },
                "detail": {
                    "family": family.as_str(),
                    "text": s.text,
                    "params": params_json,
                    "notes": s.notes,
                    "note": "this compiles as written. `preview_signals` it on a window before \
                             spending a trial, and keep the same strategy_id when you save edits.",
                },
                "ids": {},
            })
        }
    }
}

/// `get_slv2_guide` — the SLv2 reference, one section at a time.
///
/// A section is at most 4 KB so a small model fetches the one a diagnostic's
/// `see` names and not the whole language. With no `section` it lists them.
#[must_use]
pub fn get_slv2_guide(params: &Value) -> Value {
    let wanted = params
        .get("section")
        .and_then(Value::as_str)
        .map(|s| s.trim().trim_start_matches("guide#"))
        .filter(|s| !s.is_empty());
    if wanted == Some("form.schema") {
        return json!({
            "section": "form.schema",
            "schema": strategy_lang::form::document_schema_compact(),
            "note": "the grammar compile_strategy_form reads; every feature is one of the engine's families",
        });
    }
    let all = strategy_lang::guide::sections();
    match wanted {
        None => json!({
            "sections": all.iter().map(|s| json!({
                "section": s.anchor,
                "title": s.title,
                "bytes": s.body.len(),
            })).collect::<Vec<_>>(),
            "note": "pass `section` to read one. A diagnostic's `see` field names the section \
                     that explains it, with or without the `guide#` prefix.",
        }),
        Some(w) => match all.iter().find(|s| s.answers_to(w)) {
            Some(s) => json!({ "section": s.anchor, "title": s.title, "text": s.text() }),
            None => crate::tool_error::refuse_extra(
                "unknown_section",
                format!("no guide section answers to `{w}`."),
                "set `section` to one of `known`, or omit it to list them",
                json!({
                    "section": w,
                    "known": all.iter().map(|s| s.anchor.clone()).collect::<Vec<_>>(),
                }),
            ),
        },
    }
}

// ── measuring a strategy against bars ────────────────────────────────────────

/// What one probe preview said.
#[derive(Clone, Debug)]
pub struct Answer {
    pub bars_evaluated: u64,
    /// `signal` → bars on which it was true.
    pub bars_true: BTreeMap<String, u64>,
}

/// Read the parts of a preview response a probe needs.
///
/// # Errors
///
/// A tool error when the response lacks `bars_evaluated` — a shape this client
/// does not know is refused rather than read as zero.
pub fn read_answer(preview: &Value) -> Result<Answer, Value> {
    let Some(bars_evaluated) = preview.get("bars_evaluated").and_then(Value::as_u64) else {
        return Err(crate::tool_error::refuse(
            "unreadable_preview",
            "the preview response has no `bars_evaluated`, so it cannot be read as a count.",
            "call preview_signals directly and check it answers; if it does, report the shape mismatch",
        ));
    };
    let mut bars_true = BTreeMap::new();
    for s in preview
        .get("signals")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        if let (Some(name), Some(n)) = (
            s.get("signal").and_then(Value::as_str),
            s.get("bars_true").and_then(Value::as_u64),
        ) {
            bars_true.insert(name.to_string(), n);
        }
    }
    Ok(Answer {
        bars_evaluated,
        bars_true,
    })
}

/// Run every probe of every entry through `run` and fold the answers into the
/// blocking profile.
///
/// `run` receives the probe's definition, **not yet materialised**: parameters
/// still stand in the feature names, and the preview does not resolve them. The
/// caller that posts it materialises first (see [`materialised_json`]).
///
/// # Errors
///
/// A tool error for a definition SLv2 has no text for, for more probes than
/// [`MAX_PREVIEWS`], for an answer a window cannot have produced, or whatever
/// `run` returned.
pub async fn measure<F, Fut>(
    def: &StrategyDefinition,
    mut run: F,
) -> Result<strategy_lang::BlockingProfile, Value>
where
    F: FnMut(StrategyDefinition) -> Fut,
    Fut: Future<Output = Result<Answer, Value>>,
{
    let sets = strategy_lang::probes(def).map_err(|d| {
        crate::tool_error::refuse(
            "not_expressible_in_slv2",
            d.message.as_str(),
            d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX),
        )
    })?;
    let total: usize = sets.iter().map(|s| s.probes().len()).sum();
    if sets.is_empty() {
        return Err(crate::tool_error::refuse(
            "no_entry",
            "this strategy has no `entry` clause, so there is no condition to profile.",
            "add an `entry` clause to the strategy, then call again",
        ));
    }
    if total > MAX_PREVIEWS {
        return Err(crate::tool_error::refuse(
            "too_many_probes",
            format!("measuring this strategy would take {total} previews; the limit is {MAX_PREVIEWS}"),
            "split the entry conditions, or profile the version with fewer clauses",
        ));
    }

    let mut tallies = Vec::with_capacity(sets.len());
    for set in &sets {
        let mut answers: Vec<u64> = Vec::with_capacity(set.probes().len());
        let mut evaluated: Option<u64> = None;
        for probe in set.probes() {
            let a = run(probe.definition.clone()).await?;
            // Every probe declares the same inputs, so every probe has the same
            // warm-up. A preview that disagrees is not measuring this strategy.
            if *evaluated.get_or_insert(a.bars_evaluated) != a.bars_evaluated {
                return Err(crate::tool_error::refuse(
                    "probes_disagree",
                    "two previews of the same strategy evaluated different numbers of bars, so their counts cannot be subtracted",
                    "call again; if it repeats, the data is changing under the run, so pin the window with explicit start and end",
                ));
            }
            answers.push(a.bars_true.get(&probe.signal).copied().unwrap_or(0));
        }
        let tally = set
            .counts(evaluated.unwrap_or(0), &answers)
            .map_err(|d| crate::tool_error::from_diagnostic(d.code.as_str(), &d.message, d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX)))?;
        tallies.push(tally);
    }
    strategy_lang::blocking_profile(def, &tallies)
        .map_err(|d| crate::tool_error::from_diagnostic(d.code.as_str(), &d.message, d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX)))
}

/// A probe's definition as the JSON the preview route takes: parameters at
/// their defaults, because the route reads feature names as written.
///
/// # Errors
///
/// A tool error when the defaults do not materialise.
pub fn materialised_json(def: &StrategyDefinition) -> Result<Value, Value> {
    let m = domain::strategy_def::params::materialize(def, &BTreeMap::new())
        .map_err(|e| {
            crate::tool_error::refuse(
                "params_do_not_materialise",
                format!("the parameter defaults do not materialise: {e}"),
                "give every parameter a default inside its declared range",
            )
        })?;
    serde_json::to_value(m).map_err(|e| {
        crate::tool_error::refuse(
            "serialization_error",
            format!("the materialised definition could not be serialised: {e}"),
            "call again; if it repeats, report the strategy that triggers it",
        )
    })
}

/// The window and project every probe preview is asked about.
struct Window<'a> {
    params: &'a Value,
}

impl Window<'_> {
    fn body(&self, definition: Value) -> Result<Value, Value> {
        let mut body = json!({ "definition": definition });
        for field in ["instrument_id", "start", "end"] {
            let Some(v) = self.params.get(field).filter(|v| !v.is_null()) else {
                return Err(crate::tool_error::missing_field(field));
            };
            body[field] = v.clone();
        }
        for field in ["timeframe", "project_id"] {
            if let Some(v) = self.params.get(field).filter(|v| !v.is_null()) {
                body[field] = v.clone();
            }
        }
        Ok(body)
    }
}

async fn profile_over_api(
    api: &ApiClient,
    params: &Value,
    def: &StrategyDefinition,
) -> Result<strategy_lang::BlockingProfile, Value> {
    let window = Window { params };
    // Fail on a missing field before the first request, not at the fourth.
    window.body(Value::Null)?;
    measure(def, |probe| {
        let window = Window { params };
        async move {
            let body = window.body(materialised_json(&probe)?)?;
            match api.post("/api/agent/strategy/preview", body).await {
                Ok(v) => read_answer(&v),
                Err(e) => Err(e.to_tool_error()),
            }
        }
    })
    .await
}

fn definition_of(params: &Value) -> Result<StrategyDefinition, Value> {
    let text = params
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let definition = params.get("definition").filter(|v| !v.is_null());
    match (text, definition) {
        (Some(t), None) => {
            if looks_like_slv3(t) {
                return Err(slv3_refusal());
            }
            strategy_lang::text_to_definition(t).map_err(|ds| {
                crate::tool_error::refuse_extra(
                    "compile_failed",
                    format!("the SLv2 text has {} diagnostic(s); see `errors`.", ds.len()),
                    "apply the `fix` on each entry in `errors`, then call again",
                    json!({ "errors": diagnostics_as_errors(&ds) }),
                )
            })
        }
        (None, Some(d)) => serde_json::from_value(d.clone()).map_err(|e| {
            crate::tool_error::refuse(
                "invalid_definition",
                format!("`definition` is not a strategy definition: {e}"),
                "send the object get_strategy returned, or send SLv2 in `text` instead",
            )
        }),
        (None, None) => Err(crate::tool_error::missing_field_with(
            "text",
            "send SLv2 in `text`, or a stored definition object in `definition`",
        )),
        (Some(_), Some(_)) => Err(crate::tool_error::refuse(
            "ambiguous_source",
            "both `text` and `definition` were sent; they are different languages.",
            "send `text` or `definition`, not both",
        )),
    }
}

fn verdict_json(v: &strategy_lang::Verdict, entry: &strategy_lang::block::EntryProfile) -> Value {
    use strategy_lang::Verdict;
    match v {
        Verdict::NothingEvaluated => json!({ "kind": "nothing_evaluated" }),
        Verdict::Fires { share } => json!({ "kind": "fires", "fire_share": share }),
        Verdict::Blocked {
            clause,
            sole_blocker_share,
        } => json!({
            "kind": "blocked",
            "clause": entry.clauses[*clause].path,
            "clause_text": entry.clauses[*clause].text,
            "sole_blocker_share": sole_blocker_share,
        }),
        Verdict::NoSingleBlocker => json!({ "kind": "no_single_blocker" }),
    }
}

/// The profile as the tool returns it.
#[must_use]
pub fn profile_json(profile: &strategy_lang::BlockingProfile) -> Value {
    json!({
        "entries": profile.entries.iter().map(|e| json!({
            "path": e.path,
            "side": e.side.as_str(),
            "bars_evaluated": e.bars_evaluated,
            "bars_all_true": e.bars_all_true,
            "fire_share": e.fire_share,
            "verdict": verdict_json(&e.verdict, e),
            "says": e.words(),
            "clauses": e.clauses.iter().map(|c| json!({
                "path": c.path,
                "reads": c.text,
                "bars_true": c.bars_true,
                "true_share": c.true_share,
                "sole_blocker_share": c.sole_blocker_share,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "never_fires": profile.never_fires(),
        "reads_as": profile.render(),
    })
}

/// `blocking_profile` — which clause stopped the entry.
pub async fn blocking_profile(api: &ApiClient, params: &Value) -> Value {
    let def = match definition_of(params) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let profile = match profile_over_api(api, params, &def).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    json!({
        "summary_fields": {
            "n_entries": profile.entries.len(),
            "never_fires": profile.never_fires(),
        },
        "detail": {
            "profile": profile_json(&profile),
            "note": "measured by previewing this strategy with one clause swapped in at a time — \
                     the preview's own evaluator, over the same window, and no trial. \
                     `sole_blocker_share` is the share of the bars the entry failed on where \
                     that clause was the only false one.",
        },
        "ids": {},
    })
}

// ── suggest_moves ────────────────────────────────────────────────────────────

fn evidence_json(e: &strategy_lang::Evidence) -> Value {
    use strategy_lang::Evidence;
    let (kind, measured) = match e {
        Evidence::SoleBlocker {
            entry,
            clause,
            true_share,
            sole_blocker_share,
            bars_evaluated,
        } => (
            "sole_blocker",
            json!({
                "entry": entry, "clause": clause, "true_share": true_share,
                "sole_blocker_share": sole_blocker_share, "bars_evaluated": bars_evaluated,
            }),
        ),
        Evidence::NoSingleBlocker {
            entry,
            bars_evaluated,
            rarest_clause,
            rarest_true_share,
        } => (
            "no_single_blocker",
            json!({
                "entry": entry, "bars_evaluated": bars_evaluated,
                "rarest_clause": rarest_clause, "rarest_true_share": rarest_true_share,
            }),
        ),
        Evidence::FiresTooOften {
            entry,
            fire_share,
            bars_evaluated,
        } => (
            "fires_too_often",
            json!({ "entry": entry, "fire_share": fire_share, "bars_evaluated": bars_evaluated }),
        ),
        Evidence::LosesInRegime {
            label,
            objective,
            share_of_window,
        } => (
            "loses_in_regime",
            json!({ "label": label, "objective": objective, "share_of_window": share_of_window }),
        ),
        Evidence::ExitDominates {
            reason,
            share,
            trades,
        } => (
            "exit_dominates",
            json!({ "reason": reason, "share": share, "trades": trades }),
        ),
    };
    json!({ "diagnostic": kind, "measured": measured, "says": e.words() })
}

fn proposal_json(p: &strategy_lang::suggest::Proposal) -> Value {
    use strategy_lang::suggest::Proposal;
    match p {
        Proposal::MoveThreshold {
            clause,
            literal,
            direction,
        } => json!({
            "edit": "move_threshold", "clause": clause.as_str(),
            "literal": literal.as_str(), "direction": direction.as_str(),
        }),
        Proposal::WidenParamRange { param, direction } => json!({
            "edit": "widen_param_range", "param": param.as_str(), "direction": direction.as_str(),
        }),
        Proposal::DropClause { clause } => json!({ "edit": "drop_clause", "clause": clause.as_str() }),
        Proposal::AddPredicate { predicate } => json!({
            "edit": "add_predicate", "predicate": predicate.text().as_str(),
        }),
    }
}

/// Caller-supplied findings the platform has no producer for.
///
/// `exit_reasons` — how often each exit rule closed a trade — and `regimes` —
/// how the strategy did inside a labelled regime — come from a backtest's
/// diagnostics, and no endpoint here yet yields them in a typed form. Rather than
/// leave the rules that read them unreachable, the caller may pass them; a regime
/// gate is a `Predicate` and is checked against the registry at construction.
fn caller_findings(params: &Value) -> Result<strategy_lang::Findings, Value> {
    let mut f = strategy_lang::Findings::default();
    for r in params
        .get("exit_reasons")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let (Some(reason), Some(share), Some(trades)) = (
            r.get("reason").and_then(Value::as_str),
            r.get("share").and_then(Value::as_f64),
            r.get("trades").and_then(Value::as_u64),
        ) else {
            return Err(crate::tool_error::refuse(
                "unreadable_exit_reasons",
                "an exit_reasons entry is missing `reason`, `share` or `trades`.",
                "send each as {reason: \"stop_loss\"|\"take_profit\"|\"max_holding_bars\"|…, share: 0..1, trades: n}",
            ));
        };
        f.exit_reasons.push(strategy_lang::suggest::ExitReasonShare {
            reason: reason.to_string(),
            share,
            trades,
        });
    }
    for r in params
        .get("regimes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let (Some(label), Some(objective), Some(share)) = (
            r.get("label").and_then(Value::as_str),
            r.get("objective").and_then(Value::as_f64),
            r.get("share_of_window").and_then(Value::as_f64),
        ) else {
            return Err(crate::tool_error::refuse(
                "unreadable_regimes",
                "a regimes entry is missing `label`, `objective` or `share_of_window`.",
                "send each as {label, objective, share_of_window, gate?: {family, params: [n…], op: \"<\"|\"<=\"|\">\"|\">=\", bound}}",
            ));
        };
        let gate = match r.get("gate").filter(|g| !g.is_null()) {
            None => None,
            Some(g) => {
                use strategy_lang::suggest::{Cmp, Predicate};
                let op = match g.get("op").and_then(Value::as_str) {
                    Some("<") => Cmp::Lt,
                    Some("<=") => Cmp::Le,
                    Some(">") => Cmp::Gt,
                    Some(">=") => Cmp::Ge,
                    _ => {
                        return Err(crate::tool_error::refuse(
                            "unreadable_regimes",
                            "a regime gate has a missing or unknown `op`.",
                            "set `gate.op` to one of `<`, `<=`, `>`, `>=`",
                        ))
                    }
                };
                let nums: Option<Vec<u32>> = g
                    .get("params")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok()))
                            .collect()
                    })
                    .unwrap_or(Some(Vec::new()));
                let (Some(family), Some(nums), Some(bound)) = (
                    g.get("family").and_then(Value::as_str),
                    nums,
                    g.get("bound").map(|b| b.as_str().map_or_else(|| b.to_string(), str::to_string)),
                ) else {
                    return Err(crate::tool_error::refuse(
                        "unreadable_regimes",
                        "a regime gate is missing `family`, whole-number `params` or `bound`.",
                        "give the gate a `family`, whole-number `params` and a `bound`",
                    ));
                };
                Some(
                    Predicate::new(
                        family,
                        nums,
                        op,
                        &bound,
                        &strategy_lang::Catalogue::engine(),
                    )
                    .map_err(|d| crate::tool_error::from_diagnostic(d.code.as_str(), &d.message, d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX)))?,
                )
            }
        };
        f.regimes.push(strategy_lang::suggest::RegimeOutcome {
            label: label.to_string(),
            objective,
            share_of_window: share,
            gate,
        });
    }
    Ok(f)
}

/// Turn findings into the tool's answer. Separate from the I/O so it is tested
/// without a server.
#[must_use]
pub fn suggestions_json(
    def: &StrategyDefinition,
    findings: &strategy_lang::Findings,
) -> Value {
    match strategy_lang::suggest_moves(def, findings) {
        Err(d) => crate::tool_error::refuse(
            "not_expressible_in_slv2",
            d.message.as_str(),
            d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX),
        ),
        Ok(s) => json!({
            "summary_fields": { "n_moves": s.moves.len(), "n_withheld": s.withheld.len() },
            "detail": {
                "moves": s.moves.iter().map(|m| json!({
                    "kind": m.kind.as_str(),
                    "path": m.path,
                    "proposal": proposal_json(&m.proposal),
                    // Not optional and not prose: the measurement the rule fired on.
                    "why": evidence_json(&m.why),
                    "expected_effect": m.expected_effect,
                })).collect::<Vec<_>>(),
                "withheld": s.withheld.iter().map(|w| json!({
                    "kind": w.kind.as_str(),
                    "why": evidence_json(&w.why),
                    "because": w.because,
                    "code": w.code.map(|c| c.as_str()),
                })).collect::<Vec<_>>(),
                "reads_as": s.render(),
                "note": "motivated moves only: each cites the measurement it was read from, and \
                         none of them has been applied. Apply one with `apply_move`, then \
                         `preview_signals` the result.",
            },
            "ids": {},
        }),
    }
}

/// `suggest_moves` — the next structural moves, each citing its measurement.
///
/// The blocking profile is measured here, over the window the caller names.
/// `exit_reasons` and `regimes` are optional and caller-supplied: nothing in the
/// platform yet produces them as typed values (see [`caller_findings`]), and the
/// rules that read them stay reachable instead of waiting for a producer.
pub async fn suggest_moves(api: &ApiClient, params: &Value) -> Value {
    let def = match definition_of(params) {
        Ok(d) => d,
        Err(e) => return e,
    };
    let mut findings = match caller_findings(params) {
        Ok(f) => f,
        Err(e) => return e,
    };
    // The exit distribution of a recorded run, when the caller names one and has
    // not supplied its own (S-5.11): the simulator now writes it into the stored
    // result, so there is a producer and nobody has to transcribe it.
    let mut exit_reasons_from = Value::Null;
    if findings.exit_reasons.is_empty() {
        if let Some(run_id) = params.get("run_id").and_then(Value::as_str) {
            match api.get(&format!("/api/backtests/{run_id}")).await {
                Err(e) => return e.to_tool_error(),
                Ok(snap) => {
                    let rows = snap
                        .pointer("/result/exit_reasons")
                        .or_else(|| snap.get("exit_reasons"))
                        .cloned();
                    match rows.filter(|r| r.as_array().is_some_and(|a| !a.is_empty())) {
                        Some(rows) => match caller_findings(&json!({ "exit_reasons": rows })) {
                            Ok(f) => {
                                findings.exit_reasons = f.exit_reasons;
                                exit_reasons_from = json!({ "run_id": run_id, "read": "the run's stored result" });
                            }
                            Err(e) => return e,
                        },
                        None => {
                            exit_reasons_from = json!({
                                "run_id": run_id,
                                "read": "none",
                                "because": "this run's stored result carries no `exit_reasons`: it declared no exits, closed no trades, or predates the field. The rules that read it did not run.",
                            });
                        }
                    }
                }
            }
        }
    }
    let profile = match profile_over_api(api, params, &def).await {
        Ok(p) => p,
        Err(e) => return e,
    };
    let profile_json = profile_json(&profile);
    findings.blocking = Some(profile);
    let mut out = suggestions_json(&def, &findings);
    if let Some(d) = out.get_mut("detail").and_then(Value::as_object_mut) {
        d.insert("blocking_profile".into(), profile_json);
        if !exit_reasons_from.is_null() {
            d.insert("exit_reasons_from".into(), exit_reasons_from);
        }
    }
    out
}

// ── declarations ─────────────────────────────────────────────────────────────

fn window_properties() -> Value {
    json!({
        "instrument_id": {"type": "string"},
        "timeframe": {"type": "string", "default": "1m"},
        "start": {"type": "string", "description": "RFC3339"},
        "end": {
            "type": "string",
            "description": "RFC3339; may not reach the sealed holdout"
        },
        "project_id": {"type": "string"}
    })
}

#[must_use]
pub fn specs() -> Vec<ToolSpec> {
    let mut blocking_props = window_properties();
    if let Some(o) = blocking_props.as_object_mut() {
        o.insert("text".into(), json!({"type": "string", "description": "SLv2 — exactly one of this or `definition`"}));
        o.insert("definition".into(), json!({"type": "object", "description": "a stored definition object"}));
    }
    let mut suggest_props = blocking_props.clone();
    if let Some(o) = suggest_props.as_object_mut() {
        o.insert("exit_reasons".into(), json!({
            "type": "array",
            "description": "optional; else read from run_id",
            "items": {"type": "object", "properties": {
                "reason": {"type": "string"}, "share": {"type": "number"}, "trades": {"type": "integer"}
            }}
        }));
        o.insert("run_id".into(), json!({"type": "string"}));
        o.insert("regimes".into(), json!({
            "type": "array",
            "description": "optional: regimes lost in",
            "items": {"type": "object", "properties": {
                "label": {"type": "string"}, "objective": {"type": "number"},
                "share_of_window": {"type": "number"}, "gate": {"type": "object"}
            }}
        }));
    }
    crate::result_shape::declared(vec![
        ToolSpec::new(
            "strategy_skeleton",
            "core",
            Risk::Read,
            "A correct, compiling starting point for a hypothesis family — trend, mean_reversion, \
             breakout or vol_target — with its parameters and what it cannot do. carry and \
             model_signal are refused by name, with the feature families the engine lacks.",
            json!({
                "type": "object",
                "required": ["hypothesis_family"],
                "properties": {
                    "hypothesis_family": {
                        "type": "string",
                        "enum": ["trend", "mean_reversion", "breakout", "carry", "vol_target", "model_signal"]
                    }
                }
            }),
        )
        .internal()
        .summary(
            "{family} skeleton with {n_params} parameters",
            &[("family", ""), ("n_params", "")],
        ),
        ToolSpec::new(
            "get_slv2_guide",
            "core",
            Risk::Read,
            "The SLv2 reference, one section of at most 4 KB at a time. With no `section` it lists \
             them. A diagnostic's `see` field (such as guide#exits.units) names the section that \
             explains it.",
            json!({
                "type": "object",
                "properties": {
                    "section": {"type": "string", "description": "an anchor such as exits.units, with or without guide#"}
                }
            }),
        )
        .internal(),
        ToolSpec::new(
            "suggest_moves",
            "analysis",
            Risk::Read,
            "The next structural moves for a strategy, each citing the measurement that motivated \
             it (a blocking profile over your window). Motivated moves only; none is applied: \
             use apply_move. Spends no trial.",
            json!({
                "type": "object",
                "required": ["instrument_id", "start", "end"],
                "properties": suggest_props,
            }),
        )
        .internal()
        .spends_trial(crate::spec::TrialKind::None)
        .max_calls_per_run(20)
        .summary(
            "{n_moves} moves, {n_withheld} withheld",
            &[("n_moves", ""), ("n_withheld", "")],
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(text: &str) -> StrategyDefinition {
        strategy_lang::text_to_definition(text).unwrap_or_else(|e| panic!("{e:#?}"))
    }

    const SIGN_ERROR: &str = "\
strategy sign_error on $bound_at_init timeframe 1h
entry long when rel_volume(close, 20) < 50 and rsi(close, 14) < 0
size fixed 0.01
exit time 24 bars
";

    /// A scripted evaluator: the first clause true on 900 of 1000 bars, the
    /// second never. It answers each probe the way the preview would for that
    /// strategy, which is what lets the plumbing be tested without a server.
    fn scripted(
        d: &StrategyDefinition,
    ) -> impl FnMut(StrategyDefinition) -> std::future::Ready<Result<Answer, Value>> + '_ {
        let sets = strategy_lang::probes(d).expect("probes");
        move |probe: StrategyDefinition| {
            let set = &sets[0];
            let measures = set
                .probes()
                .iter()
                .find(|p| {
                    p.definition.nodes.iter().zip(&probe.nodes).all(|(a, b)| a == b)
                })
                .map(|p| p.measures)
                .expect("the probe handed to `run` is one of the set's");
            let n = match measures {
                strategy_lang::Measures::AllClauses => 0,
                strategy_lang::Measures::OnlyClause(0) => 900,
                strategy_lang::Measures::OnlyClause(_) => 0,
                strategy_lang::Measures::AllButClause(_) => unreachable!("two clauses"),
            };
            std::future::ready(Ok(Answer {
                bars_evaluated: 1000,
                bars_true: BTreeMap::from([("long".to_string(), n)]),
            }))
        }
    }

    #[tokio::test]
    async fn measuring_names_the_clause_that_blocks() {
        let d = def(SIGN_ERROR);
        let profile = measure(&d, scripted(&d)).await.expect("measured");
        let out = profile_json(&profile);
        let e = &out["entries"][0];
        assert_eq!(e["verdict"]["kind"], "blocked");
        assert_eq!(e["verdict"]["clause"], "entry.long[0].clause[1]");
        assert_eq!(e["verdict"]["clause_text"], "rsi(close, 14) < 0");
        assert!(e["says"].as_str().unwrap().contains("sole blocker"));
    }

    /// S-5.11's acceptance: every suggestion cites the diagnostic that motivated
    /// it — here a measurement with its numbers, not a sentence.
    #[tokio::test]
    async fn every_suggestion_cites_the_measurement_that_motivated_it() {
        let d = def(SIGN_ERROR);
        let profile = measure(&d, scripted(&d)).await.expect("measured");
        let findings = strategy_lang::Findings {
            blocking: Some(profile),
            ..Default::default()
        };
        let out = suggestions_json(&d, &findings);
        let moves = out["detail"]["moves"].as_array().expect("moves");
        assert!(!moves.is_empty(), "{out:#}");
        for m in moves {
            assert!(m["why"]["diagnostic"].is_string(), "{m:#}");
            assert!(m["why"]["measured"].is_object(), "{m:#}");
            assert!(m["why"]["says"].as_str().is_some_and(|s| !s.is_empty()), "{m:#}");
        }
        assert_eq!(moves[0]["kind"], "change_threshold");
        assert_eq!(moves[0]["why"]["diagnostic"], "sole_blocker");
        assert_eq!(moves[0]["path"], "entry.long[0].clause[1]");
    }

    /// Findings the platform cannot yet produce are accepted from the caller and
    /// still reach the rules that read them.
    #[test]
    fn a_losing_regime_with_a_checked_gate_becomes_a_cited_move() {
        let d = def(SIGN_ERROR);
        let findings = caller_findings(&json!({
            "regimes": [{
                "label": "high_vol", "objective": -0.4, "share_of_window": 0.3,
                "gate": {"family": "rv", "params": [20], "op": "<", "bound": "0.02"}
            }]
        }))
        .expect("readable");
        let out = suggestions_json(&d, &findings);
        let m = &out["detail"]["moves"][0];
        assert_eq!(m["kind"], "add_regime_gate", "{out:#}");
        assert_eq!(m["why"]["diagnostic"], "loses_in_regime");
        // A gate naming a feature the engine does not serve cannot be built.
        let bad = caller_findings(&json!({
            "regimes": [{
                "label": "x", "objective": -1.0, "share_of_window": 0.5,
                "gate": {"family": "not_a_family", "params": [], "op": "<", "bound": "1"}
            }]
        }));
        assert_eq!(bad.unwrap_err()["error"]["code"], "unknown_feature");
    }

    #[tokio::test]
    async fn a_strategy_that_needs_too_many_previews_is_refused_by_name() {
        let clauses: Vec<String> = (2..=14).map(|n| format!("rsi(close, {n}) < 99")).collect();
        let text = format!(
            "strategy wide on $bound_at_init timeframe 1h\nentry long when {}\nsize fixed 0.01\nexit time 24 bars\n",
            clauses.join(" and ")
        );
        // Written as separate inputs would trip the 120-character rule, so the
        // clauses are short and the conjunction is long.
        let d = def(&text);
        let err = measure(&d, |_p| std::future::ready(Err(json!({"error": "must_not_run"}))))
            .await
            .unwrap_err();
        assert_eq!(err["error"]["code"], "too_many_probes");
    }

    #[test]
    fn a_text_door_refuses_slv3_by_name() {
        assert!(looks_like_slv3(
            "# a comment\nstrategy s kind trading asset_class crypto_spot_cex timeframe 1h\n"
        ));
        assert!(!looks_like_slv3(SIGN_ERROR));
        let e = validate_strategy(
            &json!({"text": "strategy s kind trading asset_class c timeframe 1h"}),
            "",
        );
        assert_eq!(e["error"]["code"], "this_is_slv3");
    }

    #[test]
    fn validate_strategy_takes_text_and_reports_the_facts() {
        let ok = validate_strategy(&json!({"text": SIGN_ERROR}), "");
        assert_eq!(ok["valid"], true, "{ok:#}");
        assert_eq!(ok["estimated_cost_class"], "rolling");
        assert!(ok["features_used"].as_array().unwrap().len() >= 2);
        assert!(ok["warmup_bars"].as_u64().unwrap() >= 71);

        // A compile failure comes back in the validator's own shape.
        let bad = validate_strategy(
            &json!({"text": "strategy s on $bound_at_init timeframe 1h\nentry long when nope(close, 14) < 30\nsize fixed 1\nexit none\n"}),
            "",
        );
        assert_eq!(bad["valid"], false);
        let e = &bad["errors"][0];
        assert!(e["code"].is_string() && e["fix"].is_string() && e["path"].is_string(), "{bad:#}");
    }

    #[test]
    fn validate_strategy_takes_text_for_a_sweepable_strategy() {
        // Every skeleton carries `{{param}}` interpolation; the validator must see
        // the materialised copy, or `ema_{{fast}}` is an unknown feature.
        for fam in ["trend", "mean_reversion", "vol_target"] {
            let s = strategy_skeleton(&json!({"hypothesis_family": fam}));
            let text = s["detail"]["text"].as_str().expect("text").to_string();
            let v = validate_strategy(&json!({"text": text}), "");
            assert_eq!(v["valid"], true, "{fam}: {v:#}");
        }
    }

    #[test]
    fn both_doors_at_once_is_refused() {
        let e = validate_strategy(&json!({"text": SIGN_ERROR}), "{}");
        assert_eq!(e["error"]["code"], "ambiguous_source");
        let e = validate_strategy(&json!({}), "");
        assert_eq!(e["error"]["code"], "missing_field");
    }

    /// S-5.15: a comment written above a clause appears beside that clause in
    /// `explain_strategy`, and a stored definition says why it has none.
    /// S-5.8's acceptance: it names every input, entry, exit, size rule and the
    /// warm-up bars — for a swept strategy too, which is every skeleton.
    #[test]
    fn explain_strategy_names_every_part_and_the_warm_up_of_a_swept_strategy() {
        let trend = strategy_skeleton(&json!({"hypothesis_family": "trend"}));
        let text = trend["detail"]["text"].as_str().expect("text");
        let out = explain_strategy(&json!({ "text": text }));
        let d = &out["detail"];
        // Inputs: both averages and the filter, each with a warm-up.
        let inputs = d["inputs"].as_array().expect("inputs");
        assert!(inputs.len() >= 3, "{out:#}");
        assert!(inputs.iter().all(|i| i["warmup_bars"].is_number()), "{out:#}");
        // Entries, clause by clause.
        assert_eq!(d["entries"][0]["clauses"].as_array().unwrap().len(), 2);
        // Exits and the size rule, as written.
        assert_eq!(d["exits"].as_array().unwrap().len(), 3);
        assert_eq!(d["size"].as_array().unwrap().len(), 1);
        // The warm-up: `ema(close, 21)` warms up over 5 × 21 bars, and that is the longest.
        assert_eq!(d["warmup_bars"], 105, "{out:#}");
        assert!(d["warmup_evaluated_at"].is_string());
    }

    /// S-5.8's second clause, at the tool: saving SLv2 under the same name twice
    /// chains the revisions. Lineage is derived by the platform from the head of
    /// the same `strategy_id` (ADR-D-21; `strategy_revision.parent_hash`), so what
    /// this proves is that the text door does its half — it saves the compiled
    /// definition **under the header's name**, adds no `parent_strategy_id` of its
    /// own, and hands back the parent the platform reports. The derivation itself
    /// is the ledger's, and `crates/ledger/tests/pg_strategy_revision.rs` holds it.
    #[tokio::test]
    async fn saving_the_same_name_twice_through_the_text_door_chains_the_revisions() {
        use axum::extract::{Query, State};
        use axum::http::StatusCode;
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Platform {
            heads: BTreeMap<String, String>,
            /// `(slug, parent_strategy_id query parameter)`, per POST.
            posts: Vec<(String, Option<String>)>,
        }
        let state: Arc<Mutex<Platform>> = Arc::default();
        let app = Router::new()
            .route(
                "/api/strategies",
                post(
                    |State(s): State<Arc<Mutex<Platform>>>,
                     Query(q): Query<BTreeMap<String, String>>,
                     Json(body): Json<Value>| async move {
                        let slug = body["strategy_id"].as_str().expect("a slug").to_string();
                        let mut st = s.lock().unwrap();
                        let parent = st.heads.get(&slug).cloned();
                        let hash = format!("rev{}", st.posts.len() + 1);
                        st.heads.insert(slug.clone(), hash.clone());
                        st.posts.push((slug.clone(), q.get("parent_strategy_id").cloned()));
                        (
                            StatusCode::CREATED,
                            Json(json!({
                                "strategy_id": slug,
                                "revision_hash": hash,
                                "parent_revision": parent,
                                "actor_kind": "agent",
                            })),
                        )
                    },
                ),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let api = ApiClient::new(format!("http://{addr}"), "test-token");

        let first = create_strategy(&api, &json!({ "text": SIGN_ERROR }), "").await;
        assert_eq!(first["created"], true, "{first:#}");
        assert_eq!(first["parent_revision"], Value::Null, "a first version has no parent");

        // One edit, then the same header again: that is how an author iterates.
        let edited = SIGN_ERROR.replace("rsi(close, 14) < 0", "rsi(close, 14) < 30");
        let second = create_strategy(&api, &json!({ "text": edited }), "").await;
        assert_eq!(second["created"], true, "{second:#}");
        assert_eq!(
            second["parent_revision"], first["revision_hash"],
            "the second save did not chain onto the first"
        );
        assert_eq!(second["strategy_id"], "sign_error");

        let posts = &state.lock().unwrap().posts;
        assert_eq!(posts.len(), 2);
        assert!(posts.iter().all(|(slug, _)| slug == "sign_error"), "{posts:?}");
        assert!(
            posts.iter().all(|(_, parent)| parent.is_none()),
            "the tool invented a parent_strategy_id: {posts:?}"
        );
    }

    /// A text that does not compile is refused before anything is sent: the stub
    /// below would panic if it were asked.
    #[tokio::test]
    async fn text_that_does_not_compile_never_reaches_the_platform() {
        let api = ApiClient::new("http://127.0.0.1:1", "t");
        let out = create_strategy(
            &api,
            &json!({"text": "strategy s on $bound_at_init timeframe 1h
entry long when nope(close, 14) < 30
size fixed 1
exit none
"}),
            "",
        )
        .await;
        assert_eq!(out["error"]["code"], "validation_failed", "{out:#}");
        assert_eq!(out["errors"][0]["code"], "unknown_feature");
    }

    /// A swept draft reaches the preview with its parameters resolved, and
    /// anything else is left exactly as it was sent.
    #[test]
    fn a_swept_draft_is_previewed_at_its_defaults_and_nothing_else_is_touched() {
        let trend = strategy_skeleton(&json!({"hypothesis_family": "trend"}));
        let swept = compile_to_json(trend["detail"]["text"].as_str().unwrap());
        let params = json!({"definition": swept, "instrument_id": "BTC-USD", "start": "a", "end": "b"});
        let out = with_materialised_draft(&params);
        let sent = out["definition"].to_string();
        assert!(!sent.contains("{{"), "a parameter reached the preview: {sent}");
        assert!(sent.contains("ema_7") && sent.contains("ema_21"), "{sent}");
        // The rest of the request is untouched.
        assert_eq!(out["instrument_id"], "BTC-USD");

        // No parameters, a v3 graph, no definition at all: passed through.
        let plain = json!({"definition": compile_to_json(SIGN_ERROR), "start": "a"});
        assert_eq!(with_materialised_draft(&plain), plain);
        let v3 = json!({"definition": {"format": "tbot.strategy/3", "kind": "trading"}});
        assert_eq!(with_materialised_draft(&v3), v3);
        assert_eq!(with_materialised_draft(&json!({})), json!({}));
    }

    fn compile_to_json(text: &str) -> Value {
        serde_json::to_value(def(text)).expect("serialises")
    }

    /// The form reaches the platform: its schema is a tool's input schema, and a
    /// filled form compiles through the language.
    #[test]
    fn a_filled_form_compiles_and_the_schema_is_the_tools() {
        // The 1,200-byte descriptor budget keeps the grammar out of the tool's
        // own schema; it is served by the guide.
        let grammar = get_slv2_guide(&json!({"section": "form.schema"}));
        let schema = grammar["schema"].to_string();
        assert!(!schema.contains("$ref"), "a reference survived inlining");
        assert!(schema.contains("\"rsi\""), "the engine's families are in the schema");
        // A tool's schema is listed to every client: the per-family form would be
        // ~270 KB, the compact one is a few.
        assert!(schema.len() < 40_000, "{} bytes", schema.len());

        let form = json!({
            "header": {"name": "form_demo", "instrument": "$bound_at_init", "timeframe": "1h"},
            "entries": [{
                "side": "long", "combine": "all",
                "filters": [{
                    "filter": "compare",
                    "lhs": {"operand": "feature", "family": "rsi", "args": [14]},
                    "op": "lt",
                    "rhs": {"operand": "number", "value": "30"}
                }]
            }],
            "size": {"size": "fixed", "quantity": "0.01"},
            "exits": [{"exit": "stop", "percent": "2"}, {"exit": "time", "bars": 24}]
        });
        // The form is also a door of `validate_strategy`, ending at the same
        // validator, so no separate tool is needed.
        let v = validate_strategy(&json!({ "form": form }), "");
        assert_eq!(v["valid"], true, "{v:#}");
        let out = compile_strategy_form(&form);
        assert_eq!(out["detail"]["fills"], 5, "{out:#}");
        assert!(out["detail"]["text"].as_str().unwrap().contains("rsi(close, 14) < 30"));
        assert_eq!(out["summary_fields"]["strategy_id"], "form_demo");

        // The form says it cannot be read, by name, when it is not a form.
        assert_eq!(compile_strategy_form(&json!({"header": 1}))["error"]["code"], "unreadable_form");
        // And a fill the language refuses comes back with the language's own reason.
        let mut bad = form;
        bad["entries"][0]["filters"][0]["lhs"]["family"] = json!("not_a_family");
        let out = compile_strategy_form(&bad);
        assert_eq!(out["error"]["code"], "compile_failed", "{out:#}");
        assert_eq!(out["diagnostics"][0]["code"], "unknown_feature");
    }

    /// S-5.15's second half, at the tool the model already uses: a draft that
    /// never fires comes back with the profile attached, and one that fires does
    /// not pay for it.
    #[tokio::test]
    async fn a_preview_that_never_fires_names_the_blocking_clause() {
        use axum::extract::State;
        use axum::routing::post;
        use axum::{Json, Router};
        use std::sync::{Arc, Mutex};

        // The stub answers every preview the way the platform would for a
        // strategy whose first clause is true on 900 of 1000 bars and whose
        // second is true on none — told apart by the expression it is sent.
        let calls: Arc<Mutex<usize>> = Arc::default();
        let app = Router::new()
            .route(
                "/api/agent/strategy/preview",
                post(
                    |State(calls): State<Arc<Mutex<usize>>>, Json(body): Json<Value>| async move {
                        *calls.lock().unwrap() += 1;
                        let exprs: Vec<String> = body["definition"]["nodes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|n| n["expr"].as_str().map(str::to_string))
                            .collect();
                        let e = exprs.join(" ");
                        // Probes also mention every declared feature (a guard),
                        // so tell them apart by the clause text, not the names.
                        let dead = e.contains("rsi_14') < 0") || e.contains("(0 > 1)");
                        let bars_true = if !dead && e.contains("rel_volume_20') < 50") {
                            900
                        } else {
                            0
                        };
                        Json(json!({
                            "bars_evaluated": 1000,
                            "entries": 0,
                            "never_fires_code": "never_fires",
                            "signals": [{"signal": "long", "bars_true": bars_true, "fires": 0}],
                        }))
                    },
                ),
            )
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let api = ApiClient::new(format!("http://{addr}"), "t");

        let params = json!({
            "definition": compile_to_json(SIGN_ERROR),
            "instrument_id": "BTC-USD", "timeframe": "1h",
            "start": "2024-01-01T00:00:00Z", "end": "2024-06-01T00:00:00Z",
        });
        let out = preview_signals(&api, &params).await;
        let entry = &out["blocking_profile"]["entries"][0];
        assert_eq!(entry["verdict"]["kind"], "blocked", "{out:#}");
        assert_eq!(entry["verdict"]["clause_text"], "rsi(close, 14) < 0");
        // 1 preview for the draft itself, then the two probes of a two-clause entry
        // plus the whole: three. Four calls, all reads.
        assert_eq!(*calls.lock().unwrap(), 4);
    }

    #[test]
    fn explain_strategy_shows_the_comments_the_author_wrote() {
        let text = "strategy commented on $bound_at_init timeframe 1h
# 60 rather than 70: this filter refuses an extended move, it does not call a top.
entry long when ema(close, 7) > ema(close, 21) and rsi(close, 14) < 60
size fixed 0.01
exit time 24 bars
";
        let out = explain_strategy(&json!({ "text": text }));
        let comments = out["detail"]["comments"].as_array().expect("comments");
        let entry = comments
            .iter()
            .find(|c| c["clause"] == "entry.long[0]")
            .unwrap_or_else(|| panic!("{out:#}"));
        assert!(entry["says"][0].as_str().unwrap().starts_with("60 rather than 70"));
        // The base answer is untouched: it still names the entry clauses.
        assert_eq!(out["detail"]["entries"][0]["path"], "entry.long[0]");

        // The same strategy, stored: no comments, and the answer says why.
        let stored = def(text);
        let out = explain_strategy(&json!({ "definition": stored }));
        assert_eq!(out["detail"]["comments"], json!([]));
        assert!(out["detail"]["comments_are_not_stored"].is_string(), "{out:#}");
    }

    #[test]
    fn a_skeleton_is_served_and_a_missing_family_is_refused_by_name() {
        let s = strategy_skeleton(&json!({"hypothesis_family": "trend"}));
        assert!(s["detail"]["text"].as_str().unwrap().contains("entry long"));
        assert!(s["detail"]["params"].as_object().is_some_and(|p| p.contains_key("fast")));
        let carry = strategy_skeleton(&json!({"hypothesis_family": "carry"}));
        assert_eq!(carry["error"]["code"], "not_writable_yet");
        let unknown = strategy_skeleton(&json!({"hypothesis_family": "astrology"}));
        assert_eq!(unknown["error"]["code"], "unknown_family");
        assert!(unknown["known"].as_array().unwrap().len() >= 6);
    }

    #[test]
    fn the_guide_is_served_by_the_anchor_a_diagnostic_names() {
        let index = get_slv2_guide(&json!({}));
        assert!(index["sections"].as_array().unwrap().len() >= 10);
        let one = get_slv2_guide(&json!({"section": "guide#exits.units"}));
        assert!(one["text"].as_str().unwrap().contains("atr"), "{one:#}");
        assert_eq!(get_slv2_guide(&json!({"section": "nope"}))["error"]["code"], "unknown_section");
    }

    #[test]
    fn a_preview_answer_is_read_and_a_strange_one_is_refused() {
        let a = read_answer(&json!({
            "bars_evaluated": 10,
            "signals": [{"signal": "long", "bars_true": 3, "fires": 1}]
        }))
        .expect("readable");
        assert_eq!(a.bars_true["long"], 3);
        assert!(read_answer(&json!({"signals": []})).is_err());
    }
}
