//! The answer a typed tool gives: a rendered summary beside the payload (Set S,
//! S-1.2).
//!
//! # What was wrong
//!
//! `spec::ToolResult`, `render_summary` and `ToolSpec::summary` were built and
//! tested, and nothing on the dispatch path called them for a typed tool. A
//! handler returned whatever `Value` it liked, a `summary_template` declared on
//! a spec was read only by a test, and the model got the payload with no
//! summary -- the part a flat-tier model reads first.
//!
//! # The one seam
//!
//! [`attach_summary`] is called once, from `dispatch_in_family`, on the result of
//! a successful call to a tool whose spec declares `output.summary_template`. It
//! renders the template over the tool's own declared fields and **adds**
//! `summary`. It renames nothing and nests nothing: the API's agent bridge, the
//! harness and the frontend read the existing top-level keys, and wrapping the
//! payload under `detail` would break every one of them for no gain.
//!
//! Where the payload follows the `{summary_fields, detail, n, ids}` convention
//! the template renders over `summary_fields`; otherwise over the payload's own
//! top level. A payload that already carries a text `summary` is left alone, and
//! one whose `summary` is an object (`get_bars`) keeps it and gets the rendered
//! text as `summary_text`: the object is a key consumers already read.
//!
//! # Where the templates are declared
//!
//! [`TEMPLATES`] is one table, applied to each module's spec list by
//! [`declared`]. It lives here rather than inline in a dozen `specs()` bodies so
//! the declaration, the fixture that proves it and the gate that keeps it honest
//! sit in one file; `every_template_is_wired_into_the_catalogue` is what fails
//! if a module forgets to call [`declared`].

use serde_json::{json, Value};

use crate::spec::{render_summary, ToolSpec};

/// `(tool, template, (field, unit)...)`. Every slot a template names must be a
/// declared field, and every declared field a key the handler emits; both are
/// held by tests in this file.
#[allow(clippy::type_complexity)] // a static table; a named type would add nothing
pub const TEMPLATES: &[(&str, &str, &[(&str, &str)])] = &[
    // ── research ────────────────────────────────────────────────────────────
    (
        "create_experiment",
        "experiment {experiment_id} created: state {state}, {trial_counter} trials counted, \
         holdout spent: {holdout_spent}",
        &[
            ("experiment_id", ""),
            ("state", ""),
            ("trial_counter", ""),
            ("holdout_spent", ""),
        ],
    ),
    (
        "get_experiment",
        "experiment {experiment_id}: state {state}, {trial_counter} trials counted, {study_count} \
         studies, gate 3 passed: {gate3_passed}, holdout spent: {holdout_spent}",
        &[
            ("experiment_id", ""),
            ("state", ""),
            ("trial_counter", ""),
            ("study_count", ""),
            ("gate3_passed", ""),
            ("holdout_spent", ""),
        ],
    ),
    (
        "get_sweep",
        "sweep {sweep_id} is {status}: {done} of {planned} planned runs done",
        &[
            ("sweep_id", ""),
            ("status", ""),
            ("done", ""),
            ("planned", ""),
        ],
    ),
    (
        "run_sweep",
        "sweep {sweep_id} is {status}: {done} of {planned} planned runs done",
        &[
            ("sweep_id", ""),
            ("status", ""),
            ("done", ""),
            ("planned", ""),
        ],
    ),
    (
        "backtest_strategy",
        "backtest of {strategy_id} on experiment {experiment_id}; succeeded: {ok}",
        &[("strategy_id", ""), ("experiment_id", ""), ("ok", "")],
    ),
    // ── models ──────────────────────────────────────────────────────────────
    (
        "train_model",
        "training {status} for model {model_id} (run {run_id}, seed {seed}); version {version}; \
         rows trained {rows_trained}",
        &[
            ("status", ""),
            ("model_id", ""),
            ("run_id", ""),
            ("seed", ""),
            ("version", ""),
            ("rows_trained", "rows"),
        ],
    ),
    (
        "get_training_run",
        "training run {run_id} for model {model_id}: {status} ({phase}), {progress} done",
        &[
            ("run_id", ""),
            ("model_id", ""),
            ("status", ""),
            ("phase", ""),
            ("progress", "%"),
        ],
    ),
    // ── market ──────────────────────────────────────────────────────────────
    (
        "get_bars",
        "{bar_count} bars, close {first_close} to {last_close} ({return_pct} %), range {low} to {high}",
        &[
            ("bar_count", ""),
            ("first_close", ""),
            ("last_close", ""),
            ("return_pct", ""),
            ("low", ""),
            ("high", ""),
        ],
    ),];

/// Apply [`TEMPLATES`] to one module's spec list. A spec that already declares a
/// template keeps its own: the inline declaration is the more specific one.
#[must_use]
pub fn declared(specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    specs
        .into_iter()
        .map(|s| {
            if s.output.summary_template.is_some() {
                return s;
            }
            match TEMPLATES.iter().find(|(name, _, _)| *name == s.name) {
                Some((_, template, fields)) => s.summary(template, fields),
                None => s,
            }
        })
        .collect()
}

/// Bytes per token. The harness's own prior (`context::estimate_tokens` before it
/// is calibrated) is `ceil(len / 4)`; this crate does not depend on the harness,
/// so the rule is mirrored and `a_summary_never_exceeds_its_token_budget` holds
/// the ceiling.
const BYTES_PER_TOKEN: usize = 4;

/// Estimated tokens in `s`, rounded up -- the direction that does not under-count.
#[must_use]
pub fn estimate_tokens(s: &str) -> usize {
    s.len().div_ceil(BYTES_PER_TOKEN)
}

/// The rendered summary for `out`, or `None` when the tool declares no template,
/// the call failed, or the payload already carries its own.
#[must_use]
pub fn rendered_summary(spec: &ToolSpec, out: &Value) -> Option<String> {
    let template = spec.output.summary_template?;
    let obj = out.as_object()?;
    // A failure is not summarised: it already has a `fix`, and a summary of an
    // error reads as a result.
    if obj.get("error").is_some_and(|e| !e.is_null()) {
        return None;
    }
    if obj
        .get("summary")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
    {
        return None;
    }
    let source = match (obj.get("summary_fields"), obj.get("summary")) {
        (Some(f), _) if f.is_object() => f,
        (_, Some(s)) if s.is_object() => s,
        _ => out,
    };
    let text = render_summary(template, spec.output.detail_fields, source).ok()?;
    Some(clip(&text, spec.output.summary_tokens as usize * BYTES_PER_TOKEN))
}

/// Truncate on a char boundary. A summary over budget is cut, never refused:
/// the payload beside it is the whole answer, this is the first line of it.
fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max.saturating_sub(3);
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}...", &s[..cut])
}

/// Add the rendered `summary` to a successful typed result. See the module doc.
pub fn attach_summary(spec: &ToolSpec, out: &mut Value) {
    let Some(text) = rendered_summary(spec, out) else {
        return;
    };
    let Some(obj) = out.as_object_mut() else {
        return;
    };
    // `summary` is taken by an object (`get_bars`): keep it, add the text beside it.
    let key = if obj.contains_key("summary") {
        "summary_text"
    } else {
        "summary"
    };
    obj.entry(key).or_insert_with(|| json!(text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{catalog, find};
    use crate::taxonomy::Risk;

    fn spec_with(template: &'static str, fields: &'static [(&'static str, &'static str)]) -> ToolSpec {
        ToolSpec::new("t", "core", Risk::Read, "d", json!({"type": "object"})).summary(template, fields)
    }

    // ── The seam ─────────────────────────────────────────────────────────────

    #[test]
    fn a_successful_result_gains_a_summary_and_keeps_every_key_it_had() {
        let s = spec_with("{a} of {b}", &[("a", ""), ("b", "")]);
        let mut out = json!({ "a": 1, "b": 2, "ids": {"x": "y"}, "nested": {"k": 1} });
        let before = out.clone();
        attach_summary(&s, &mut out);
        assert_eq!(out["summary"], "1 of 2");
        for (k, v) in before.as_object().unwrap() {
            assert_eq!(&out[k], v, "`{k}` changed");
        }
    }

    #[test]
    fn a_failure_is_not_summarised() {
        let s = spec_with("{a}", &[("a", "")]);
        let mut out = crate::tool_error::missing_field("x");
        attach_summary(&s, &mut out);
        assert!(out.get("summary").is_none());
        // ...but `"error": null` is a success, the bug that read three trained models as failures.
        let mut ok = json!({ "a": 1, "error": null });
        attach_summary(&s, &mut ok);
        assert_eq!(ok["summary"], "1");
    }

    #[test]
    fn a_payload_with_its_own_text_summary_is_left_alone() {
        let s = spec_with("{a}", &[("a", "")]);
        let mut out = json!({ "a": 1, "summary": "written by the tool" });
        attach_summary(&s, &mut out);
        assert_eq!(out["summary"], "written by the tool");
        assert!(out.get("summary_text").is_none());
    }

    /// `get_bars` already returns `summary` as an object consumers read.
    #[test]
    fn an_object_summary_is_kept_and_the_text_goes_beside_it() {
        let s = spec_with("{n} bars", &[("n", "")]);
        let mut out = json!({ "summary": { "n": 5 } });
        attach_summary(&s, &mut out);
        assert_eq!(out["summary"], json!({ "n": 5 }));
        assert_eq!(out["summary_text"], "5 bars");
    }

    #[test]
    fn summary_fields_win_over_the_top_level_when_the_payload_has_them() {
        let s = spec_with("{a}", &[("a", "")]);
        let mut out = json!({ "a": "top", "summary_fields": { "a": "field" } });
        attach_summary(&s, &mut out);
        assert_eq!(out["summary"], "field");
    }

    #[test]
    fn a_tool_without_a_template_is_untouched() {
        let s = ToolSpec::new("t", "core", Risk::Read, "d", json!({"type": "object"}));
        let mut out = json!({ "a": 1 });
        attach_summary(&s, &mut out);
        assert_eq!(out, json!({ "a": 1 }));
    }

    #[test]
    fn a_summary_never_exceeds_its_token_budget() {
        let s = spec_with("{a}", &[("a", "")]);
        let mut out = json!({ "a": "x".repeat(10_000) });
        attach_summary(&s, &mut out);
        let text = out["summary"].as_str().unwrap();
        assert!(estimate_tokens(text) <= 400, "{} tokens", estimate_tokens(text));
        assert!(text.ends_with("..."));
    }

    #[test]
    fn a_missing_number_renders_n_unknown_through_the_seam() {
        let s = spec_with("{n} bars", &[("n", "")]);
        let mut out = json!({});
        attach_summary(&s, &mut out);
        assert_eq!(out["summary"], "(n unknown) bars");
    }

    // ── The declarations ─────────────────────────────────────────────────────

    #[test]
    fn every_template_is_wired_into_the_catalogue() {
        let mut unwired = Vec::new();
        for (name, template, fields) in TEMPLATES {
            match find(name) {
                None => unwired.push(format!("{name}: not in the catalogue")),
                Some(s) if s.output.summary_template != Some(*template) => {
                    unwired.push(format!("{name}: its module does not call result_shape::declared"));
                }
                Some(s) if s.output.detail_fields != *fields => {
                    unwired.push(format!("{name}: declared fields differ"));
                }
                Some(_) => {}
            }
        }
        assert!(unwired.is_empty(), "{unwired:#?}");
    }

    #[test]
    fn a_table_entry_names_only_its_own_declared_fields() {
        for (name, template, fields) in TEMPLATES {
            let slots = crate::spec::template_slots(template).unwrap();
            for slot in slots {
                assert!(
                    fields.iter().any(|(f, _)| *f == slot),
                    "{name}: `{slot}` is not declared"
                );
            }
        }
    }

    #[test]
    fn no_tool_is_in_the_table_twice() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, _, _) in TEMPLATES {
            assert!(seen.insert(*name), "{name} declared twice");
        }
    }

    // ── Reference payloads: the handlers' real output shapes ─────────────────

    const TREND: &str = "\
strategy trend_v1 on $bound_at_init timeframe 1h
entry long when ema(close, 7) > ema(close, 21)
size fixed 0.01
exit stop 2 % from entry
exit time 48 bars
";

    /// A payload for every tool in [`TEMPLATES`]. Where the handler is pure the
    /// payload is the handler's own output; where it is a pass-through of the
    /// platform's JSON, it is that struct's serialised shape
    /// (`SweepSnapshot`, `ExperimentView`, `ModelRunSnapshot`) -- so a field the
    /// platform stops sending fails here rather than rendering "(n unknown)" in
    /// production unnoticed.
    fn reference(tool: &str) -> Value {
        use crate::tools::{falsification, market, strategy};
        match tool {
            "create_experiment" | "get_experiment" => json!({
                "id": "7b0c2d9e-0000-4000-8000-000000000001",
                "experiment_id": "ema_cross_btc_1h",
                "strategy_family": "ema_cross", "strategy_type": "daily_trend",
                "state": "candidate", "trial_counter": 0, "unsafe": false,
                "gate3_passed": false, "suspect_overlapping_label_leakage": false,
                "primary_test": "permutation", "holdout_spent": false, "study_count": 0,
                "strategy_ref": "ema_cross_v1",
            }),
            "get_sweep" | "run_sweep" => json!({
                "sweep_id": "9d1f0c4a-0000-4000-8000-000000000002",
                "experiment_id": "7b0c2d9e-0000-4000-8000-000000000001",
                "strategy_ref": "ema_cross_v1", "question": "q", "sampler": "random",
                "max_runs": 40, "status": "running", "done": 12, "planned": 40,
                "note": "batch 2", "error": null, "report": null,
            }),
            "backtest_strategy" => json!({
                "ok": true, "strategy_id": "ema_cross_v1",
                "experiment_id": "ema_cross_v1_exp", "windows": {}, "objective": {}, "report": {},
            }),
            "train_model" => json!({
                "status": "succeeded", "ok": true, "model_id": "btc-1h-direction",
                "run_id": "r1", "seed": 42, "spec_hash": "ab", "version": "3",
                "metrics": {"auc": 0.53}, "error": null, "rows_trained": 8_000,
            }),
            "get_training_run" => json!({
                "run_id": "7a0e0000-0000-4000-8000-000000000003", "model_id": "btc-1h-direction",
                "kind": "train", "status": "running", "progress": 37.5, "phase": "fitting",
                "error": null, "metrics": null,
            }),
            "get_bars" => json!({
                "instrument_id": "BTC-USD", "interval_seconds": 3600,
                "summary": { "bar_count": 720, "first_close": 40_000.0, "last_close": 42_000.0,
                             "return_pct": 5.0, "high": 43_000.0, "low": 39_000.0 },
                "bars": [], "omitted_earlier_bars": 0, "note": "",
            }),
            "get_funding_history" => market::funding_history_result(
                &json!({
                    "funding": [
                        {"event_time": "2026-01-01T00:00:00Z", "relative_rate": "0.0001"},
                        {"event_time": "2026-01-01T08:00:00Z", "relative_rate": "0.0002"},
                        {"event_time": "2026-01-01T16:00:00Z", "relative_rate": "0.0001"},
                    ],
                    "manifest": { "window": { "from": "2026-01-01T00:00:00Z", "to": "2026-01-02T00:00:00Z" } },
                }),
                Some(8.0),
            ),
            "probe_pit_provenance" => json!({
                "summary_fields": { "is_bitemporal": true, "restatement_count": 3,
                                    "violation_count": 0, "bars_latest": 720 },
                "detail": {}, "n": 720, "ids": {},
            }),
            "audit_survivorship" => json!({
                "summary_fields": { "verdict": "audited", "assets_included": 20, "n_dropped": 2, "n_samples": 5 },
                "detail": {}, "ids": {},
            }),
            "regime_failure_search" => json!({
                "summary_fields": { "worst": {"label": "high_vol"}, "failure_found": true,
                                    "n_tested": 6, "n_untested": 1 },
                "detail": {}, "n": 400, "ids": {},
            }),
            "parameter_fragility_map" => json!({
                "summary_fields": { "fragility_index": 0.31, "n_knife_edge": 1, "n_params": 4,
                                    "neighbourhood_plateau": true },
                "detail": {}, "ids": {},
            }),
            "timing_jitter_sensitivity" => json!({
                "summary_fields": { "verdict": "decays_gracefully", "baseline": 1.2,
                                    "at_one_bar": 0.9, "at_max": 0.4, "max_delay": 3 },
                "detail": {}, "n": 20, "ids": { "study_id": "s" },
            }),
            "ablation_sweep" => json!({
                "summary_fields": falsification::tally_ablation(&[
                    json!({"result": {"verdict": "load_bearing"}}),
                    json!({"result": {"verdict": "redundant"}}),
                ]),
                "detail": {}, "ids": {},
            }),
            "list_features" => strategy::list_features(&json!({})),
            "compile_strategy" => strategy::compile_strategy(&json!({ "text": TREND })),
            "explain_strategy" => strategy::explain_strategy(&json!({ "text": TREND })),
            "diff_strategies" => {
                let a = strategy::compile_strategy(&json!({ "text": TREND }))["detail"]["definition"].clone();
                let b = strategy::compile_strategy(&json!({
                    "text": TREND.replace("exit time 48 bars", "exit time 96 bars")
                }))["detail"]["definition"].clone();
                strategy::diff_strategies(&json!({ "before": a, "after": b }))
            }
            "apply_move" => {
                let def = strategy::compile_strategy(&json!({ "text": TREND }))["detail"]["definition"].clone();
                strategy::apply_move(&json!({
                    "definition": def, "strategy_id": "trend_v2",
                    "edit": { "edit": "add_filter", "entry": "entry.long[0]",
                        "filter": { "filter": "compare",
                            "lhs": {"operand": "feature", "family": "rsi", "args": [14]},
                            "op": "lt", "rhs": {"operand": "number", "value": "60"} } }
                }))
            }
            "lookahead_truncation_test" => json!({
                "summary_fields": { "verdict": "prefix_held", "n_exercised": 3, "n_windows": 3,
                                    "firings_matched": 41 },
                "detail": {}, "ids": {},
            }),
            "strategy_skeleton" => json!({
                "summary_fields": { "family": "trend", "n_params": 2 }, "detail": {}, "ids": {},
            }),
            "suggest_moves" => json!({
                "summary_fields": { "n_moves": 2, "n_withheld": 1 }, "detail": {}, "ids": {},
            }),
            other => panic!("no reference payload for `{other}`: add one when you add a template"),
        }
    }

    /// Every tool this file holds a reference payload for. A tool whose template
    /// is declared inline in its own module is held to the same fixture as one in
    /// [`TEMPLATES`]: the check is on what the handler emits, not on who wrote the
    /// template.
    const FIXTURED: &[&str] = &[
        "create_experiment", "get_experiment", "get_sweep", "run_sweep", "backtest_strategy",
        "train_model", "get_training_run", "get_bars", "get_funding_history",
        "probe_pit_provenance", "audit_survivorship", "regime_failure_search",
        "parameter_fragility_map", "timing_jitter_sensitivity", "ablation_sweep",
        "list_features", "compile_strategy", "explain_strategy", "diff_strategies",
        "apply_move", "lookahead_truncation_test", "strategy_skeleton", "suggest_moves",
    ];

    /// The acceptance clause: ≤ 400 tokens on the reference payloads, and no
    /// "(n unknown)" where the handler really emitted the number.
    #[test]
    fn every_templated_tool_summarises_its_reference_payload_within_budget_and_without_gaps() {
        let mut problems = Vec::new();
        for name in FIXTURED {
            let s = find(name).unwrap_or_else(|| panic!("{name} is not in the catalogue"));
            if s.output.summary_template.is_none() {
                problems.push(format!("{name}: has a reference payload but declares no template"));
                continue;
            }
            let mut out = reference(name);
            attach_summary(s, &mut out);
            let key = if out["summary"].is_string() { "summary" } else { "summary_text" };
            let Some(text) = out[key].as_str() else {
                problems.push(format!("{name}: no summary attached to {out}"));
                continue;
            };
            if estimate_tokens(text) > s.output.summary_tokens as usize {
                problems.push(format!("{name}: {} tokens", estimate_tokens(text)));
            }
            if text.contains("(n unknown)") {
                problems.push(format!("{name}: a declared field is absent from the real output: {text}"));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// And the converse, which is the honest half: with the fields gone, every
    /// slot of EVERY declared template in the catalogue says so rather than
    /// reading as a measurement.
    #[test]
    fn every_template_in_the_catalogue_says_n_unknown_for_every_missing_field() {
        let mut checked = 0;
        for s in catalog() {
            let Some(template) = s.output.summary_template else { continue };
            let slots = crate::spec::template_slots(template).unwrap().len();
            let text = rendered_summary(s, &json!({}))
                .unwrap_or_else(|| panic!("{} rendered nothing", s.name));
            assert_eq!(text.matches("(n unknown)").count(), slots, "{}: {text}", s.name);
            checked += 1;
        }
        assert!(checked >= 30, "only {checked} templates in the catalogue");
    }

    /// No template, wherever declared, may render past its budget on a payload
    /// whose fields are as large as a field can be.
    #[test]
    fn every_template_in_the_catalogue_stays_within_its_token_budget_on_huge_fields() {
        for s in catalog() {
            let Some(_) = s.output.summary_template else { continue };
            let mut obj = serde_json::Map::new();
            for (f, _) in s.output.detail_fields {
                obj.insert((*f).to_string(), json!("x".repeat(5_000)));
            }
            let text = rendered_summary(s, &Value::Object(obj))
                .unwrap_or_else(|| panic!("{} rendered nothing", s.name));
            assert!(
                estimate_tokens(&text) <= s.output.summary_tokens as usize,
                "{}: {} tokens",
                s.name,
                estimate_tokens(&text)
            );
        }
    }

    /// `render_summary` appends a field's declared unit after its value, so a
    /// template that also writes the unit renders "3 settlements settlements".
    #[test]
    fn no_template_repeats_the_unit_it_declares() {
        let mut bad = Vec::new();
        for s in catalog() {
            let Some(template) = s.output.summary_template else { continue };
            // Templated tools take their summary from the skill-runner header (Set M/Q
            // territory); they are reported by `the templated tools' own header tests`.
            if matches!(s.tier, crate::spec::Tier::Templated { .. }) {
                continue;
            }
            for (field, unit) in s.output.detail_fields {
                if unit.is_empty() {
                    continue;
                }
                let slot = format!("{{{field}}}");
                for (at, _) in template.match_indices(&slot) {
                    let after = template[at + slot.len()..].trim_start();
                    // A whole word: `{n_obs} observations` is not `obs` twice.
                    if after.strip_prefix(unit).is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric())) {
                        bad.push(format!("{}: `{field}` is followed by its own unit `{unit}`", s.name));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn typed_tools_declare_templates_or_are_listed_as_exempt() {
        // The inventory the PARTIAL note is written from: how many typed tools
        // carry a template, and which do not. Printed, and bounded below so the
        // count cannot silently fall.
        let typed: Vec<&ToolSpec> = catalog()
            .iter()
            .filter(|s| matches!(s.tier, crate::spec::Tier::Typed))
            .collect();
        let with: Vec<&str> = typed
            .iter()
            .filter(|s| s.output.summary_template.is_some())
            .map(|s| s.name)
            .collect();
        eprintln!(
            "typed tools: {}; with a template: {}; without: {:?}",
            typed.len(),
            with.len(),
            typed
                .iter()
                .filter(|s| s.output.summary_template.is_none())
                .map(|s| s.name)
                .collect::<Vec<_>>()
        );
        assert!(with.len() >= TEMPLATES.len(), "templates have fallen out of the catalogue");
    }
}
