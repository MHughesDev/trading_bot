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
            return crate::tool_error::refuse_extra(
                "unknown_cost",
                format!("cost class `{other}` is not one of the feature cost classes."),
                "set cost to \"cheap\", \"rolling\" or \"model\", or omit it",
                json!({ "cost": other, "known": ["cheap", "rolling", "model"] }),
            )
        }
        None => None,
    };

    let all = features::feature_families().len();
    let matched = features::list_features(text("family"), cost, text("query"));
    // Paged: the cap is what makes ignoring the filters survivable, and `offset`
    // is what makes the families past it reachable at all (S-5.2).
    let offset = params
        .get("offset")
        .and_then(Value::as_u64)
        .map_or(0, |o| usize::try_from(o).unwrap_or(usize::MAX));
    let shown: Vec<Value> = matched
        .iter()
        .skip(offset)
        .take(MAX_FAMILIES)
        .map(|f| render(f))
        .collect();
    let truncated = matched.len().saturating_sub(offset.saturating_add(shown.len()));
    let next_offset = (truncated > 0).then(|| offset + shown.len());

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
            "offset": offset,
            "next_offset": next_offset,
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
            return crate::tool_error::missing_field(field);
        }
    }
    match api
        .post("/api/agent/strategy/preview", params.clone())
        .await
    {
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
            Err(e) => Err(crate::tool_error::refuse(
                "invalid_definition",
                format!("`definition` is not a strategy definition: {e}"),
                "send the object `get_strategy` returned, or send SLv2 in `text` instead",
            )),
        },
        (None, None) => Err(crate::tool_error::missing_field_with(
            "text",
            "send SLv2 in `text`, or a stored definition object in `definition`",
        )),
        (Some(_), Some(_)) => Err(crate::tool_error::refuse(
            "ambiguous_source",
            "both `text` and `definition` were sent; they are different languages and this tool will not guess.",
            "send `text` or `definition`, not both",
        )),
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
    crate::tool_error::refuse_extra(
        "compile_failed",
        format!("the SLv2 text has {} diagnostic(s); see `diagnostics`.", ds.len()),
        "apply the `fix` on each entry in `diagnostics`, then call again",
        json!({
            "n_diagnostics": ds.len(),
            "diagnostics": ds.iter().map(render_diagnostic).collect::<Vec<_>>(),
        }),
    )
}

/// A refusal the language raised about a *stored* definition: it holds something
/// SLv2 has no text for. Its own code, because the fix is different — nothing
/// the caller writes will help, and the answer names the mechanism that is
/// missing rather than the clause.
fn one_diagnostic(d: strategy_lang::Diagnostic) -> Value {
    crate::tool_error::refuse_extra(
        "not_expressible_in_slv2",
        "the stored definition holds something SLv2 has no text for.",
        "keep working on it as a definition object (get_strategy / create_strategy with `definition`); nothing in `text` will help",
        json!({ "diagnostic": render_diagnostic(&d) }),
    )
}

/// `compile_strategy` — SLv2 text to the canonical definition. Persists nothing.
#[must_use]
pub fn compile_strategy(params: &Value) -> Value {
    let Some(text) = params.get("text").and_then(Value::as_str) else {
        return crate::tool_error::missing_field("text");
    };
    let def = match strategy_lang::text_to_definition(text) {
        Ok(d) => d,
        Err(ds) => return diagnostics(&ds),
    };
    let Ok(definition) = serde_json::to_value(&def) else {
        return crate::tool_error::refuse(
            "serialization_error",
            "the result could not be serialised.",
            "call again; if it repeats, report the strategy that triggers it",
        );
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
            return Err(crate::tool_error::missing_field(field));
        };
        serde_json::from_value(v.clone()).map_err(
            |e| {
                crate::tool_error::refuse_ids(
                    "invalid_definition",
                    format!("`{field}` is not a strategy definition: {e}"),
                    format!("send `{field}` as the object get_strategy returned"),
                    &[("field", field)],
                )
            },
        )
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
        return crate::tool_error::missing_field("definition");
    };
    let def: StrategyDefinition = match serde_json::from_value(v.clone()) {
        Ok(d) => d,
        Err(e) => {
            return crate::tool_error::refuse(
                "invalid_definition",
                format!("`definition` is not a strategy definition: {e}"),
                "send the object get_strategy returned",
            )
        }
    };
    let Some(raw) = params.get("edit").filter(|v| !v.is_null()) else {
        return crate::tool_error::missing_field("edit");
    };
    // The edit is a closed enum with typed clause fills, so an unreadable one
    // is refused here with serde's own message rather than becoming a
    // half-applied version. A free-form "what to change" string is the defect
    // class this repository keeps finding.
    let edit: strategy_lang::Edit = match serde_json::from_value(raw.clone()) {
        Ok(e) => e,
        Err(e) => {
            return crate::tool_error::refuse(
                "unreadable_edit",
                format!("`edit` could not be read: {e}"),
                "name the edit's kind in `edit` and give that kind's fields; clause addresses are the ones \
                 `explain_strategy` prints, such as `entry.long[0].clause[1]`",
            )
        }
    };

    let applied = match strategy_lang::apply_move(&def, &edit) {
        Ok(a) => a,
        Err(d) => {
            return crate::tool_error::from_diagnostic_see(d.code.as_str(), &d.message, d.fix.as_deref().unwrap_or(crate::tool_error::DIAGNOSTIC_FIX), &d.see)
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
        return crate::tool_error::refuse(
            "serialization_error",
            "the result could not be serialised.",
            "call again; if it repeats, report the strategy that triggers it",
        );
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

// ── lookahead_truncation_test (ADR-D-19; AGENT-006 §5.4; Q-S3-03) ────────────
//
// Freqtrade's shipped `lookahead-analysis`, in substance: run the same strategy
// over a shorter window ending earlier and check that the shorter run's signal
// firings are exactly the prefix of the longer run's. It is *not* §5.4's
// `future_perturbation_test`, which moves a feature column forward by
// `shift_bars`; that needs a perturbation mode the strategy runtime does not
// have, and shipping this under that name would claim a computation it did not
// perform.
//
// # What this can and cannot see, measured rather than assumed
//
// `backtest::preview::preview_signals` walks bars in order and evaluates every
// feature through `features::runtime::value_at(f, &rows, decision)`, where
// `rows` holds only bars up to and including the decision bar and
// `dataplane::feature::WindowedFrame::back(k)` resolves `decision - k` and
// refuses `k >= lookback_bars`. **No feature on this path can read a future
// bar.** The vectorised leak Freqtrade's analyser exists to catch — a
// `shift(-1)`, a full-sample `max()`, a scaler fitted on the whole frame — has
// no expression here.
//
// So this is not an audit of the simulator, and a held prefix is not a clean
// bill of health. What it does measure:
//
//   * a feature or node whose answer depends on the window's *length* rather
//     than its contents;
//   * disagreement between two runs of the same code over the same prefix;
//   * a bar revised between the two reads — the two previews read at different
//     `as_of` instants, so a late correction inside the window shows up, and is
//     reported as `series_revised` rather than blamed on the strategy;
//   * and the regression that matters most: the day a batch path appears under
//     the preview — `features::runtime::backfill_column` is already one, for
//     other callers — the structural argument above stops holding and this is
//     the test that notices.
//
// # A leak not found is not a pass
//
// Every refusal to conclude is named. A strategy whose warm-up exceeds the
// shorter window produces no firings at all, and reporting that as "no lookahead
// detected" would be the exact failure this catalogue exists to prevent, so it
// is `not_exercised` with a reason, and the result always says how many firings
// it actually compared.

/// One signal firing, reduced to the fields the prefix invariant is about.
#[derive(Clone, Debug, PartialEq)]
pub struct Firing {
    pub signal: String,
    /// Index into the window. Both runs share `start`, so indices line up
    /// whenever the underlying series does — and when they do not, `at` says so
    /// and the pair is reported inconclusive rather than compared.
    pub bar: u64,
    pub at: String,
    pub close: f64,
}

/// What one preview said, reduced to what the comparison needs.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSignals {
    pub n_bars: u64,
    pub warmup_bars: u64,
    pub bars_evaluated: u64,
    /// Rising edges over the whole window, summed across signals. The preview
    /// caps `sample_events`; this comes from the per-signal stats, which are not
    /// capped, and is what lets the monotonicity check see past the cap.
    pub total_fires: u64,
    pub firings: Vec<Firing>,
}

impl RunSignals {
    /// Whether `sample_events` stopped short of the firings the stats counted —
    /// derived from the answer rather than from a second copy of the preview's
    /// own `MAX_EVENTS`, which is private and would drift.
    #[must_use]
    pub fn events_capped(&self) -> bool {
        self.total_fires > self.firings.len() as u64
    }
}

/// A way the prefix failed. Closed, because a caller branches on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakKind {
    /// The longer run fired inside the overlap and the shorter run did not.
    MissingFiring,
    /// The shorter run fired where the longer run did not.
    ExtraFiring,
    /// Same bar and instant, different signal.
    SignalChanged,
    /// More rising edges in the shorter window than in the window containing it.
    MoreFiringsInShorterWindow,
    /// The derived warm-up changed between two runs of one definition.
    WarmupChanged,
}

impl BreakKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingFiring => "missing_firing",
            Self::ExtraFiring => "extra_firing",
            Self::SignalChanged => "signal_changed",
            Self::MoreFiringsInShorterWindow => "more_firings_in_shorter_window",
            Self::WarmupChanged => "warmup_changed",
        }
    }
}

/// Why a window pair could not say anything. Never "passed".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotExercised {
    /// The preview refused the shorter window outright.
    ShortWindowRefused,
    /// Truncating removed no bars, so there were not two windows.
    TruncationRemovedNoBars,
    /// No bar in the shorter window had a value for every feature — the warm-up
    /// exceeds it, and nothing could have fired whatever the conditions say.
    ShortWindowNeverEvaluated,
    /// Both runs were quiet over the overlap: nothing to compare.
    NoFiringsInOverlap,
    /// The two runs disagree about which bar an instant is, so bar indices are
    /// not comparable. A data question, not a strategy one.
    SeriesMisaligned,
    /// The same bar came back with a different close, so a bar was revised
    /// between the two point-in-time reads.
    SeriesRevised,
}

impl NotExercised {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ShortWindowRefused => "short_window_refused",
            Self::TruncationRemovedNoBars => "truncation_removed_no_bars",
            Self::ShortWindowNeverEvaluated => "short_window_never_evaluated",
            Self::NoFiringsInOverlap => "no_firings_in_overlap",
            Self::SeriesMisaligned => "series_misaligned",
            Self::SeriesRevised => "series_revised",
        }
    }
}

/// What one (full, truncated) pair concluded.
#[derive(Clone, Debug, PartialEq)]
pub enum PairOutcome {
    /// The prefix held **over the firings compared**, which is the number
    /// carried here and never dropped from the answer.
    PrefixHeld {
        firings_compared: u64,
    },
    PrefixBroken {
        kind: BreakKind,
        at_bar: Option<u64>,
        says: String,
    },
    NotExercised {
        reason: NotExercised,
        says: String,
    },
}

impl PairOutcome {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrefixHeld { .. } => "prefix_held",
            Self::PrefixBroken { .. } => "prefix_broken",
            Self::NotExercised { .. } => "not_exercised",
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::PrefixHeld { firings_compared } => json!({
                "outcome": "prefix_held",
                "firings_matched": firings_compared,
            }),
            Self::PrefixBroken { kind, at_bar, says } => json!({
                "outcome": "prefix_broken",
                "break": kind.as_str(),
                "at_bar": at_bar,
                "says": says,
            }),
            Self::NotExercised { reason, says } => json!({
                "outcome": "not_exercised",
                "reason": reason.as_str(),
                "says": says,
            }),
        }
    }
}

/// The verdict over every pair. Three values, and none of them is "clean".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    PrefixBroken,
    PrefixHeld,
    NotExercised,
}

impl Verdict {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PrefixBroken => "prefix_broken",
            Self::PrefixHeld => "prefix_held",
            Self::NotExercised => "not_exercised",
        }
    }
}

/// Roll the pairs up. One break outranks any number of holds; no exercised pair
/// outranks any number of holds that compared nothing, which cannot happen —
/// `PrefixHeld` is only reachable with at least one firing compared.
#[must_use]
pub fn verdict_over(pairs: &[PairOutcome]) -> Verdict {
    if pairs
        .iter()
        .any(|p| matches!(p, PairOutcome::PrefixBroken { .. }))
    {
        return Verdict::PrefixBroken;
    }
    if pairs
        .iter()
        .any(|p| matches!(p, PairOutcome::PrefixHeld { firings_compared } if *firings_compared > 0))
    {
        return Verdict::PrefixHeld;
    }
    Verdict::NotExercised
}

/// The comparison, as a pure function of two previews.
///
/// The order is the order of blame, exactly as `preview::diagnose` orders its
/// reasons: a run that could not be evaluated is not a strategy verdict, a
/// series that moved under us is not a strategy verdict, and only when neither
/// of those is the answer does the prefix itself get to speak.
#[must_use]
pub fn compare(full: &RunSignals, short: &RunSignals) -> PairOutcome {
    if short.warmup_bars != full.warmup_bars {
        return PairOutcome::PrefixBroken {
            kind: BreakKind::WarmupChanged,
            at_bar: None,
            says: format!(
                "the same definition derived a {}-bar warm-up on the full window and {} on the \
                 truncated one. Warm-up is a property of the definition, so the two runs did not \
                 evaluate the same strategy.",
                full.warmup_bars, short.warmup_bars
            ),
        };
    }
    if short.n_bars >= full.n_bars {
        return PairOutcome::NotExercised {
            reason: NotExercised::TruncationRemovedNoBars,
            says: format!(
                "the truncated window loaded {} bars and the full one {}. Nothing was truncated, \
                 so there was no prefix to check.",
                short.n_bars, full.n_bars
            ),
        };
    }
    if short.bars_evaluated == 0 {
        return PairOutcome::NotExercised {
            reason: NotExercised::ShortWindowNeverEvaluated,
            says: format!(
                "no bar in the truncated window had a value for every feature — the warm-up is \
                 {} bars and the window is {}. No signal could fire here whatever the conditions \
                 say, so a quiet shorter run says nothing about lookahead.",
                short.warmup_bars, short.n_bars
            ),
        };
    }
    // Sees past `sample_events`' cap: the containing window cannot have fewer
    // rising edges than the window it contains.
    if short.total_fires > full.total_fires {
        return PairOutcome::PrefixBroken {
            kind: BreakKind::MoreFiringsInShorterWindow,
            at_bar: None,
            says: format!(
                "the truncated window fired {} times and the full window, which contains it, \
                 fired {}. Every firing in the shorter run must also be a firing in the longer \
                 one.",
                short.total_fires, full.total_fires
            ),
        };
    }

    // The firings the full run is known to have inside the overlap. Because
    // `sample_events` is the *first* N in bar order, this prefix is exactly the
    // truncated run's own event list whether or not the cap was reached: if the
    // cap fell inside the overlap both lists stop at the same event, and if it
    // fell outside, this list already covers all of the overlap.
    let boundary = short.n_bars;
    let expected: Vec<&Firing> = full.firings.iter().filter(|f| f.bar < boundary).collect();

    let mut expected_bars = group(expected.iter().copied());
    let mut got_bars = group(short.firings.iter());

    // When the event cap fell inside the overlap, the last bar in each list may
    // be a *partial* bar — the cap can land between two signals of one bar, and
    // which of them was kept is the same arbitrary `HashSet` order. Both lists
    // stop at the same event, so dropping the last bar from both keeps them
    // aligned; judging it would be judging a coin flip.
    let truncated_mid_list =
        short.events_capped() || (full.events_capped() && expected.len() == full.firings.len());
    if truncated_mid_list {
        expected_bars.pop();
        got_bars.pop();
    }

    let mut firings_compared = 0u64;
    for i in 0..expected_bars.len().max(got_bars.len()) {
        match (expected_bars.get(i), got_bars.get(i)) {
            (Some(e), Some(g)) => {
                if e.at != g.at || e.bar != g.bar {
                    if e.at == g.at || e.bar == g.bar {
                        return PairOutcome::NotExercised {
                            reason: NotExercised::SeriesMisaligned,
                            says: format!(
                                "the runs disagree about the window itself: the full run fires \
                                 at bar {} ({}) where the truncated run fires at bar {} ({}). \
                                 Bar indices are not comparable, so the prefix was not judged.",
                                e.bar, e.at, g.bar, g.at
                            ),
                        };
                    }
                    let (kind, at_bar, says) = if g.bar < e.bar {
                        (
                            BreakKind::ExtraFiring,
                            g.bar,
                            format!(
                                "the truncated run fired at bar {} ({}); the full window, which \
                                 contains that bar, did not fire there at all.",
                                g.bar, g.at
                            ),
                        )
                    } else {
                        (
                            BreakKind::MissingFiring,
                            e.bar,
                            format!(
                                "the full run fired at bar {} ({}), inside the truncated window, \
                                 and the truncated run's next firing is at bar {}.",
                                e.bar, e.at, g.bar
                            ),
                        )
                    };
                    return PairOutcome::PrefixBroken {
                        kind,
                        at_bar: Some(at_bar),
                        says,
                    };
                }
                // Data before strategy, the same order of blame `preview::diagnose`
                // uses. A corrected close changes the feature values, so a signal
                // that also changed is *explained* by the correction; calling it a
                // break would report a leak the data alone accounts for.
                if e.close != g.close {
                    return PairOutcome::NotExercised {
                        reason: NotExercised::SeriesRevised,
                        says: format!(
                            "bar {} ({}) closed at {} on the full read and {} on the truncated \
                             one. The two previews read at different `as_of` instants, so a bar \
                             was corrected between them — a data finding, not a strategy one.",
                            e.bar, e.at, e.close, g.close
                        ),
                    };
                }
                if e.signals != g.signals {
                    let missing: Vec<&str> = e
                        .signals
                        .difference(&g.signals)
                        .map(String::as_str)
                        .collect();
                    let extra: Vec<&str> = g
                        .signals
                        .difference(&e.signals)
                        .map(String::as_str)
                        .collect();
                    let kind = match (missing.is_empty(), extra.is_empty()) {
                        (false, true) => BreakKind::MissingFiring,
                        (true, false) => BreakKind::ExtraFiring,
                        _ => BreakKind::SignalChanged,
                    };
                    return PairOutcome::PrefixBroken {
                        kind,
                        at_bar: Some(e.bar),
                        says: format!(
                            "bar {} ({}) fired {:?} on the full window and {:?} on the truncated \
                             one, on identical bar data.",
                            e.bar,
                            e.at,
                            e.signals.iter().collect::<Vec<_>>(),
                            g.signals.iter().collect::<Vec<_>>()
                        ),
                    };
                }
                firings_compared += e.signals.len() as u64;
            }
            (Some(e), None) => {
                return PairOutcome::PrefixBroken {
                    kind: BreakKind::MissingFiring,
                    at_bar: Some(e.bar),
                    says: format!(
                        "the full run fired {:?} at bar {} ({}), which is inside the truncated \
                         window, and the truncated run has no firing there.",
                        e.signals.iter().collect::<Vec<_>>(),
                        e.bar,
                        e.at
                    ),
                };
            }
            (None, Some(g)) => {
                return PairOutcome::PrefixBroken {
                    kind: BreakKind::ExtraFiring,
                    at_bar: Some(g.bar),
                    says: format!(
                        "the truncated run fired {:?} at bar {} ({}) and the full window, which \
                         contains that bar, did not.",
                        g.signals.iter().collect::<Vec<_>>(),
                        g.bar,
                        g.at
                    ),
                };
            }
            (None, None) => unreachable!("the loop runs to the longer of the two"),
        }
    }

    if firings_compared == 0 {
        return PairOutcome::NotExercised {
            reason: NotExercised::NoFiringsInOverlap,
            says: if truncated_mid_list {
                format!(
                    "the only firings visible inside the first {boundary} bars were on the one \
                     bar the event cap cut in half, which cannot be judged. Nothing was compared."
                )
            } else {
                format!(
                    "neither run fired inside the first {boundary} bars, so there was nothing to \
                     compare. The strategy was evaluated on {} bars of the truncated window and \
                     stayed quiet.",
                    short.bars_evaluated
                )
            },
        };
    }
    PairOutcome::PrefixHeld { firings_compared }
}

/// The firings of one bar, as a set.
///
/// `preview_signals` walks `true_now.difference(&active)` — a `HashSet`
/// difference — so two signals that fire on the *same* bar come back in an
/// arbitrary order, and `std`'s default hasher is seeded per set, so the order
/// differs between two runs of the same code on the same data. Comparing the
/// flat list position by position would read that as a changed signal and
/// report a leak that is not there. Order *between* bars is real and is still
/// compared; order *within* one is not information.
#[derive(Clone, Debug, PartialEq)]
struct BarFirings {
    bar: u64,
    at: String,
    close: f64,
    signals: std::collections::BTreeSet<String>,
}

/// Collapse a bar-ordered firing list onto its bars.
///
/// Merges only *consecutive* runs of one bar, which is what the producer emits:
/// the preview's signal loop sits inside its per-bar loop. A bar that reappeared
/// later would therefore become a second entry and be reported as a
/// misalignment, which is the right answer for a list that is not bar-ordered.
fn group<'a>(firings: impl IntoIterator<Item = &'a Firing>) -> Vec<BarFirings> {
    let mut out: Vec<BarFirings> = Vec::new();
    for f in firings {
        match out.last_mut() {
            Some(g) if g.bar == f.bar && g.at == f.at => {
                g.signals.insert(f.signal.clone());
            }
            _ => out.push(BarFirings {
                bar: f.bar,
                at: f.at.clone(),
                close: f.close,
                signals: std::iter::once(f.signal.clone()).collect(),
            }),
        }
    }
    out
}

/// Read a preview answer into the shape [`compare`] takes.
///
/// Strict on purpose: a malformed event is an error rather than a dropped one,
/// because a dropped event reads as a missing firing and would be reported as a
/// leak.
fn read_run(v: &Value) -> Result<RunSignals, Value> {
    let unreadable = || {
        crate::tool_error::refuse_extra(
            "unreadable_preview",
            "the platform's preview answer was not in the shape this tool reads.",
            "call preview_signals directly and check it answers; if it does, report the mismatch",
            json!({ "detail": v.clone() }),
        )
    };
    if v.get("error").is_some() {
        return Err(v.clone());
    }
    let u = |k: &str| v.get(k).and_then(Value::as_u64);
    let (Some(n_bars), Some(warmup_bars), Some(bars_evaluated)) =
        (u("n_bars"), u("warmup_bars"), u("bars_evaluated"))
    else {
        return Err(unreadable());
    };

    let mut total_fires = 0u64;
    for s in v
        .get("signals")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match s.get("fires").and_then(Value::as_u64) {
            Some(n) => total_fires += n,
            None => return Err(unreadable()),
        }
    }

    let mut firings = Vec::new();
    for e in v
        .get("sample_events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let parsed = (|| {
            Some(Firing {
                signal: e.get("signal")?.as_str()?.to_string(),
                bar: e.get("bar")?.as_u64()?,
                at: e.get("at")?.as_str()?.to_string(),
                close: e.get("close")?.as_f64()?,
            })
        })();
        match parsed {
            Some(f) => firings.push(f),
            None => return Err(unreadable()),
        }
    }

    Ok(RunSignals {
        n_bars,
        warmup_bars,
        bars_evaluated,
        total_fires,
        firings,
    })
}

/// The truncated ends, as fractions of the window's span.
///
/// Two by default: one that halves the window and one that takes a quarter off.
/// A single truncation can be unlucky — the boundary can land between firings —
/// and more than four is four times the cost for a question the first two
/// usually answer.
const DEFAULT_FRACTIONS: [f64; 2] = [0.5, 0.75];
const MAX_FRACTIONS: usize = 4;

/// Read and bound `fractions`.
///
/// Numbers rather than a free-form string, and checked rather than clamped: a
/// fraction of 0 or 1 is not a shorter window, and silently moving it would make
/// the tool report on a window nobody asked for.
fn read_fractions(params: &Value) -> Result<Vec<f64>, Value> {
    let Some(raw) = params.get("fractions").filter(|v| !v.is_null()) else {
        return Ok(DEFAULT_FRACTIONS.to_vec());
    };
    let Some(items) = raw.as_array() else {
        return Err(crate::tool_error::refuse(
            "invalid_fractions",
            "`fractions` is not an array.",
            "send an array of numbers strictly between 0 and 1, e.g. [0.5, 0.75]",
        ));
    };
    if items.is_empty() || items.len() > MAX_FRACTIONS {
        return Err(crate::tool_error::refuse(
            "invalid_fractions",
            format!("{} fractions were sent.", items.len()),
            format!("send between 1 and {MAX_FRACTIONS} fractions"),
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item.as_f64() {
            Some(f) if f.is_finite() && f > 0.0 && f < 1.0 => out.push(f),
            _ => {
                return Err(crate::tool_error::refuse(
                    "invalid_fractions",
                    format!("`{item}` is not a fraction of the window."),
                    "make every fraction a number strictly between 0 and 1; 1 is the full window and 0 is nothing",
                ))
            }
        }
    }
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let before = out.len();
    out.dedup();
    if out.len() != before {
        return Err(crate::tool_error::refuse(
            "invalid_fractions",
            "a fraction is repeated; the same window twice costs a second read and answers the same question.",
            "drop the repeated fraction",
        ));
    }
    Ok(out)
}

/// The truncated end for one fraction of `[start, end]`.
#[must_use]
pub fn truncated_end(
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
    fraction: f64,
) -> chrono::DateTime<chrono::Utc> {
    #[allow(clippy::cast_possible_truncation)]
    let ms = ((end - start).num_milliseconds() as f64 * fraction) as i64;
    start + chrono::Duration::milliseconds(ms)
}

fn rfc3339(params: &Value, field: &str) -> Result<chrono::DateTime<chrono::Utc>, Value> {
    params
        .get(field)
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
        .ok_or_else(|| {
            crate::tool_error::refuse_ids(
                "invalid_timestamp",
                format!("`{field}` is missing or not an RFC3339 instant."),
                format!("send `{field}` as RFC3339, e.g. 2025-01-01T00:00:00Z"),
                &[("field", field)],
            )
        })
}

/// One preview over `[start, window_end]`.
///
/// The caller's own arguments are forwarded whole with only `end` replaced, so
/// `project_id`, `timeframe` and anything the route gains later travel with
/// every run. `fractions` is this tool's argument and not the route's, and the
/// route declares `deny_unknown_fields`.
async fn one_preview(
    api: &ApiClient,
    params: &Value,
    window_end: chrono::DateTime<chrono::Utc>,
) -> Result<Value, crate::ApiError> {
    let mut body = params.clone();
    if let Some(map) = body.as_object_mut() {
        map.remove("fractions");
        map.insert(
            "end".into(),
            json!(window_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
        );
    }
    api.post("/api/agent/strategy/preview", body).await
}

/// `lookahead_truncation_test` — the prefix invariant, over real windows.
///
/// Routed through `preview_signals`' own route for every read, so there is no
/// second path to bars: the point-in-time store, the research cutoff and the
/// sealed-holdout refusal all apply to each of the runs exactly as they apply to
/// one preview. That is also why the trial cost is `TrialKind::None` at the
/// declaration and not a function of `fractions` — the route it calls writes no
/// ledger row at any count.
pub async fn lookahead_truncation_test(api: &ApiClient, params: &Value) -> Value {
    for field in ["definition", "instrument_id", "start", "end"] {
        if params.get(field).is_none() {
            return crate::tool_error::missing_field(field);
        }
    }
    let fractions = match read_fractions(params) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let (start, end) = match (rfc3339(params, "start"), rfc3339(params, "end")) {
        (Ok(s), Ok(e)) => (s, e),
        (Err(e), _) | (_, Err(e)) => return e,
    };
    if end <= start {
        return crate::tool_error::refuse(
            "empty_window",
            "`end` is not after `start`; there is no shorter window inside an empty one.",
            "make `end` later than `start`",
        );
    }

    let full_raw = match one_preview(api, params, end).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let full = match read_run(&full_raw) {
        Ok(r) => r,
        Err(e) => {
            return crate::tool_error::refuse_extra(
                "full_window_refused",
                "the full window did not preview, so no shorter window can be compared to it.",
                "fix what `detail` reports so preview_signals runs on the full window, then call again",
                json!({ "detail": e }),
            )
        }
    };
    if full.bars_evaluated == 0 {
        return json!({
            "summary_fields": {
                "verdict": Verdict::NotExercised.as_str(),
                "n_exercised": 0,
                "n_windows": fractions.len(),
                "firings_matched": 0,
            },
            "detail": {
                "verdict": Verdict::NotExercised.as_str(),
                "n_exercised": 0,
                "n_windows": fractions.len(),
                "firings_matched": 0,
                "says": format!(
                    "the full window itself was never evaluated — warm-up is {} bars and the \
                     window has {}. No truncation of it could say anything.",
                    full.warmup_bars, full.n_bars
                ),
            },
            "ids": {},
        });
    }

    let mut pairs: Vec<PairOutcome> = Vec::with_capacity(fractions.len());
    let mut windows: Vec<Value> = Vec::with_capacity(fractions.len());
    let mut any_capped = full.events_capped();
    for fraction in &fractions {
        let window_end = truncated_end(start, end, *fraction);
        let outcome = match one_preview(api, params, window_end).await {
            Err(e) => PairOutcome::NotExercised {
                reason: NotExercised::ShortWindowRefused,
                says: format!("the preview refused this window: {}", e.to_tool_error()),
            },
            Ok(v) => match read_run(&v) {
                Err(e) => PairOutcome::NotExercised {
                    reason: NotExercised::ShortWindowRefused,
                    says: format!("the preview refused this window: {e}"),
                },
                Ok(short) => {
                    any_capped |= short.events_capped();
                    compare(&full, &short)
                }
            },
        };
        let mut row = outcome.to_json();
        if let Some(map) = row.as_object_mut() {
            map.insert("fraction".into(), json!(fraction));
            map.insert(
                "end".into(),
                json!(window_end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            );
        }
        windows.push(row);
        pairs.push(outcome);
    }

    let verdict = verdict_over(&pairs);
    let n_exercised = pairs
        .iter()
        .filter(|p| !matches!(p, PairOutcome::NotExercised { .. }))
        .count();
    let firings_compared: u64 = pairs
        .iter()
        .map(|p| match p {
            PairOutcome::PrefixHeld { firings_compared } => *firings_compared,
            _ => 0,
        })
        .sum();

    json!({
        "summary_fields": {
            "verdict": verdict.as_str(),
            "n_exercised": n_exercised,
            "n_windows": fractions.len(),
            "firings_matched": firings_compared,
        },
        "detail": {
            "verdict": verdict.as_str(),
            "n_exercised": n_exercised,
            "n_windows": fractions.len(),
            "firings_matched": firings_compared,
            "windows": windows,
            "full_window": {
                "n_bars": full.n_bars,
                "warmup_bars": full.warmup_bars,
                "bars_evaluated": full.bars_evaluated,
                "total_fires": full.total_fires,
            },
            "events_capped": any_capped,
            "note": "`prefix_held` means the prefix held over the firings counted in \
                     `firings_matched` — it is not a clean bill of health. The previews \
                     evaluate every feature through a window that ends at the decision bar, so \
                     the vectorised leak this test is modelled on cannot arise on this path; \
                     what a break here means is that something depends on the window's length. \
                     When `events_capped` is true only the earliest firings were visible.",
        },
        "ids": {},
    })
}

pub fn specs() -> Vec<ToolSpec> {
    crate::result_shape::declared(vec![
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
                },
                "offset": {"type": "integer", "minimum": 0, "description": "skip this many matches; use next_offset"}
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
    ToolSpec::new(
        "lookahead_truncation_test",
        "strategy",
        Risk::Read,
        "Rerun a draft over shorter windows sharing its start and check each shorter run's \
         signal firings are exactly the prefix of the longer run's. A held prefix is not a \
         clean bill of health: the answer says how many firings it compared, and reports \
         not_exercised when a shorter window was too short to fire.",
        json!({
            "type": "object",
            "required": ["definition", "instrument_id", "start", "end"],
            "properties": {
                "definition": {"type": "object", "description": "the draft preview_signals takes"},
                "instrument_id": {"type": "string"},
                "timeframe": {"type": "string", "default": "1m"},
                "start": {"type": "string", "description": "RFC3339; every run shares it"},
                "end": {"type": "string", "description": "RFC3339; the full window's end"},
                "fractions": {
                    "type": "array",
                    "items": {"type": "number"},
                    "maxItems": 4,
                    "description": "truncated ends as fractions of the span, each in (0,1); default [0.5, 0.75]"
                },
                "project_id": {"type": "string"}
            }
        }),
    )
    .internal()
    // Stated at the declaration rather than decided in the handler, and it does
    // not move with `fractions`: every read goes through `preview_signals`' own
    // route, which writes no ledger row at any count. A trial cost that depended
    // on an argument could not be audited from the catalogue at all.
    .spends_trial(crate::spec::TrialKind::None)
    // Bounds expense rather than trials: one call is up to five preview reads.
    .max_calls_per_run(20)
    .summary(
        "{verdict} — {n_exercised} of {n_windows} windows exercised it, {firings_matched} firings matched",
        &[
            ("verdict", ""),
            ("n_exercised", ""),
            ("n_windows", ""),
            ("firings_matched", ""),
        ],
    ),
    ])
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
        assert_eq!(out["error"]["code"], "unknown_cost");
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
        assert_eq!(
            def.definition_version, "1.3",
            "the exits block is a 1.3 document"
        );
    }

    /// **What the language compiles, the platform accepts.**
    ///
    /// This replaces `the_only_thing_the_validator_holds_against_a_compiled_strategy_is_its_version`,
    /// the pin that recorded the gate refusing every document SLv2 could produce.
    /// That test's own doc said the repair on the day the gate opened was to
    /// delete it *and assert `validate` is `Ok`*; deleting it alone leaves the
    /// join untested, which is how the blockage lasted as long as it did.
    ///
    /// It has to live here. `strategy-lang` and `strategy-validator` do not
    /// depend on each other, and `mcp-server-lib` is the only crate that depends
    /// on both — so `the_version_gate_is_a_range` can check the gate against
    /// hand-written version strings, but nothing there can notice the compiler
    /// walking away from what the gate accepts. That is the divergence that
    /// actually happened.
    ///
    /// Asserting `Ok` rather than "no version error" is deliberate: the version
    /// was the only complaint last time, and a narrower assertion would still
    /// pass if the next divergence were an exits rule or a feature name.
    #[test]
    fn a_compiled_strategy_is_accepted_by_the_validator() {
        let def: domain::strategy_def::StrategyDefinition =
            serde_json::from_value(compiled(TREND)).expect("a definition");
        if let Err(errs) = strategy_validator::validate(&def) {
            panic!("the language compiled a document the platform refuses: {errs:#?}");
        }
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
        assert_eq!(out["error"]["code"], "compile_failed");
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
        assert!(d["size"][0]
            .as_str()
            .is_some_and(|s| s.starts_with("size ")));

        // `ema_21` declares a 105-bar warm-up (5N), and the longest wins. The
        // number matters less than that it is the engine's own, not a second
        // arithmetic here.
        let longest = d["warmup_bars"].as_u64().expect("a warm-up");
        assert_eq!(
            longest,
            features::runtime::lookback_bars("ema_21").expect("ema_21 resolves") as u64
        );
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
        assert_eq!(out["error"]["code"], "ambiguous_source");

        let none = explain_strategy(&json!({}));
        assert_eq!(none["error"]["code"], "missing_field");
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
        assert_eq!(out["error"]["code"], "nothing_to_move");
        assert!(out["error"]["fix"].is_string());
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
        assert_eq!(out["error"]["code"], "unreadable_edit");
        assert!(
            out["error"]["fix"]
                .as_str()
                .is_some_and(|f| f.contains("entry.long[0].clause[1]")),
            "the refusal must show the shape that works: {out:#}"
        );
    }

    /// The new tools are declared, and the namespace still routes.
    #[test]
    fn the_language_is_in_the_catalogue() {
        for name in [
            "compile_strategy",
            "explain_strategy",
            "diff_strategies",
            "apply_move",
        ] {
            let spec = crate::spec::find(name)
                .unwrap_or_else(|| panic!("`{name}` is not declared, so nothing can call it"));
            assert_eq!(spec.namespace, "strategy");
            assert_eq!(spec.spends_trial, crate::spec::TrialKind::None);
        }
    }

    // ── lookahead_truncation_test (ADR-D-19) ─────────────────────────────────

    /// One firing on a one-minute lane, so `bar` and `at` agree the way a real
    /// preview's do — the comparison reads both and a fixture where they
    /// disagreed would exercise the wrong branch.
    fn firing(signal: &str, bar: u64, close: f64) -> Firing {
        let base = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        Firing {
            signal: signal.into(),
            bar,
            at: (base + chrono::Duration::minutes(bar as i64))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            close,
        }
    }

    /// A run whose `total_fires` agrees with the firings it carries — the
    /// uncapped case; the capped one is built by hand where it is needed.
    fn run(n_bars: u64, warmup: u64, evaluated: u64, firings: Vec<Firing>) -> RunSignals {
        RunSignals {
            n_bars,
            warmup_bars: warmup,
            bars_evaluated: evaluated,
            total_fires: firings.len() as u64,
            firings,
        }
    }

    /// The property the whole tool turns on: a window that could not exercise
    /// the strategy is never reported as a window that exonerated it.
    ///
    /// A strategy whose warm-up exceeds the shorter window fires nothing there.
    /// Calling that "the prefix held" would be this catalogue's own failure mode.
    #[test]
    fn a_shorter_window_that_could_not_fire_is_not_a_pass() {
        let full = run(500, 40, 460, vec![firing("entry", 50, 10.0)]);
        let short = run(30, 40, 0, vec![]);
        match compare(&full, &short) {
            PairOutcome::NotExercised { reason, says } => {
                assert_eq!(reason, NotExercised::ShortWindowNeverEvaluated);
                assert!(says.contains("40"), "the reason names the warm-up: {says}");
            }
            other => panic!("a warm-up-starved window is not evidence: {other:?}"),
        }
        assert_eq!(
            verdict_over(&[compare(&full, &short)]),
            Verdict::NotExercised
        );
    }

    /// And the roll-up agrees: no exercised window is `not_exercised`, never
    /// `prefix_held`.
    #[test]
    fn a_run_that_tested_nothing_reports_nothing() {
        let pairs = vec![
            PairOutcome::NotExercised {
                reason: NotExercised::ShortWindowNeverEvaluated,
                says: String::new(),
            },
            PairOutcome::NotExercised {
                reason: NotExercised::NoFiringsInOverlap,
                says: String::new(),
            },
        ];
        assert_eq!(verdict_over(&pairs), Verdict::NotExercised);
    }

    /// One break outranks any number of holds. A leak found is the finding.
    #[test]
    fn a_single_break_outranks_every_hold() {
        let pairs = vec![
            PairOutcome::PrefixHeld {
                firings_compared: 9,
            },
            PairOutcome::PrefixBroken {
                kind: BreakKind::MissingFiring,
                at_bar: Some(7),
                says: String::new(),
            },
            PairOutcome::PrefixHeld {
                firings_compared: 4,
            },
        ];
        assert_eq!(verdict_over(&pairs), Verdict::PrefixBroken);
    }

    /// The clean case, and it still carries its `n`: the prefix held over four
    /// firings, not over nothing.
    #[test]
    fn a_held_prefix_reports_how_many_firings_it_compared() {
        let full = run(
            400,
            20,
            380,
            vec![
                firing("entry", 30, 10.0),
                firing("entry", 31, 14.0),
                firing("entry", 41, 11.0),
                firing("entry", 50, 13.0),
                firing("flag", 55, 12.0),
            ],
        );
        let short = run(
            35,
            20,
            15,
            vec![firing("entry", 30, 10.0), firing("entry", 31, 14.0)],
        );
        assert_eq!(
            compare(&full, &short),
            PairOutcome::PrefixHeld {
                firings_compared: 2
            },
            "firings past bar 35 are outside the truncated window and are not expected in it"
        );
    }

    /// The leak this test exists to catch: the shorter run does not reproduce a
    /// firing the longer run made inside the overlap.
    #[test]
    fn a_firing_the_shorter_run_lost_is_a_break() {
        let full = run(
            400,
            20,
            380,
            vec![firing("entry", 30, 10.0), firing("entry", 40, 13.0)],
        );
        let short = run(200, 20, 180, vec![firing("entry", 30, 10.0)]);
        match compare(&full, &short) {
            PairOutcome::PrefixBroken { kind, at_bar, .. } => {
                assert_eq!(kind, BreakKind::MissingFiring);
                assert_eq!(at_bar, Some(40));
            }
            other => panic!("expected a break, got {other:?}"),
        }
    }

    /// And the mirror: the shorter run fires where the longer one did not.
    #[test]
    fn a_firing_the_shorter_run_invented_is_a_break() {
        // Not a count mismatch — the full window fires again past the overlap —
        // so the bar-by-bar check is what has to catch this.
        let full = run(
            400,
            20,
            380,
            vec![firing("entry", 30, 10.0), firing("entry", 250, 20.0)],
        );
        let short = run(
            200,
            20,
            180,
            vec![firing("entry", 30, 10.0), firing("entry", 38, 13.0)],
        );
        match compare(&full, &short) {
            PairOutcome::PrefixBroken { kind, at_bar, .. } => {
                assert_eq!(kind, BreakKind::ExtraFiring);
                assert_eq!(at_bar, Some(38));
            }
            other => panic!("expected a break, got {other:?}"),
        }
    }

    /// Same bar, same instant, different signal.
    #[test]
    fn the_same_bar_firing_a_different_signal_is_a_break() {
        let full = run(400, 20, 380, vec![firing("entry", 30, 10.0)]);
        let short = run(200, 20, 180, vec![firing("exit", 30, 10.0)]);
        match compare(&full, &short) {
            PairOutcome::PrefixBroken { kind, at_bar, .. } => {
                assert_eq!(kind, BreakKind::SignalChanged);
                assert_eq!(at_bar, Some(30));
            }
            other => panic!("expected a break, got {other:?}"),
        }
    }

    /// The false leak this tool most easily could have shipped.
    ///
    /// `preview_signals` emits a bar's firings from a `HashSet` difference, and
    /// `std`'s hasher is seeded per set, so two signals on one bar come back in
    /// an order that differs between two runs of the same code on the same data.
    /// A positional list compare would call that `signal_changed` and report a
    /// leak that is not there — worse than finding nothing.
    #[test]
    fn two_signals_on_one_bar_may_come_back_in_either_order() {
        let full = run(
            400,
            20,
            380,
            vec![firing("entry", 30, 10.0), firing("flag", 30, 10.0)],
        );
        let short = run(
            200,
            20,
            180,
            vec![firing("flag", 30, 10.0), firing("entry", 30, 10.0)],
        );
        assert_eq!(
            compare(&full, &short),
            PairOutcome::PrefixHeld {
                firings_compared: 2
            },
            "order within one bar is not information; order between bars is"
        );
    }

    /// When the event cap cut a bar in half, which of that bar's signals
    /// survived is the same coin flip, so the half-bar is not judged — and the
    /// count of what was compared drops accordingly rather than staying quiet.
    #[test]
    fn a_bar_the_event_cap_cut_in_half_is_not_judged() {
        let mut full = run(
            400,
            20,
            380,
            vec![firing("entry", 10, 1.0), firing("entry", 30, 10.0)],
        );
        full.total_fires = 7;
        let mut short = run(
            200,
            20,
            180,
            vec![firing("entry", 10, 1.0), firing("flag", 30, 10.0)],
        );
        short.total_fires = 7;
        assert_eq!(
            compare(&full, &short),
            PairOutcome::PrefixHeld {
                firings_compared: 1
            },
            "bar 10 is compared; bar 30 is the bar the cap cut and is dropped from both"
        );
    }

    /// And the guard drops only the cut bar. Evidence before it still speaks.
    #[test]
    fn the_cap_guard_drops_the_cut_bar_and_nothing_before_it() {
        let mut full = run(
            400,
            20,
            380,
            vec![firing("entry", 10, 1.0), firing("entry", 30, 10.0)],
        );
        full.total_fires = 7;
        let mut short = run(
            200,
            20,
            180,
            vec![firing("exit", 10, 1.0), firing("flag", 30, 10.0)],
        );
        short.total_fires = 7;
        match compare(&full, &short) {
            PairOutcome::PrefixBroken { kind, at_bar, .. } => {
                assert_eq!(kind, BreakKind::SignalChanged);
                assert_eq!(at_bar, Some(10));
            }
            other => panic!("the bar before the cut is still evidence: {other:?}"),
        }
    }

    /// And the order of blame, pinned: when the bar itself moved, the changed
    /// signal is explained by the data and is not reported as a leak.
    #[test]
    fn a_changed_signal_on_a_changed_bar_is_blamed_on_the_data() {
        let full = run(400, 20, 380, vec![firing("entry", 30, 10.0)]);
        let short = run(200, 20, 180, vec![firing("exit", 30, 10.5)]);
        match compare(&full, &short) {
            PairOutcome::NotExercised { reason, .. } => {
                assert_eq!(
                    reason,
                    NotExercised::SeriesRevised,
                    "a corrected close changes the feature values, so the signal changing with \
                     it is accounted for; reporting a leak here would send the author to fix \
                     the strategy for a data problem"
                );
            }
            other => panic!("data is blamed before the strategy is: {other:?}"),
        }
    }

    /// The check that sees past `sample_events`' cap.
    ///
    /// The positional comparison can only read the earliest firings the preview
    /// returns. The per-signal counts are not capped, and a window contained in
    /// another cannot fire more often than the window containing it.
    #[test]
    fn more_firings_in_the_shorter_window_is_a_break_past_the_event_cap() {
        let mut full = run(400, 20, 380, vec![]);
        full.total_fires = 40;
        let mut short = run(200, 20, 180, vec![]);
        short.total_fires = 41;
        match compare(&full, &short) {
            PairOutcome::PrefixBroken { kind, .. } => {
                assert_eq!(kind, BreakKind::MoreFiringsInShorterWindow);
            }
            other => panic!("expected a break, got {other:?}"),
        }
    }

    /// A truncation that removed no bars compared two copies of one window.
    #[test]
    fn a_truncation_that_removed_no_bars_is_not_a_pass() {
        let full = run(400, 20, 380, vec![firing("entry", 30, 10.0)]);
        let short = run(400, 20, 380, vec![firing("entry", 30, 10.0)]);
        match compare(&full, &short) {
            PairOutcome::NotExercised { reason, .. } => {
                assert_eq!(reason, NotExercised::TruncationRemovedNoBars);
            }
            other => panic!("two identical windows test nothing: {other:?}"),
        }
    }

    /// Both runs quiet over the overlap is silence, not agreement.
    #[test]
    fn two_quiet_runs_over_the_overlap_compared_nothing() {
        let full = run(400, 20, 380, vec![firing("entry", 250, 10.0)]);
        let short = run(200, 20, 180, vec![]);
        match compare(&full, &short) {
            PairOutcome::NotExercised { reason, .. } => {
                assert_eq!(reason, NotExercised::NoFiringsInOverlap);
            }
            other => panic!("expected silence, got {other:?}"),
        }
    }

    /// A bar corrected between the two point-in-time reads is a data finding.
    /// Blaming the strategy for it would send the author to fix the wrong thing.
    #[test]
    fn a_bar_revised_between_the_reads_is_named_as_data_not_lookahead() {
        let full = run(400, 20, 380, vec![firing("entry", 30, 10.0)]);
        let short = run(200, 20, 180, vec![firing("entry", 30, 10.5)]);
        match compare(&full, &short) {
            PairOutcome::NotExercised { reason, .. } => {
                assert_eq!(reason, NotExercised::SeriesRevised);
            }
            other => panic!("a revised close is not a strategy verdict: {other:?}"),
        }
    }

    /// Same instant, different bar index: the series gained or lost a bar before
    /// it, so indices are not comparable and the prefix was not judged.
    #[test]
    fn a_series_that_moved_under_the_two_reads_is_inconclusive() {
        let full = run(400, 20, 380, vec![firing("entry", 30, 10.0)]);
        // The same instant, one bar earlier: a bar before it went missing.
        let mut moved = firing("entry", 29, 10.0);
        moved.at = firing("entry", 30, 10.0).at;
        let short = run(200, 20, 180, vec![moved]);
        match compare(&full, &short) {
            PairOutcome::NotExercised { reason, .. } => {
                assert_eq!(reason, NotExercised::SeriesMisaligned);
            }
            other => panic!("expected inconclusive, got {other:?}"),
        }
    }

    /// Warm-up is derived from the definition alone, so two runs of one
    /// definition disagreeing about it means they did not run the same strategy.
    #[test]
    fn a_warmup_that_changed_between_runs_is_a_break() {
        let full = run(400, 20, 380, vec![]);
        let short = run(200, 21, 180, vec![]);
        match compare(&full, &short) {
            PairOutcome::PrefixBroken { kind, .. } => {
                assert_eq!(kind, BreakKind::WarmupChanged);
            }
            other => panic!("expected a break, got {other:?}"),
        }
    }

    /// Adversarial on the reader: a malformed event is an error, never a
    /// silently dropped one. A dropped event reads as a missing firing, which
    /// this tool would then report as a leak.
    #[test]
    fn a_malformed_event_is_an_error_not_a_dropped_firing() {
        let v = json!({
            "n_bars": 100, "warmup_bars": 5, "bars_evaluated": 95,
            "signals": [{"signal": "entry", "fires": 2}],
            "sample_events": [
                {"signal": "entry", "bar": 10, "at": "2025-01-01T00:10:00Z", "close": 1.0},
                {"signal": "entry", "at": "2025-01-01T00:20:00Z", "close": 1.0}
            ]
        });
        let err = read_run(&v).unwrap_err();
        assert_eq!(err["error"]["code"], "unreadable_preview");
    }

    /// The cap is read off the answer rather than copied from the preview's own
    /// private constant, which would drift the first time it changed.
    #[test]
    fn the_event_cap_is_detected_from_the_answer_itself() {
        let v = json!({
            "n_bars": 100, "warmup_bars": 5, "bars_evaluated": 95,
            "signals": [{"signal": "entry", "fires": 40}],
            "sample_events": [
                {"signal": "entry", "bar": 10, "at": "2025-01-01T00:10:00Z", "close": 1.0}
            ]
        });
        let r = read_run(&v).unwrap();
        assert_eq!(r.total_fires, 40);
        assert!(r.events_capped(), "40 firings counted, 1 event returned");
    }

    /// A preview that refused is propagated, not read as an empty run.
    #[test]
    fn a_refused_preview_is_not_an_empty_run() {
        let v = json!({"error": "window_too_short", "have": 4, "need": 30});
        assert!(read_run(&v).is_err());
    }

    /// Bounded and checked, not clamped: 0 and 1 are not shorter windows.
    #[test]
    fn a_fraction_outside_the_open_interval_is_refused() {
        for bad in [
            json!([0.0]),
            json!([1.0]),
            json!([1.5]),
            json!([-0.2]),
            json!(["half"]),
        ] {
            assert!(
                read_fractions(&json!({ "fractions": bad })).is_err(),
                "{bad} was accepted"
            );
        }
        assert!(read_fractions(&json!({"fractions": []})).is_err());
        assert!(read_fractions(&json!({"fractions": [0.1, 0.2, 0.3, 0.4, 0.5]})).is_err());
        assert!(read_fractions(&json!({"fractions": [0.5, 0.5]})).is_err());
        assert!(read_fractions(&json!({"fractions": "0.5"})).is_err());
    }

    #[test]
    fn fractions_default_and_sort_ascending() {
        assert_eq!(read_fractions(&json!({})).unwrap(), vec![0.5, 0.75]);
        assert_eq!(
            read_fractions(&json!({"fractions": [0.8, 0.25]})).unwrap(),
            vec![0.25, 0.8]
        );
    }

    #[test]
    fn a_fraction_is_a_fraction_of_the_span() {
        let start = chrono::DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let end = chrono::DateTime::parse_from_rfc3339("2025-01-11T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            truncated_end(start, end, 0.5).to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "2025-01-06T00:00:00Z"
        );
    }

    /// ADR-D-19, as a build-time check rather than a paragraph.
    ///
    /// The truncation test must not ship under the perturbation signature: this
    /// platform has no `shift_bars` mode, and a tool naming one would claim a
    /// computation nothing performs.
    #[test]
    fn the_truncation_test_does_not_claim_a_perturbation_it_cannot_run() {
        let spec = crate::spec::find("lookahead_truncation_test").expect("declared");
        // The runtime mode landed (S-3.10: `backtest::perturb`, the preview's
        // `perturbations` field), so the perturbation tool is declared as its own
        // tool. What must stay true is that the *truncation* tool does not borrow
        // its signature, which the loop below checks.
        assert!(
            crate::spec::find("future_perturbation_test").is_some(),
            "S-3.10 declared future_perturbation_test; ADR-D-19 keeps it a separate tool"
        );
        let schema = spec.input_schema.to_string();
        for absent in [
            "shift_bars",
            "shift_column",
            "mask_column",
            "jitter_bars",
            "fill_delay_ms",
            "perturbation_pct",
        ] {
            assert!(
                !schema.contains(absent),
                "the schema names `{absent}`, which nothing in this workspace implements"
            );
        }
    }

    /// The trial cost is a property of the declaration, not of the arguments.
    ///
    /// One call makes between two and five preview reads depending on
    /// `fractions`, and every one of them goes through a route that writes no
    /// ledger row — so the declared cost is the same at either end, which is
    /// what makes it auditable from the catalogue.
    #[test]
    fn the_declared_trial_cost_does_not_move_with_the_arguments() {
        let spec = crate::spec::find("lookahead_truncation_test").expect("declared");
        assert_eq!(spec.spends_trial, crate::spec::TrialKind::None);
        assert_eq!(spec.risk, Risk::Read);
        assert_eq!(spec.namespace, "strategy");
        assert_eq!(spec.envelope, crate::spec::Envelope::None);
        assert!(
            spec.max_calls_per_run.is_some(),
            "a read that fans out to five HTTP reads still costs money, and §2.4b's bound is \
             the only thing that prices it"
        );
    }

    /// The description says the thing that stops the tool being misread.
    #[test]
    fn the_description_refuses_to_promise_a_clean_bill_of_health() {
        let spec = crate::spec::find("lookahead_truncation_test").expect("declared");
        assert!(
            spec.description.contains("not a clean bill of health"),
            "a model reading only the catalogue must not learn that a held prefix clears the \
             strategy: {}",
            spec.description
        );
    }
}
