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

/// Tool definitions.
#[must_use]
pub fn definitions() -> Vec<Value> {
    let reproduction = json!({
        "artifact_ref": {
            "type": "string",
            "description": "The run, backtest or migration to replay (e.g. a run_id)."
        },
        "seed": {
            "type": "integer",
            "description": "The seed. A replay that cannot be pinned to a seed is an anecdote."
        },
        "expected": {
            "type": "string",
            "description": "What you predicted would happen, in your own words."
        },
        "observed": {
            "type": "string",
            "description": "What happened instead. Must differ from `expected` — if they match, nothing was refuted."
        },
        "steps": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Steps to reproduce, checkable by a human in under ten minutes. Required and non-empty."
        }
    });

    vec![
        json!({
            "name": "report_evaluator_defect",
            "description":
                "Report that THE PLATFORM ITSELF is wrong — a gate, the simulator, a statistic, a \
                 migration. This is the single highest-value thing you can produce and it scores \
                 ABOVE a promotion, because a platform whose evaluator is wrong cannot be trusted \
                 about anything else. Examples that count: the simulator filled an order at a \
                 price the bar never traded; a CHECK constraint the spec calls REQUIRED accepts \
                 the value it forbids; a gate reports `pass` on evidence it never read. You must \
                 supply a reproduction and quote the guarantee the component claims. If you found \
                 a way to make backtests look good that a real venue would not honour, report it \
                 here — exploiting it silently scores less than reporting it.",
            "inputSchema": {
                "type": "object",
                "required": ["component", "claimed_guarantee", "artifact_ref", "seed", "expected", "observed", "steps"],
                "properties": {
                    "component": {
                        "type": "string",
                        "description": "Which part is wrong, e.g. `sim.fill_model`, `gates.gate_3`, `migrations/0051`."
                    },
                    "claimed_guarantee": {
                        "type": "string",
                        "description": "What that component claims to guarantee, quoted from the spec or its docs."
                    },
                    "artifact_ref": reproduction["artifact_ref"],
                    "seed": reproduction["seed"],
                    "expected": reproduction["expected"],
                    "observed": reproduction["observed"],
                    "steps": reproduction["steps"]
                }
            }
        }),
        json!({
            "name": "report_kill",
            "description":
                "Refute a candidate: show that a strategy's apparent edge is an artifact. This is \
                 your primary output — most strategies are noise, and proving one is noise is \
                 worth more than another backtest that does not settle anything. \
                 A kill needs a SPECIFIC REPRODUCIBLE DEFECT, not a low score. \
                 Counts: 'returns invert when signals are shifted one bar' (lookahead); '94% of \
                 P&L is six bars in a window whose OHLCV shows a 40% wick with no supporting \
                 prints' (data_artifact); 'break-even cost is 1.8bp against a 20bp fee floor' \
                 (below_cost_floor). \
                 Does NOT count: 'Sharpe is only 0.4', 'it underperforms buy-and-hold', 'the \
                 drawdown is large'. Those are results. A report whose `expected` matches its \
                 `observed` is rejected, and a kill nobody can reproduce scores zero.",
            "inputSchema": {
                "type": "object",
                "required": ["target_trial_id", "defect_class", "artifact_ref", "seed", "expected", "observed", "steps"],
                "properties": {
                    "target_trial_id": { "type": "string", "description": "The trial being refuted." },
                    "defect_class": {
                        "type": "string",
                        "enum": [
                            "lookahead", "data_artifact", "below_cost_floor",
                            "below_tradeable_frontier", "fill_model_exploit",
                            "duplicate_of_existing", "invalid_metric"
                        ],
                        "description": "The defect, from a closed list. There is no class for 'underperforms' on purpose."
                    },
                    "artifact_ref": reproduction["artifact_ref"],
                    "seed": reproduction["seed"],
                    "expected": reproduction["expected"],
                    "observed": reproduction["observed"],
                    "steps": reproduction["steps"]
                }
            }
        }),
        json!({
            "name": "check_tradeable_frontier",
            "description":
                "Ask whether a holding period is searchable at all on this instrument, BEFORE \
                 spending a trial on it. Measures the expected absolute move over the horizon \
                 against the round-trip cost at the given fee tier. Below the frontier the \
                 expected move is smaller than the cost of capturing it, so no signal of any \
                 quality is profitable there and a positive backtest is a statement about the \
                 backtest. Returns the whole E|Δp| curve, not just the verdict, so you can see \
                 how much room a horizon has rather than only whether it cleared. \
                 Call this before designing a search, not after it disappoints.",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id", "holding_bars"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "holding_bars": { "type": "integer", "description": "Intended holding period, in bars of `timeframe`." },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "venue": { "type": "string", "description": "Fee schedule to price against. Defaults to the instrument's venue." },
                    "fee_tier": { "type": "string", "description": "Tier name at that venue. Defaults to the account's tier." },
                    "execution_style": {
                        "type": "string",
                        "enum": ["taker_both", "maker_both", "maker_entry_taker_exit"],
                        "description": "Default `taker_both` — the only style honest against OHLCV alone, since a maker fill needs a queue model minute bars cannot supply."
                    },
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "run_oracle_test",
            "description":
                "Run a PERFECT-FORESIGHT strategy — one that knows the next bar's close — through \
                 the cost model. This is a test of the SIMULATOR, not of a strategy. At short \
                 horizons an always-in oracle must LOSE money: it earns the mean absolute move \
                 (σ√(2/π) ≈ 5.6bp at σ=7bp) and pays the round trip (≈20bp). If it turns a \
                 profit, the cost model is not charging what it claims and every backtest that \
                 passed through it is suspect — report that with report_evaluator_defect. \
                 Also returns how often a selective oracle could trade profitably, which is the \
                 ceiling on how often ANY strategy can at this horizon.",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "venue": { "type": "string" },
                    "fee_tier": { "type": "string" },
                    "execution_style": {
                        "type": "string",
                        "enum": ["taker_both", "maker_both", "maker_entry_taker_exit"]
                    },
                    "start": { "type": "string" },
                    "end": { "type": "string" }
                }
            }
        }),
        json!({
            "name": "get_trade_list",
            "description":
                "Every fill from a backtest, WITH its P&L concentration already measured.                  This is how you gather evidence for a `data_artifact` kill: it reports what                  fraction of the gross gain sits in the top few trades, and what the net becomes                  without them. A strategy whose edge disappears when you remove its best six                  bars did not have an edge — it had those six bars.                  Concentration alone is not a kill. It tells you WHICH bars to look at;                  probe_data_quality tells you whether those bars are real. You need both.",
            "inputSchema": {
                "type": "object",
                "required": ["run_id"],
                "properties": {
                    "run_id": { "type": "string" },
                    "top_n": { "type": "integer", "description": "How many top trades to isolate. Default 6." }
                }
            }
        }),
        json!({
            "name": "probe_data_quality",
            "description":
                "Scan the underlying bars for defects that make a backtest lie: impossible bars                  (a close outside its own range), long wicks on ZERO VOLUME (a price nothing                  traded at, which the simulator will happily fill against), stale republished                  bars, and spikes that fully revert next bar. Returns timestamps you can quote                  in a reproduction.                  An empty result is NOT a clean bill of health — it means none of those four                  things was found, and a plausible-looking wrong price passes all four.",
            "inputSchema": {
                "type": "object",
                "required": ["instrument_id"],
                "properties": {
                    "instrument_id": { "type": "string" },
                    "timeframe": { "type": "string", "description": "Default `1m`." },
                    "start": { "type": "string" },
                    "end": { "type": "string", "description": "Narrow this to the window get_trade_list pointed you at." }
                }
            }
        }),
        json!({
            "name": "find_similar_trials",
            "description":
                "Search the trial ledger before proposing something. An exact `config_hash`                  match means THIS CONFIGURATION HAS ALREADY BEEN EVALUATED: re-running it                  produces no new market evidence, still spends trial budget, and inflates the                  effective trial count that every selection-adjusted statistic depends on.                  `experiment_id` finds neighbours — what has already been tried in this line of                  work. Use it to avoid rediscovering your own last session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "config_hash": { "type": "string", "description": "Find exact repeats." },
                    "experiment_id": { "type": "string", "description": "Find neighbours." },
                    "limit": { "type": "integer", "description": "1–200, default 20." }
                }
            }
        }),
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
        assert!(e["fix"].as_str().unwrap().contains("A low score is not a kill"));
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

    // ── the definitions ──────────────────────────────────────────────────────

    #[test]
    fn every_tool_is_defined_with_a_schema() {
        let defs = definitions();
        assert_eq!(defs.len(), 7, "four falsification tools plus three evidence tools");
        for d in &defs {
            assert!(d["name"].is_string());
            assert!(d["description"].as_str().unwrap().len() > 120, "{}", d["name"]);
            assert_eq!(d["inputSchema"]["type"], "object");
            // Not every tool has a required field — `find_similar_trials` takes
            // either of two selectors and neither alone is mandatory. What every
            // tool must have is properties, or the model has nothing to fill in.
            assert!(
                !d["inputSchema"]["properties"].as_object().unwrap().is_empty(),
                "{} has no properties",
                d["name"]
            );
        }
    }

    /// The ordering has to reach the model, not just the database. A tool
    /// description that does not say "scores above a promotion" leaves the agent
    /// to infer that reporting a bug costs it.
    #[test]
    fn the_defect_tool_tells_the_model_it_outranks_a_promotion() {
        let defs = definitions();
        let d = defs.iter().find(|d| d["name"] == "report_evaluator_defect").unwrap();
        let desc = d["description"].as_str().unwrap();
        assert!(desc.contains("ABOVE a promotion"), "{desc}");
        assert!(desc.contains("exploiting it silently scores less"), "{desc}");
    }

    /// The kill tool must spell out what does not count, or the model will file
    /// low Sharpes until the constructor stops it.
    #[test]
    fn the_kill_tool_says_what_does_not_count() {
        let defs = definitions();
        let d = defs.iter().find(|d| d["name"] == "report_kill").unwrap();
        let desc = d["description"].as_str().unwrap();
        assert!(desc.contains("Does NOT count"), "{desc}");
        assert!(desc.contains("Sharpe is only 0.4"), "{desc}");
    }

    /// There is no `underperforms` class, and there must never be one.
    #[test]
    fn the_defect_class_enum_contains_no_performance_threshold() {
        let defs = definitions();
        let d = defs.iter().find(|d| d["name"] == "report_kill").unwrap();
        let classes = d["inputSchema"]["properties"]["defect_class"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(classes.len(), 7);
        for c in classes {
            let s = c.as_str().unwrap();
            for banned in ["underperform", "sharpe", "return", "loss", "worse"] {
                assert!(!s.contains(banned), "{s} names a threshold, not a defect");
            }
        }
    }

    /// The three evidence tools close a hole the falsification sprint left: a
    /// defect vocabulary with no way to gather evidence for it. Each must point
    /// at the next step rather than reading as a verdict on its own.
    #[test]
    fn the_evidence_tools_say_what_they_do_not_prove() {
        let defs = definitions();
        let d = |n: &str| {
            defs.iter()
                .find(|d| d["name"] == n)
                .unwrap_or_else(|| panic!("{n} must be defined"))["description"]
                .as_str()
                .unwrap()
                .to_string()
        };

        let trades = d("get_trade_list");
        assert!(trades.contains("Concentration alone is not a kill"), "{trades}");
        assert!(trades.contains("probe_data_quality"), "it must point at the next step: {trades}");

        let quality = d("probe_data_quality");
        assert!(quality.contains("NOT a clean bill of health"), "{quality}");
        assert!(quality.contains("ZERO VOLUME"), "{quality}");

        let similar = d("find_similar_trials");
        assert!(similar.contains("ALREADY BEEN EVALUATED"), "{similar}");
        assert!(
            similar.contains("effective trial count"),
            "the reason repetition matters is N_eff, and the model has to be told: {similar}"
        );
    }

    #[test]
    fn the_frontier_tool_defaults_to_the_only_honest_execution_style() {
        let defs = definitions();
        let d = defs.iter().find(|d| d["name"] == "check_tradeable_frontier").unwrap();
        let style = &d["inputSchema"]["properties"]["execution_style"];
        assert_eq!(style["enum"][0], "taker_both");
        assert!(style["description"].as_str().unwrap().contains("queue model"));
    }

    /// The oracle test is about the simulator. A description that reads as a
    /// strategy tool would get it used as one.
    #[test]
    fn the_oracle_tool_says_it_tests_the_simulator_and_what_failure_means() {
        let defs = definitions();
        let d = defs.iter().find(|d| d["name"] == "run_oracle_test").unwrap();
        let desc = d["description"].as_str().unwrap();
        assert!(desc.contains("test of the SIMULATOR"), "{desc}");
        assert!(desc.contains("must LOSE money"), "{desc}");
        assert!(desc.contains("report_evaluator_defect"), "route failure onward: {desc}");
    }
}
