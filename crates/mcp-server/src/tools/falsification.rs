//! Falsification tools: the agent's primary output surface
//! (EXTERNAL-REVIEW-2026-09 §4; ADR-P6-05, ADR-P6-06, ADR-P6-03).
//!
//! Four tools, and the ordering between two of them is the whole design.
//!
//! - `report_evaluator_defect` — the platform itself is wrong. Scores **above**
//!   a promotion. Without that ordering an agent that discovers the simulator
//!   lets it buy the intrabar low will exploit it silently, because reporting it
//!   would destroy its best result.
//! - `report_kill` — a candidate is refuted. Dense reward: most strategies are
//!   noise, so this fires constantly and is usually right.
//! - `check_tradeable_frontier` — is this horizon searchable at all, before a
//!   trial is spent finding out.
//! - `run_oracle_test` — is the cost model itself sane.
//!
//! Every description here tells the model what does **not** count. A kill is a
//! reproducible defect; "the Sharpe was low" is a result, and the server refuses
//! it at the constructor rather than scoring it zero after the fact.

use serde_json::{json, Value};

use crate::ApiClient;

fn missing(field: &str) -> Value {
    json!({ "error": "missing_field", "field": field })
}

fn str_field(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn steps_field(params: &Value) -> Vec<String> {
    params
        .get("steps")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Shared pre-flight on the reproduction, so the agent gets the *reason* rather
/// than a 422 from the database.
///
/// The server checks again — this is a better error message, not a security
/// boundary, and a check that only exists here is a check an HTTP client skips.
fn check_reproduction(params: &Value) -> Result<(), Value> {
    for k in ["artifact_ref", "expected", "observed"] {
        if str_field(params, k).is_none() {
            return Err(missing(k));
        }
    }
    if params.get("seed").and_then(Value::as_u64).is_none() {
        return Err(json!({
            "error": "missing_field",
            "field": "seed",
            "fix": "a replay that cannot be pinned to a seed is an anecdote"
        }));
    }
    if steps_field(params).is_empty() {
        return Err(json!({
            "error": "no_steps",
            "fix": "list the steps to reproduce. A report a human cannot re-run in ten \
                    minutes scores zero, and one with no steps cannot be re-run at all."
        }));
    }
    let expected = str_field(params, "expected").unwrap_or_default();
    let observed = str_field(params, "observed").unwrap_or_default();
    if expected == observed {
        return Err(json!({
            "error": "not_falsifiable",
            "fix": "`expected` and `observed` are the same, so nothing was refuted. State what \
                    you predicted would happen and what happened instead. A low score is not a \
                    kill: 'Sharpe is only 0.4' is a result, not a defect."
        }));
    }
    Ok(())
}

async fn post(api: &ApiClient, path: &str, body: Value) -> Value {
    match api.post(path, body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// `report_kill` — refute a candidate.
pub async fn report_kill(api: &ApiClient, params: &Value) -> Value {
    if let Err(e) = check_reproduction(params) {
        return e;
    }
    let Some(trial_id) = str_field(params, "target_trial_id") else {
        return missing("target_trial_id");
    };
    let Some(defect_class) = str_field(params, "defect_class") else {
        return missing("defect_class");
    };

    post(
        api,
        "/api/falsification/kills",
        json!({
            "target_trial_id": trial_id,
            "defect_class": defect_class,
            "artifact_ref": str_field(params, "artifact_ref"),
            "seed": params.get("seed").and_then(Value::as_u64),
            "expected": str_field(params, "expected"),
            "observed": str_field(params, "observed"),
            "steps": steps_field(params),
        }),
    )
    .await
}

/// `report_evaluator_defect` — the platform is wrong.
pub async fn report_evaluator_defect(api: &ApiClient, params: &Value) -> Value {
    if let Err(e) = check_reproduction(params) {
        return e;
    }
    let Some(component) = str_field(params, "component") else {
        return missing("component");
    };
    let Some(guarantee) = str_field(params, "claimed_guarantee") else {
        return json!({
            "error": "missing_field",
            "field": "claimed_guarantee",
            "fix": "quote what this component claims to guarantee, from the spec or its own \
                    docs. A defect is a gap between a claim and a behaviour; without the claim \
                    there is only a behaviour, and behaviours are not defects."
        });
    };

    post(
        api,
        "/api/falsification/defects",
        json!({
            "component": component,
            "claimed_guarantee": guarantee,
            "artifact_ref": str_field(params, "artifact_ref"),
            "seed": params.get("seed").and_then(Value::as_u64),
            "expected": str_field(params, "expected"),
            "observed": str_field(params, "observed"),
            "steps": steps_field(params),
        }),
    )
    .await
}

/// `check_tradeable_frontier` — may this horizon be searched at all.
pub async fn check_tradeable_frontier(api: &ApiClient, params: &Value) -> Value {
    let Some(instrument_id) = str_field(params, "instrument_id") else {
        return missing("instrument_id");
    };
    let Some(holding_bars) = params.get("holding_bars").and_then(Value::as_u64) else {
        return missing("holding_bars");
    };

    post(
        api,
        "/api/research/tradeable-frontier",
        json!({
            "instrument_id": instrument_id,
            "holding_bars": holding_bars,
            "timeframe": str_field(params, "timeframe").unwrap_or_else(|| "1m".into()),
            "venue": str_field(params, "venue"),
            "fee_tier": str_field(params, "fee_tier"),
            "execution_style": str_field(params, "execution_style")
                .unwrap_or_else(|| "taker_both".into()),
            "start": str_field(params, "start"),
            "end": str_field(params, "end"),
        }),
    )
    .await
}

/// `run_oracle_test` — is the cost model charging what it claims.
pub async fn run_oracle_test(api: &ApiClient, params: &Value) -> Value {
    let Some(instrument_id) = str_field(params, "instrument_id") else {
        return missing("instrument_id");
    };
    post(
        api,
        "/api/research/oracle-test",
        json!({
            "instrument_id": instrument_id,
            "timeframe": str_field(params, "timeframe").unwrap_or_else(|| "1m".into()),
            "venue": str_field(params, "venue"),
            "fee_tier": str_field(params, "fee_tier"),
            "execution_style": str_field(params, "execution_style")
                .unwrap_or_else(|| "taker_both".into()),
            "start": str_field(params, "start"),
            "end": str_field(params, "end"),
        }),
    )
    .await
}

// ── evidence (ADR-P7-05) ─────────────────────────────────────────────────────

/// `get_trade_list` — the fills, with concentration measured.
pub async fn get_trade_list(api: &ApiClient, params: &Value) -> Value {
    let Some(run_id) = str_field(params, "run_id") else {
        return missing("run_id");
    };
    post(
        api,
        "/api/evidence/trades",
        json!({
            "run_id": run_id,
            "top_n": params.get("top_n").and_then(Value::as_u64).unwrap_or(6),
        }),
    )
    .await
}

/// `probe_data_quality` — scan bars for the defects that make a backtest lie.
pub async fn probe_data_quality(api: &ApiClient, params: &Value) -> Value {
    let Some(instrument_id) = str_field(params, "instrument_id") else {
        return missing("instrument_id");
    };
    post(
        api,
        "/api/evidence/data-quality",
        json!({
            "instrument_id": instrument_id,
            "timeframe": str_field(params, "timeframe").unwrap_or_else(|| "1m".into()),
            "start": str_field(params, "start"),
            "end": str_field(params, "end"),
        }),
    )
    .await
}

/// `find_similar_trials` — what the ledger already knows.
pub async fn find_similar_trials(api: &ApiClient, params: &Value) -> Value {
    if str_field(params, "config_hash").is_none() && str_field(params, "experiment_id").is_none() {
        return json!({
            "error": "no_selector",
            "fix": "supply `config_hash` to find exact repeats of a configuration, or                     `experiment_id` to find neighbours in the same line of work"
        });
    }
    post(
        api,
        "/api/evidence/similar-trials",
        json!({
            "config_hash": str_field(params, "config_hash"),
            "experiment_id": str_field(params, "experiment_id"),
            "limit": params.get("limit").and_then(Value::as_u64).unwrap_or(20),
        }),
    )
    .await
}

/// One declaration per tool (AGENT-006 §2.3; Set S tasks S-1.1, S-1.4).
///
/// Filing a report is a `Write`: it appends a row nothing can edit afterwards.
/// It is emphatically **not** `Destructive` — an agent that hesitates to report a
/// defect because the tool is flagged dangerous is an agent that exploits the
/// defect instead, which is the failure the whole mechanism exists to prevent.
///
/// The two reports declare `TrialKind::Falsification` because
/// `audit.selection_bias_verdict` folds an attack on a strategy into N_eff
/// differently from a search for one. `check_tradeable_frontier` and
/// `run_oracle_test` only measure and spend nothing; that is the point of asking
/// them *before* a search rather than after it disappoints.
#[must_use]
pub fn specs() -> Vec<crate::spec::ToolSpec> {
    use crate::spec::{ToolSpec, TrialKind};
    use crate::taxonomy::Risk;

    // The five fields a `Reproduction` needs. Shared, because a refutation that
    // cannot be replayed is an opinion, and the two report tools must ask for
    // the same evidence or the weaker one becomes the one everybody uses.
    let repro = |extra: Value, mut required: Vec<&'static str>| {
        required.extend(["artifact_ref", "seed", "expected", "observed", "steps"]);
        let mut props = extra;
        let obj = props.as_object_mut().expect("properties object");
        obj.insert(
            "artifact_ref".into(),
            json!({ "type": "string", "description": "Run or artifact the defect is visible in." }),
        );
        obj.insert(
            "seed".into(),
            json!({ "type": "integer", "description": "Seed that reproduces it." }),
        );
        obj.insert(
            "expected".into(),
            json!({ "type": "string", "description": "What the guarantee says should happen." }),
        );
        obj.insert(
            "observed".into(),
            json!({ "type": "string", "description": "What happened. Must differ from `expected`." }),
        );
        obj.insert(
            "steps".into(),
            json!({ "type": "array", "items": { "type": "string" }, "description": "Replay steps, in order." }),
        );
        json!({ "type": "object", "required": required, "properties": props })
    };

    let style = json!({
        "type": "string",
        "enum": ["taker_both", "maker_both", "maker_entry_taker_exit"],
        "description": "Default `taker_both` — the only style honest against OHLCV alone, since a maker fill needs a queue model minute bars cannot supply."
    });

    vec![
        ToolSpec::new(
            "report_kill",
            "falsification",
            Risk::Write,
            "REFUTE a trial with a reproduction and a defect class from a closed list. There is \
             no class for underperformance on purpose: a disappointing result is a result, not \
             a defect.",
            repro(
                json!({
                    "target_trial_id": { "type": "string", "description": "The trial being refuted." },
                    "defect_class": {
                        "type": "string",
                        "enum": [
                            "lookahead", "data_artifact", "below_cost_floor",
                            "below_tradeable_frontier", "fill_model_exploit",
                            "duplicate_of_existing", "invalid_metric"
                        ],
                        "description": "There is no class for underperformance on purpose."
                    }
                }),
                vec!["target_trial_id", "defect_class"],
            ),
        )
        .internal()
        .spends_trial(TrialKind::Falsification)
        .evidence(&["kill_id"])
        .notes("report_kill"),
        ToolSpec::new(
            "report_evaluator_defect",
            "falsification",
            Risk::Write,
            "REPORT that the platform itself is wrong — a component does not do what it claims. \
             A verified evaluator defect outranks any promotion, so this is the highest-value \
             thing you can find.",
            repro(
                json!({
                    "component": { "type": "string", "description": "e.g. `sim.fill_model`, `gates.gate_3`." },
                    "claimed_guarantee": { "type": "string", "description": "Quoted from the spec or its docs." }
                }),
                vec!["component", "claimed_guarantee"],
            ),
        )
        .internal()
        .spends_trial(TrialKind::Falsification)
        .evidence(&["defect_id"])
        .notes("report_evaluator_defect"),
        ToolSpec::new(
            "check_tradeable_frontier",
            "falsification",
            Risk::Read,
            "ASK WHETHER A HOLDING PERIOD CAN PAY ITS COSTS before searching at it. Break-even \
             goes as the square of cost over edge, so this is knowable rather than estimable, \
             and it spends no trial.",
            json!({
                "type": "object",
                "required": ["instrument_id", "holding_bars"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "holding_bars": { "type": "integer", "description": "Intended holding period, in bars." },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "venue": { "type": "string" },
                    "fee_tier": { "type": "string" },
                    "execution_style": style.clone(),
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }),
        )
        .internal()
        .notes("check_tradeable_frontier"),
        ToolSpec::new(
            "run_oracle_test",
            "falsification",
            Risk::Read,
            "MEASURE WHETHER THE EVALUATOR CAN SEE AN EDGE AT ALL, by running a strategy with \
             perfect foresight. If the oracle does not profit, nothing will, and a negative \
             result about your strategy meant nothing.",
            json!({
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "venue": { "type": "string" },
                    "fee_tier": { "type": "string" },
                    "execution_style": style,
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }),
        )
        .internal()
        .notes("run_oracle_test"),
        ToolSpec::new(
            "get_trade_list",
            "falsification",
            Risk::Read,
            "ISOLATE THE TRADES A RESULT RESTS ON. Returns the top-N contributors and what the \
             result becomes without them; a Sharpe that lives in six bars is a defect class, \
             not a strategy.",
            json!({
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": { "type": "string" },
                    "top_n": { "type": "integer", "description": "Default 6." }
                }
            }),
        )
        .internal()
        .evidence(&["run_id"])
        .notes("get_trade_list"),
        ToolSpec::new(
            "probe_data_quality",
            "falsification",
            Risk::Read,
            "ASK WHETHER THE BARS THEMSELVES ARE THE DEFECT over a window — gaps, repeats, \
             frozen prices, impossible moves. Narrow it to the window `get_trade_list` pointed \
             you at.",
            json!({
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }),
        )
        .internal()
        .notes("probe_data_quality"),
        ToolSpec::new(
            "find_similar_trials",
            "falsification",
            Risk::Read,
            "FIND WHETHER THIS HAS BEEN TRIED. An exact config repeat means the earlier result \
             already answers you; a cluster of neighbours means the family's N_eff is larger \
             than its trial count suggests.",
            json!({
                "type": "object",
                "properties": {
                    "config_hash": { "type": "string", "description": "Find exact repeats." },
                    "experiment_id": { "type": "string", "description": "Find neighbours." },
                    "limit": { "type": "integer", "description": "1-200, default 20." }
                }
            }),
        )
        .internal()
        .evidence(&["trial_id"])
        .notes("find_similar_trials"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Value {
        json!({
            "artifact_ref": "run_1",
            "seed": 42,
            "expected": "shifting signals back one bar leaves the Sharpe unchanged",
            "observed": "the Sharpe inverts from +2.9 to -2.7",
            "steps": ["re-run with signal_shift=1", "compare net_sharpe"]
        })
    }

    fn err(params: Value) -> Value {
        check_reproduction(&params).unwrap_err()
    }

    #[test]
    fn a_complete_reproduction_passes_preflight() {
        assert!(check_reproduction(&good()).is_ok());
    }

    /// The degenerate report. The agent is told *why*, in the words that make
    /// the distinction learnable rather than just enforced.
    #[test]
    fn a_report_that_refutes_nothing_is_rejected_with_the_reason() {
        let mut p = good();
        p["expected"] = json!("sharpe is 0.4");
        p["observed"] = json!("sharpe is 0.4");
        let e = err(p);
        assert_eq!(e["error"], "not_falsifiable");
        assert!(e["fix"]
            .as_str()
            .unwrap()
            .contains("A low score is not a kill"));
    }

    #[test]
    fn whitespace_does_not_make_a_report_falsifiable() {
        let mut p = good();
        p["expected"] = json!("  same  ");
        p["observed"] = json!("same");
        assert_eq!(err(p)["error"], "not_falsifiable");
    }

    #[test]
    fn a_report_without_steps_is_rejected() {
        let mut p = good();
        p["steps"] = json!([]);
        assert_eq!(err(p.clone())["error"], "no_steps");
        p["steps"] = json!(["   ", ""]);
        assert_eq!(err(p)["error"], "no_steps");
    }

    #[test]
    fn a_report_without_a_seed_is_rejected_and_says_why() {
        let mut p = good();
        p["seed"] = Value::Null;
        let e = err(p);
        assert_eq!(e["field"], "seed");
        assert!(e["fix"].as_str().unwrap().contains("anecdote"));
    }

    #[test]
    fn every_reproduction_field_is_required() {
        for field in ["artifact_ref", "expected", "observed"] {
            let mut p = good();
            p[field] = json!("  ");
            assert_eq!(err(p)["field"], field);
        }
    }

    // ── the declarations, and the prose that moved out of them ───────────────
    //
    // These seven descriptions used to carry 6.4 KB of guidance between them,
    // and the catalogue is sent on every model call. S-1.4 moved that guidance
    // to `## Tool notes` in the authoring guide, which a model fetches once.
    //
    // The tests moved with it. Each one below asserts the same claim it always
    // did — it just asserts it where the words now live. The point was never
    // that a particular string sat in a particular field; it was that the agent
    // is told the thing. A test that checked only the short description would
    // pass while the substance quietly rotted out of the guide.

    fn notes(tool: &str) -> String {
        // Line endings are not part of the claim. `include_str!` reads the working
        // copy, and a checkout under `core.autocrlf` hands back CRLF whatever the
        // file was committed as, so a helper matching the heading plus a newline
        // reports every section missing on one machine and present on another,
        // from identical content. Normalise first, and the test is about the guide.
        let guide = crate::authoring_guide().replace("\r\n", "\n");
        let heading = format!("### {tool}\n");
        let start = guide
            .find(&heading)
            .unwrap_or_else(|| panic!("`### {tool}` must exist in the authoring guide"));
        let rest = &guide[start + heading.len()..];
        let end = rest.find("\n### ").unwrap_or(rest.len());
        // The guide is hard-wrapped; these assertions are about content, so a
        // claim must not become invisible because it straddled a line break.
        rest[..end].split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn described(tool: &str) -> &'static str {
        specs()
            .into_iter()
            .find(|s| s.name == tool)
            .unwrap_or_else(|| panic!("{tool} must be declared"))
            .description
    }

    #[test]
    fn every_tool_is_declared_with_a_schema() {
        let specs = specs();
        assert_eq!(
            specs.len(),
            7,
            "four falsification tools plus three evidence tools"
        );
        for s in &specs {
            assert!(!s.description.is_empty(), "{}", s.name);
            assert_eq!(s.input_schema["type"], "object");
            // Not every tool has a required field — `find_similar_trials` takes
            // either of two selectors and neither alone is mandatory. What every
            // tool must have is properties, or the model has nothing to fill in.
            assert!(
                !s.input_schema["properties"].as_object().unwrap().is_empty(),
                "{} has no properties",
                s.name
            );
            // Every one of these moved prose out, so every one must point at it.
            assert_eq!(
                s.notes_section,
                Some(s.name),
                "{} moved prose to the guide and must name the section",
                s.name
            );
        }
    }

    /// The ordering has to reach the model, not just the database. A tool that
    /// does not say "scores above a promotion" leaves the agent to infer that
    /// reporting a bug costs it.
    #[test]
    fn the_defect_tool_tells_the_model_it_outranks_a_promotion() {
        let n = notes("report_evaluator_defect");
        assert!(n.contains("ABOVE a promotion"), "{n}");
        assert!(n.contains("exploiting it silently scores less"), "{n}");
        // The short form must carry the ranking too: it is the half a flat-tier
        // model sees without fetching anything.
        assert!(described("report_evaluator_defect").contains("outranks any promotion"));
    }

    /// The kill tool must spell out what does not count, or the model will file
    /// low Sharpes until the constructor stops it.
    #[test]
    fn the_kill_tool_says_what_does_not_count() {
        let n = notes("report_kill");
        assert!(n.contains("Does NOT count"), "{n}");
        assert!(n.contains("Sharpe is only 0.4"), "{n}");
        assert!(described("report_kill").contains("no class for underperformance"));
    }

    /// There is no `underperforms` class, and there must never be one.
    #[test]
    fn the_defect_class_enum_contains_no_performance_threshold() {
        let s = specs();
        let d = s.iter().find(|s| s.name == "report_kill").unwrap();
        let classes = d.input_schema["properties"]["defect_class"]["enum"]
            .as_array()
            .expect("the closed list is in the schema, not only in the guide");
        assert_eq!(classes.len(), 7);
        for c in classes {
            let s = c.as_str().unwrap();
            for banned in ["underperform", "sharpe", "return", "loss", "worse"] {
                assert!(!s.contains(banned), "{s} names a threshold, not a defect");
            }
            // And the guide must name every class the schema accepts, or the
            // model is offered a value with nowhere to read what it means.
            assert!(
                notes("report_kill").contains(s),
                "`{s}` is accepted by the schema and absent from the guide"
            );
        }
    }

    /// The three evidence tools close a hole the falsification sprint left: a
    /// defect vocabulary with no way to gather evidence for it. Each must point
    /// at the next step rather than reading as a verdict on its own.
    #[test]
    fn the_evidence_tools_say_what_they_do_not_prove() {
        let trades = notes("get_trade_list");
        assert!(
            trades.contains("Concentration alone is not a kill"),
            "{trades}"
        );
        assert!(
            trades.contains("probe_data_quality"),
            "it must point at the next step: {trades}"
        );

        let quality = notes("probe_data_quality");
        assert!(quality.contains("NOT a clean bill of health"), "{quality}");
        assert!(quality.contains("ZERO VOLUME"), "{quality}");

        let similar = notes("find_similar_trials");
        assert!(similar.contains("ALREADY BEEN EVALUATED"), "{similar}");
        assert!(
            similar.contains("effective trial count"),
            "the reason repetition matters is N_eff, and the model has to be told: {similar}"
        );
    }

    #[test]
    fn the_frontier_tool_defaults_to_the_only_honest_execution_style() {
        let s = specs();
        let d = s
            .iter()
            .find(|s| s.name == "check_tradeable_frontier")
            .unwrap();
        let style = &d.input_schema["properties"]["execution_style"];
        assert_eq!(style["enum"][0], "taker_both");
        // The *reason* stays in the schema rather than the guide: it is read at
        // the moment the field is filled in, which is the moment it matters.
        assert!(style["description"]
            .as_str()
            .unwrap()
            .contains("queue model"));
    }

    /// The oracle test is about the simulator. A description that reads as a
    /// strategy tool would get it used as one.
    #[test]
    fn the_oracle_tool_says_it_tests_the_simulator_and_what_failure_means() {
        let n = notes("run_oracle_test");
        assert!(n.contains("test of the SIMULATOR"), "{n}");
        assert!(n.contains("must LOSE money"), "{n}");
        assert!(
            n.contains("report_evaluator_defect"),
            "route failure onward: {n}"
        );
        assert!(described("run_oracle_test").contains("EVALUATOR CAN SEE AN EDGE"));
    }
}
