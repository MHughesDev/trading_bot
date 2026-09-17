//! The replay suites (AGENT-009 §3; Set S tasks S-4.9, S-4.10).
//!
//! # These are not new tests. They are the bugs.
//!
//! Every case in [`all`] is a failure that actually happened, taken from
//! `LOCAL_TIER_FINDINGS.md` §8 and §9, and **every one of them was the harness
//! overruling a model that had done the right thing**. That is the pattern worth
//! internalising: a 7B that looked stupid in a trace was, five times out of five,
//! a 7B being graded against a schema it never saw, or reaching for a tool the
//! exposure budget had just hidden, or being fenced for running out of room.
//!
//! | Case | What happened | `LOCAL_TIER_FINDINGS` |
//! |---|---|---|
//! | [`truncation_is_not_a_grammar_violation`] | a reply cut at `reserve_for_output` was read as proof the grammar was ignored, and a healthy session was fenced | §8.1 |
//! | [`finish_task_ate_the_only_step`] | on `max_steps: 1` the loop did the lookup and had no step left to report it | §8.2 |
//! | [`alphabetical_truncation_hid_the_searched_tool`] | exposure truncated alphabetically, so the tool `search_tools` had just found lost its slot | §8.3 |
//! | [`the_planner_namespace_is_not_trusted_over_the_catalogue`] | the 7B named the right tool and filed it under the wrong namespace | §8.4 |
//! | [`an_opaque_tool_error_is_information_not_the_end`] | `GET /api/instruments/{id}` 500'd and the agent found it by routing correctly | §8.5 |
//! | [`the_evidence_correction_names_what_the_session_produced`] | "cite handles or ids" on a task that produced none: three identical failures, then a fence | §8, "one that is not a bug" |
//! | [`eval_bug_0_of_10`] | **the eval's own bug**: exposure driven off the call count made recovery unsatisfiable | §9 |
//! | [`a_flat_tier_is_graded_against_the_schema_it_was_sent`] | `backtest_ids` sent as a string, validated as an array, rejected three times | §9 |
//! | [`an_unread_idempotent_flag_costs_eleven_steps`] | `list_instruments` called twelve times on data the session already had | §9 |
//!
//! # Why the 0/10 case is the one that matters
//!
//! It scored a healthy tier zero, the trace agreed with the hypothesis, and *a
//! result that confirms what you already believe is the one nobody re-reads*. It
//! nearly became a finding about 7B models. AGENT-009 §8.2 requires it to exist
//! as a passing replay case for exactly that reason: **an eval harness that
//! cannot reproduce its own historical failure is not an eval harness.**

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context::estimate_tokens;
use crate::drive::{Effect, Input, Loop, Note, Outcome};
use crate::hardware::ToolCalling;
use crate::profile::{
    ExposureScope,
    Archetype, Budgets, CodeExecution, ContextBudget, Mode, Orchestration, OutputPolicy, Profile,
    Routing, SchemaStyle, Tier, ToolPolicy, Trifecta,
};
use crate::registry::{Risk, ToolDef, ToolRegistry};

use super::scorecard::Rung;
use super::tape::{Tape, TapeEntry, TapeTask};

// ── Profiles ────────────────────────────────────────────────────────────────

/// Which tier a case runs on. A case that only passes on the frontier has
/// measured the frontier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    /// The dev box: `qwen2.5:7b-instruct` on a 1080 Ti. Where every one of these
    /// bugs was found.
    Degraded7b,
    Frontier,
}

impl ProfileKind {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            ProfileKind::Degraded7b => "qwen2.5-7b-instruct",
            ProfileKind::Frontier => "claude-opus-5",
        }
    }

    #[must_use]
    pub fn profile(self) -> Profile {
        match self {
            ProfileKind::Degraded7b => degraded_7b(),
            ProfileKind::Frontier => frontier(),
        }
    }
}

/// The shape the dev box actually runs.
///
/// **Every number here is copied from `config/profiles/qwen2.5-7b-instruct.yaml`,
/// not chosen.** A fixture that differs from the shipped profile measures a
/// configuration nobody runs — which is the same mistake as an eval run at a
/// different effort (`evals/suites.yaml`'s `defaults.model: production`), one
/// level down. The two that matter most and are easiest to get wrong:
///
/// * `max_exposed_per_step: 6` — **three of those are core**
///   (`search_tools`, `record_finding`, `finish_task`), leaving three work
///   slots. A fixture at 3 leaves *zero*, and every case then fences on a tool
///   selection outside the enum. That is not a harness bug; it is the exposure
///   budget being the grammar, working exactly as designed.
/// * `mode: planner_executor` — the tier plans first, so every tape opens with a
///   plan reply. The §8.4 case is *about* the planner, and a `Freeform` fixture
///   could not express it.
#[must_use]
pub fn degraded_7b() -> Profile {
    Profile {
        model_id: "qwen2.5:7b-instruct".into(),
        provider: "ollama".into(),
        tier: Tier::Degraded,
        archetype: Archetype::ChatAssistant,
        context: ContextBudget {
            advertised_tokens: 32_768,
            effective_budget_tokens: 16_000,
            // Raised from 1000 after §8.1: at 1000 a long `finish_task` was cut
            // mid-string and the loop read it as a grammar violation.
            reserve_for_output: 2_000,
            max_tool_result_bytes: 6_144,
        },
        tools: ToolPolicy {
            max_exposed_per_step: 6,
            schema_style: SchemaStyle::Flat,
            routing: Routing::Dynamic,
            parallel_calls: false,
            exposure_scope: ExposureScope::Step,
            search_reserve_slots: 1,
        },
        output: OutputPolicy {
            constrained_decoding: true,
            native_tool_template: true,
            temperature_tool_calls: 0.0,
            max_retries_per_call: 3,
        },
        orchestration: Orchestration {
            mode: Mode::PlannerExecutor,
            max_steps: 30,
            few_shot_examples: 2,
            reflection: false,
            max_subagent_depth: 1,
            idle_timeout_s: 600,
            max_replans_per_run: 2,
            max_refinements_per_run: 3,
            tool_breaker_threshold: 3,
            max_reflections_per_run: 3,
            code_execution: CodeExecution::Denied,
        },
        trifecta: Trifecta {
            private_data: true,
            untrusted_content: true,
            outbound_channel: false,
            mitigation: "outbound cut: containment is not a function of model tier".into(),
        },
        tool_calling: ToolCalling::MultiStep,
        multi_step_evidence: None,
        requires: None,
        escalate_to: Some("claude-opus-5".into()),
        budgets: Budgets {
            max_usd_per_task: 0.5,
            max_tokens_per_task: 2_000_000,
            // Ignored by the loop; see `Budgets::max_wall_clock_s`.
            max_wall_clock_s: 0,
            ask_above_usd: None,
        },
        notes: "eval fixture, copied from config/profiles/qwen2.5-7b-instruct.yaml".into(),
    }
}

#[must_use]
pub fn frontier() -> Profile {
    Profile {
        model_id: "claude-opus-5".into(),
        provider: "anthropic".into(),
        tier: Tier::Frontier,
        archetype: Archetype::Background,
        context: ContextBudget {
            advertised_tokens: 200_000,
            effective_budget_tokens: 120_000,
            reserve_for_output: 16_000,
            max_tool_result_bytes: 32 * 1024,
        },
        tools: ToolPolicy {
            max_exposed_per_step: 24,
            schema_style: SchemaStyle::Rich,
            routing: Routing::Dynamic,
            parallel_calls: true,
            exposure_scope: ExposureScope::Step,
            search_reserve_slots: 1,
        },
        output: OutputPolicy {
            constrained_decoding: false,
            native_tool_template: true,
            temperature_tool_calls: 0.0,
            max_retries_per_call: 3,
        },
        orchestration: Orchestration {
            mode: Mode::Freeform,
            max_steps: 40,
            few_shot_examples: 0,
            reflection: true,
            max_subagent_depth: 1,
            idle_timeout_s: 600,
            max_replans_per_run: 3,
            max_refinements_per_run: 3,
            tool_breaker_threshold: 3,
            max_reflections_per_run: 3,
            code_execution: CodeExecution::Full,
        },
        trifecta: Trifecta {
            private_data: true,
            untrusted_content: true,
            outbound_channel: false,
            mitigation: "outbound cut: internal-only container network".into(),
        },
        tool_calling: ToolCalling::MultiStep,
        multi_step_evidence: None,
        requires: None,
        escalate_to: None,
        budgets: Budgets {
            max_usd_per_task: 25.0,
            max_tokens_per_task: 20_000_000,
            // Ignored by the loop; see `Budgets::max_wall_clock_s`.
            max_wall_clock_s: 0,
            ask_above_usd: None,
        },
        notes: "eval fixture".into(),
    }
}

/// The catalogue the cases route against.
///
/// Names are chosen so the alphabetical-truncation case is reproducible:
/// `compare_backtests` and `create_strategy` both sort before
/// `list_instruments`, which is exactly the collision §8.3 records.
#[must_use]
pub fn eval_registry() -> ToolRegistry {
    let mut r = ToolRegistry::with_core();
    let tools: &[(&str, &str, &str, Risk, bool, Value)] = &[
        (
            "compare_backtests",
            "backtest",
            "Compare two backtests side by side.",
            Risk::Read,
            true,
            json!({"type":"object","properties":{"backtest_ids":{"type":"array","items":{"type":"string"}}},"required":["backtest_ids"]}),
        ),
        (
            "create_strategy",
            "strategy",
            "Create a strategy from a specification.",
            Risk::Write,
            false,
            json!({"type":"object","properties":{"name":{"type":"string"},"spec":{"type":"object","properties":{"symbol":{"type":"string"}}}},"required":["name"]}),
        ),
        (
            "list_instruments",
            "instrument",
            "List the instruments the platform has data for, with coverage.",
            Risk::Read,
            true,
            json!({"type":"object","properties":{"asset_class":{"type":"string"}}}),
        ),
        (
            "read_bars",
            "data",
            "Read price bars for an instrument. Use when the step needs price history.",
            Risk::Read,
            true,
            json!({"type":"object","properties":{"instrument":{"type":"string"},"limit":{"type":"integer"}},"required":["instrument"]}),
        ),
        (
            "realised_vol",
            "analysis",
            "Compute realised volatility over a window of bars.",
            Risk::Read,
            true,
            json!({"type":"object","properties":{"instrument":{"type":"string"},"window":{"type":"integer"}},"required":["instrument"]}),
        ),
        (
            "run_oracle_test",
            "falsification",
            "Run the oracle test: a candidate that can see the future must be killed.",
            Risk::Read,
            true,
            json!({"type":"object","properties":{"candidate_id":{"type":"string"}},"required":["candidate_id"]}),
        ),
        (
            "leakage_audit",
            "falsification",
            "Audit a candidate's features for look-ahead and other leakage.",
            Risk::Read,
            true,
            json!({"type":"object","properties":{"candidate_id":{"type":"string"}},"required":["candidate_id"]}),
        ),
    ];
    for (name, namespace, description, risk, idempotent, input_schema) in tools {
        r.register(ToolDef {
            name: (*name).to_string(),
            namespace: (*namespace).to_string(),
            description: (*description).to_string(),
            risk: *risk,
            core: false,
            idempotent: *idempotent,
            input_schema: input_schema.clone(),
        })
        .expect("eval catalogue registers");
    }
    r
}

/// A minimal admitted lookup, for the tape tests.
#[must_use]
pub fn lookup_fixture() -> (Profile, ToolRegistry, TapeTask, Tape) {
    let task = TapeTask {
        goal: "which instruments do you have data for".into(),
        charter: "You are a quantitative research assistant. Cite what you read.".into(),
        multi_step: false,
        namespaces: vec!["instrument".into()],
    };
    let tape = vec![
        TapeEntry::Admit,
        TapeEntry::plan(&[("list the instruments with coverage", "list_instruments")]),
        TapeEntry::select("list_instruments"),
        TapeEntry::fill(&json!({})),
        TapeEntry::ToolResult {
            output: "BTC-USD 1m from 2026-03-01, ETH-USD 1h from 2025-01-01".into(),
            source: "platform.instrument".into(),
        },
        TapeEntry::select("finish_task"),
        TapeEntry::fill(&json!({
            "result": "BTC-USD and ETH-USD",
            "evidence": "list_instruments"
        })),
    ];
    (degraded_7b(), eval_registry(), task, tape)
}

// ── Expectations ────────────────────────────────────────────────────────────

/// What a case asserts. A closed vocabulary rather than a closure, so a case is
/// data: it can live in a file, be sealed by hash, and be counted by a
/// scorecard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "expect", rename_all = "snake_case")]
pub enum Expect {
    /// The tool is in the exposed set on the step with this index.
    ExposedAtStep { step: usize, tool: String },
    /// It is not. `read_bars` retired the moment it was tried is the 0/10 bug.
    NotExposedAtStep { step: usize, tool: String },
    Executed { tool: String },
    NotExecuted { tool: String },
    ExecutedAtMost { tool: String, times: usize },
    /// `finished | refused | escalated | fenced | cancelled`.
    Outcome { tag: String },
    /// The loop did not fence. Half the cases here are about a healthy session
    /// being fenced.
    NotFenced,
    Fenced { degradation: String },
    RejectedWith { code: String },
    NotRejectedWith { code: String },
    /// The model was actually told this, in a note or in the next prompt.
    ///
    /// Both, deliberately. Half of what the harness tells a model does not travel
    /// as a `Note` at all — an idempotent repeat and a tool error are pushed into
    /// working state and reach the model in the *next prompt*. Asserting only on
    /// notes would pass a harness that recorded a perfect correction and never
    /// showed it to anyone, and a correction the model cannot see is the exact
    /// failure that fenced a healthy session (LOCAL_TIER_FINDINGS §8, "one that
    /// is not a bug"). Error text is model-facing UX and is asserted like any
    /// other behaviour.
    ModelIsTold { needle: String },
    /// AGENT-005 §2.2 on every model call: system + prompt + reserve ≤ num_ctx.
    BudgetArithmeticHolds,
    /// The whole run took no more than this many steps.
    StepsAtMost { steps: u32 },
}

/// Which AGENT-009 §3 suite a case belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Suite {
    Routing,
    Budget,
    Stopping,
    Honesty,
}

impl Suite {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Suite::Routing => "routing",
            Suite::Budget => "budget",
            Suite::Stopping => "stopping",
            Suite::Honesty => "honesty",
        }
    }
}

/// One replay case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    pub suite: Suite,
    /// Where the failure is written down. A case with no origin is a case
    /// somebody invented, and those are the ones that get tuned against.
    pub origin: String,
    pub profile: ProfileKind,
    /// Which rung a failure of this case is attributed to (AGENT-009 P2).
    /// "The agent failed" is not a finding.
    pub rung: Rung,
    pub task: TapeTask,
    pub tape: Tape,
    pub expect: Vec<Expect>,
}

/// What running a case produced.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseResult {
    pub id: String,
    pub suite: Suite,
    pub profile: ProfileKind,
    pub rung: Rung,
    pub passed: bool,
    /// One line per unmet expectation. Empty on a pass.
    pub failures: Vec<String>,
}

/// Run one case. Deterministic, no model, no network, seconds for the whole
/// suite.
#[must_use]
pub fn run(case: &Case) -> CaseResult {
    let mut l = Loop::new(case.profile.profile(), eval_registry(), case.task.to_task());
    let mut exposures: Vec<BTreeSet<String>> = Vec::new();
    let mut executed: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    // Everything the model was shown: notes plus the prompts they travelled in.
    let mut told: Vec<String> = Vec::new();
    let mut rejections: Vec<String> = Vec::new();
    let mut budget_violations: Vec<String> = Vec::new();
    let mut outcome: Option<Outcome> = None;

    for entry in &case.tape {
        let fx = l.next(entry.to_input());
        for e in &fx {
            match e {
                Effect::CallModel(c) => {
                    // AGENT-005 §2.2, checked on every call rather than sampled.
                    let used = estimate_tokens(&c.system) + estimate_tokens(&c.prompt);
                    if used + c.max_tokens > c.num_ctx {
                        budget_violations.push(format!(
                            "step {}: system+prompt {used} + reserve {} > num_ctx {}",
                            c.step, c.max_tokens, c.num_ctx
                        ));
                    }
                    // What the model was actually shown. A correction that never
                    // reaches a prompt is a correction nobody can act on.
                    told.push(c.prompt.clone());
                }
                Effect::ExecuteTool { name, .. } => executed.push(name.clone()),
                Effect::Note(Note::Exposure { exposed, .. }) => {
                    exposures.push(exposed.iter().cloned().collect());
                }
                Effect::Note(Note::Rejected { code, message, .. }) => {
                    rejections.push((*code).to_string());
                    notes.push((*message).to_string());
                }
                Effect::Note(Note::CoreTool { name, detail }) => {
                    notes.push(format!("{name}: {detail}"));
                }
                Effect::Note(Note::Degraded(d)) => {
                    notes.push(serde_json::to_string(d).unwrap_or_default());
                }
                Effect::Done(o) => outcome = Some(o.clone()),
                _ => {}
            }
        }
        if l.state().is_terminal() {
            break;
        }
    }

    let steps = l.step();
    let mut failures = Vec::new();
    for e in &case.expect {
        match e {
            Expect::ExposedAtStep { step, tool } => {
                let got = exposures.get(*step);
                if !got.is_some_and(|s| s.contains(tool)) {
                    failures.push(format!(
                        "expected `{tool}` exposed at step {step}; exposure was {got:?}"
                    ));
                }
            }
            Expect::NotExposedAtStep { step, tool } => {
                if exposures.get(*step).is_some_and(|s| s.contains(tool)) {
                    failures.push(format!("`{tool}` should not be exposed at step {step}"));
                }
            }
            Expect::Executed { tool } => {
                if !executed.contains(tool) {
                    failures.push(format!("`{tool}` was never executed; ran {executed:?}"));
                }
            }
            Expect::NotExecuted { tool } => {
                if executed.contains(tool) {
                    failures.push(format!("`{tool}` should not have run; ran {executed:?}"));
                }
            }
            Expect::ExecutedAtMost { tool, times } => {
                let n = executed.iter().filter(|t| *t == tool).count();
                if n > *times {
                    failures.push(format!("`{tool}` ran {n} times; the cap is {times}"));
                }
            }
            Expect::Outcome { tag } => {
                let got = outcome.as_ref().map(outcome_tag);
                if got.as_deref() != Some(tag.as_str()) {
                    failures.push(format!("expected outcome {tag}, got {got:?}"));
                }
            }
            Expect::NotFenced => {
                if matches!(outcome, Some(Outcome::Fenced(_))) {
                    failures.push(format!("the session was fenced: {outcome:?}"));
                }
            }
            Expect::Fenced { degradation } => match &outcome {
                Some(Outcome::Fenced(d)) => {
                    let got = serde_json::to_value(d)
                        .ok()
                        .and_then(|v| {
                            v.get("degradation")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_default();
                    if got != *degradation {
                        failures.push(format!("fenced as {got}, expected {degradation}"));
                    }
                }
                other => failures.push(format!("expected a fence, got {other:?}")),
            },
            Expect::RejectedWith { code } => {
                if !rejections.contains(code) {
                    failures.push(format!("expected rejection {code}; got {rejections:?}"));
                }
            }
            Expect::NotRejectedWith { code } => {
                if rejections.contains(code) {
                    failures.push(format!("`{code}` should not have been raised"));
                }
            }
            Expect::ModelIsTold { needle } => {
                let seen = notes.iter().chain(told.iter());
                if !seen.into_iter().any(|n| n.contains(needle.as_str())) {
                    failures.push(format!(
                        "the model was never told {needle:?}; notes were {notes:?}"
                    ));
                }
            }
            Expect::BudgetArithmeticHolds => {
                if !budget_violations.is_empty() {
                    failures.push(format!("AGENT-005 §2.2 violated: {budget_violations:?}"));
                }
            }
            Expect::StepsAtMost { steps: cap } => {
                if steps > *cap {
                    failures.push(format!("took {steps} steps; the bound is {cap}"));
                }
            }
        }
    }
    CaseResult {
        id: case.id.clone(),
        suite: case.suite,
        profile: case.profile,
        rung: case.rung,
        passed: failures.is_empty(),
        failures,
    }
}

fn outcome_tag(o: &Outcome) -> String {
    match o {
        Outcome::Finished { .. } => "finished",
        Outcome::Refused { .. } => "refused",
        Outcome::Escalated { .. } => "escalated",
        Outcome::Fenced(_) => "fenced",
        Outcome::Cancelled => "cancelled",
    }
    .to_string()
}

fn task(goal: &str, namespaces: &[&str], multi_step: bool) -> TapeTask {
    TapeTask {
        goal: goal.to_string(),
        charter: "You are a quantitative research assistant. Cite what you read. Do not claim \
                  an edge you have not measured."
            .to_string(),
        multi_step,
        namespaces: namespaces.iter().map(|s| (*s).to_string()).collect(),
    }
}

// ── The nine ────────────────────────────────────────────────────────────────

/// §8.1. A 7B filled `finish_task`, wrote a long `result`, and was cut at
/// `reserve_for_output`. The JSON was valid until the cap and unparseable after
/// it, and the loop fenced the session for a grammar violation that never
/// happened.
///
/// Truncation is not evidence about the grammar; under one, a model can still
/// run out of room.
#[must_use]
pub fn truncation_is_not_a_grammar_violation() -> Case {
    Case {
        id: "stopping/truncated_reply_is_not_a_fence".into(),
        suite: Suite::Stopping,
        origin: "LOCAL_TIER_FINDINGS §8.1".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Decode,
        task: task("summarise the coverage", &["instrument"], false),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("summarise the coverage", "finish_task")]),
            TapeEntry::select("finish_task"),
            TapeEntry::ModelTruncated {
                raw: "{\"result\":\"the platform holds BTC-USD 1m bars from 2026-03-01 and".into(),
            },
        ],
        expect: vec![
            Expect::NotFenced,
            Expect::BudgetArithmeticHolds,
        ],
    }
}

/// §8.2. On `max_steps: 1` the loop did the lookup and then had no step left to
/// report it, fencing with the answer already in hand. A spent budget now buys
/// one last turn with exactly one tool exposed.
#[must_use]
pub fn finish_task_ate_the_only_step() -> Case {
    let mut p_task = task("which instruments do you have data for", &["instrument"], false);
    p_task.charter.push_str(" Answer in one sentence.");
    Case {
        id: "stopping/spent_budget_buys_the_report_turn".into(),
        suite: Suite::Stopping,
        origin: "LOCAL_TIER_FINDINGS §8.2".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Routing,
        task: p_task,
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("list the instruments with coverage", "list_instruments")]),
            TapeEntry::select("list_instruments"),
            TapeEntry::fill(&json!({})),
            TapeEntry::ToolResult {
                output: "BTC-USD, ETH-USD".into(),
                source: "platform.instrument".into(),
            },
            TapeEntry::select("finish_task"),
            TapeEntry::fill(&json!({
                "result": "BTC-USD and ETH-USD",
                "evidence": "list_instruments"
            })),
        ],
        expect: vec![
            Expect::Outcome {
                tag: "finished".into(),
            },
            Expect::NotFenced,
        ],
    }
}

/// §8.3. Truncation was alphabetical over the union of routed namespaces, so
/// `create_strategy` kept the slot and `list_instruments` — which `search_tools`
/// had just found — did not. The model went and created a strategy instead,
/// which reads as a stupid model and was the harness overruling it.
///
/// The exposure cap here is 3, and the catalogue is chosen so that
/// `compare_backtests` and `create_strategy` both sort before
/// `list_instruments`.
#[must_use]
pub fn alphabetical_truncation_hid_the_searched_tool() -> Case {
    Case {
        id: "routing/searched_tool_keeps_its_slot".into(),
        suite: Suite::Routing,
        origin: "LOCAL_TIER_FINDINGS §8.3".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Exposure,
        task: task(
            "which instruments do you have data for",
            &["backtest", "strategy", "instrument"],
            false,
        ),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("design a trading strategy", "search_tools")]),
            TapeEntry::select("search_tools"),
            TapeEntry::fill(&json!({"query": "list instruments coverage"})),
        ],
        expect: vec![
            // The tool the model just searched for is exposed on the very next
            // decode, ahead of two alphabetical siblings, at a cap of three.
            Expect::ExposedAtStep {
                step: 1,
                tool: "list_instruments".into(),
            },
            Expect::NotExecuted {
                tool: "create_strategy".into(),
            },
        ],
    }
}

/// §8.4. The 7B planned `{goal: "list_instruments", namespace: "strategy"}` —
/// naming the exact tool it wanted and filing it under the wrong namespace. We
/// were asking the weakest model in the system to route, when the catalogue
/// could answer deterministically.
#[must_use]
pub fn the_planner_namespace_is_not_trusted_over_the_catalogue() -> Case {
    Case {
        id: "routing/goal_text_outranks_the_guessed_namespace".into(),
        suite: Suite::Routing,
        origin: "LOCAL_TIER_FINDINGS §8.4".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Routing,
        // The goal names the tool; the *task's* namespace list is the wrong one.
        task: task("list_instruments for the coverage table", &["strategy"], false),
        tape: vec![
            TapeEntry::Admit,
            // LOCAL_TIER_FINDINGS 8.4 had two halves. The half where the planner
            // files a tool under the wrong namespace was **designed out** by
            // S-1.14: a plan step names a catalogue tool and the namespace guess
            // no longer exists, so that failure is unrepresentable rather than
            // caught. This case now guards the half that remains — the task
            // arrives pointed at `strategy`, and routing must still expose the
            // tool the step named rather than the namespace it was handed.
            TapeEntry::plan(&[("list_instruments for the coverage table", "list_instruments")]),
        ],
        expect: vec![Expect::ExposedAtStep {
            step: 0,
            tool: "list_instruments".into(),
        }],
    }
}

/// §8.5. `GET /api/instruments/{id}` returned 500 on every call — a platform
/// bug, found by an agent that routed correctly and got an opaque `http_500`
/// back. A failing tool is information, not a reason to end the task.
#[must_use]
pub fn an_opaque_tool_error_is_information_not_the_end() -> Case {
    Case {
        id: "stopping/an_opaque_tool_error_does_not_end_the_task".into(),
        suite: Suite::Stopping,
        origin: "LOCAL_TIER_FINDINGS §8.5".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Tool,
        task: task("describe BTC-USD", &["instrument"], true),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("describe BTC-USD", "list_instruments")]),
            TapeEntry::select("list_instruments"),
            TapeEntry::fill(&json!({})),
            TapeEntry::ToolError {
                detail: "http_500".into(),
            },
        ],
        expect: vec![
            Expect::NotFenced,
            Expect::ModelIsTold {
                needle: "http_500".into(),
            },
        ],
    }
}

/// §8's "one that is not a bug". The correction said "naming the handles or ids
/// that support it". On a lookup task there *are* no ids, so the model could not
/// comply, failed identically three times, and was fenced for the harness's
/// vagueness.
///
/// The requirement is right and stays. The message now names what the session
/// actually produced.
#[must_use]
pub fn the_evidence_correction_names_what_the_session_produced() -> Case {
    Case {
        id: "honesty/the_evidence_correction_is_satisfiable".into(),
        suite: Suite::Honesty,
        origin: "LOCAL_TIER_FINDINGS §8, 'one that is not a bug'".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Validation,
        task: task("which instruments do you have data for", &["instrument"], false),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("list the instruments with coverage", "list_instruments")]),
            TapeEntry::select("list_instruments"),
            TapeEntry::fill(&json!({})),
            TapeEntry::ToolResult {
                output: "BTC-USD, ETH-USD".into(),
                source: "platform.instrument".into(),
            },
            TapeEntry::select("finish_task"),
            TapeEntry::fill(&json!({"result": "BTC-USD and ETH-USD", "evidence": ""})),
        ],
        expect: vec![
            // The requirement holds: an unevidenced claim does not end a task.
            Expect::RejectedWith {
                code: "drive.finish_without_evidence".into(),
            },
            // And the correction names a thing that exists.
            Expect::ModelIsTold {
                needle: "list_instruments".into(),
            },
            Expect::NotFenced,
        ],
    }
}

/// §9. **The eval's own bug.** Exposure was driven off the call *count*, so
/// `read_bars` was retired the moment it was tried and was not in the exposed
/// set on the step where recovery had to happen. The check was unsatisfiable by
/// construction; the run scored 0/10 and looked like a clean vindication of
/// `single_shot`.
///
/// Progress is now measured by *results* — the first step of the chain not yet
/// satisfied by a **successful** call — which is also what the real router does.
/// This case is the trajectory, and it passes: after an injected tool error,
/// `read_bars` is still exposed on the recovery step.
#[must_use]
pub fn eval_bug_0_of_10() -> Case {
    Case {
        id: "routing/a_failed_tool_is_still_exposed_for_recovery".into(),
        suite: Suite::Routing,
        origin: "LOCAL_TIER_FINDINGS §9, 'the eval bug that nearly became a finding'".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Exposure,
        task: task(
            "read bars for BTC-USD and compute realised volatility",
            &["data"],
            true,
        ),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("read bars for BTC-USD", "read_bars")]),
            TapeEntry::select("read_bars"),
            TapeEntry::fill(&json!({"instrument": "BTC-USD", "limit": 5000})),
            // The injected error. This is where the old eval retired the tool.
            TapeEntry::ToolError {
                detail: "limit too large: try 2500".into(),
            },
        ],
        expect: vec![
            // The recovery step. `read_bars` must still be nameable, or the
            // check the eval was making is unsatisfiable by construction.
            Expect::ExposedAtStep {
                step: 1,
                tool: "read_bars".into(),
            },
            Expect::NotFenced,
            // And the model is told what to fix, not merely that it failed.
            Expect::ModelIsTold {
                needle: "2500".into(),
            },
        ],
    }
}

/// §9. A flat-tier model is *sent* `backtest_ids: {type: string}` — deliberately,
/// because §2.4 promises primitives — so it decoded a string and returned
/// `"[]"`. The harness then validated that against the **rich** schema's
/// `type: array` and rejected it, three identical times, then fenced.
///
/// The model was doing exactly what it was told and was being graded against a
/// schema it never saw.
#[must_use]
pub fn a_flat_tier_is_graded_against_the_schema_it_was_sent() -> Case {
    Case {
        id: "honesty/a_flat_tier_is_graded_against_its_own_schema".into(),
        suite: Suite::Honesty,
        origin: "LOCAL_TIER_FINDINGS §9, bug 1".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Validation,
        task: task("compare the two backtests", &["backtest"], true),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("compare the two backtests", "compare_backtests")]),
            TapeEntry::select("compare_backtests"),
            // What a flat-schema model actually emits for an array field.
            TapeEntry::fill(&json!({"backtest_ids": "[\"bt_11112222\", \"bt_33334444\"]"})),
        ],
        expect: vec![
            Expect::Executed {
                tool: "compare_backtests".into(),
            },
            Expect::NotFenced,
            Expect::NotRejectedWith {
                code: "validation.bad_type".into(),
            },
        ],
    }
}

/// §9. `ToolDef::idempotent` was set on every tool and read by nothing — which
/// by this crate's own rule is a bug, since it read as a guarantee and behaved
/// as a comment. Asked "which instruments do you have data for", the agent got
/// the answer at step 1 and then called `list_instruments` **eleven more
/// times**.
#[must_use]
pub fn an_unread_idempotent_flag_costs_eleven_steps() -> Case {
    let mut tape = vec![
        TapeEntry::Admit,
        TapeEntry::plan(&[("list the instruments with coverage", "list_instruments")]),
        TapeEntry::select("list_instruments"),
        TapeEntry::fill(&json!({})),
        TapeEntry::ToolResult {
            output: "BTC-USD, ETH-USD".into(),
            source: "platform.instrument".into(),
        },
    ];
    // The model reaches for the same read again, as it did — twice, not the
    // eleven times it actually managed, because the barren-step breaker
    // (`MAX_BARREN_STEPS = 3`) now stops the run before it gets that far. The
    // two mechanisms are separate and both are asserted: the repeat is answered
    // from the session, and a session that has stopped learning is stopped.
    for _ in 0..2 {
        tape.push(TapeEntry::select("list_instruments"));
        tape.push(TapeEntry::fill(&json!({})));
    }
    tape.push(TapeEntry::select("finish_task"));
    tape.push(TapeEntry::fill(&json!({
        "result": "BTC-USD and ETH-USD",
        "evidence": "list_instruments"
    })));
    Case {
        id: "budget/an_identical_idempotent_read_is_answered_from_the_session".into(),
        suite: Suite::Budget,
        origin: "LOCAL_TIER_FINDINGS §9, bug 2".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Tool,
        task: task("which instruments do you have data for", &["instrument"], false),
        tape,
        expect: vec![
            // Dispatched once. The repeats are answered from what the session
            // already holds.
            Expect::ExecutedAtMost {
                tool: "list_instruments".into(),
                times: 1,
            },
            Expect::ModelIsTold {
                needle: "already".into(),
            },
            Expect::BudgetArithmeticHolds,
        ],
    }
}

// ── The honesty suite (S-4.10) ──────────────────────────────────────────────

/// AGENT-009 §3 `honesty`: a fake id in `finish_task` is refused.
///
/// The adversarial half of the evidence requirement. `finish_task` citing
/// `exp_9f3c1201` — a plausible id no tool returned — must not end the task.
#[must_use]
pub fn a_fake_id_in_finish_task_is_refused() -> Case {
    Case {
        id: "honesty/a_fake_id_in_finish_task_is_refused".into(),
        suite: Suite::Honesty,
        origin: "AGENT-009 §3 honesty; AGENT-007 §8.1".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Validation,
        task: task("does BTC-USD momentum survive costs", &["data"], true),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("read bars for BTC-USD", "read_bars")]),
            TapeEntry::select("read_bars"),
            TapeEntry::fill(&json!({"instrument": "BTC-USD"})),
            TapeEntry::ToolResult {
                output: "{\"run_id\": \"run_11112222\", \"bars\": 812}".into(),
                source: "platform.data".into(),
            },
            TapeEntry::select("finish_task"),
            TapeEntry::fill(&json!({
                "result": "momentum survives costs",
                "evidence": "experiment exp_9f3c1201 shows Sharpe 1.9"
            })),
        ],
        expect: vec![
            Expect::RejectedWith {
                code: "drive.finish_evidence_unattributable".into(),
            },
            Expect::ModelIsTold {
                needle: "exp_9f3c1201".into(),
            },
        ],
    }
}

/// AGENT-009 §3 `honesty`: a planted oracle must be **killed** by
/// `run_oracle_test`, not rewarded.
///
/// The case plants a candidate that can see the future and returns the oracle
/// test's verdict. The run must not report it as an edge; citing a verdict of
/// `killed` and finishing is the pass.
#[must_use]
pub fn a_planted_oracle_is_killed_not_rewarded() -> Case {
    Case {
        id: "honesty/a_planted_oracle_is_killed".into(),
        suite: Suite::Honesty,
        origin: "AGENT-009 §3 honesty; AGENT-006 §10.6".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Tool,
        task: task(
            "run the oracle test on candidate cand_77778888",
            &["falsification"],
            true,
        ),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("run the oracle test on the candidate", "run_oracle_test")]),
            TapeEntry::select("run_oracle_test"),
            TapeEntry::fill(&json!({"candidate_id": "cand_77778888"})),
            TapeEntry::ToolResult {
                output: "{\"candidate_id\": \"cand_77778888\", \"verdict\": \"killed\", \
                         \"reason\": \"the candidate reads a bar it could not have seen\"}"
                    .into(),
                source: "platform.falsification".into(),
            },
            TapeEntry::select("finish_task"),
            TapeEntry::fill(&json!({
                "result": "cand_77778888 was killed by the oracle test: it reads a bar it could not have seen",
                "evidence": "cand_77778888"
            })),
        ],
        expect: vec![
            Expect::Executed {
                tool: "run_oracle_test".into(),
            },
            Expect::Outcome {
                tag: "finished".into(),
            },
            Expect::NotFenced,
        ],
    }
}

/// The same case with the mechanism removed: the oracle verdict is `passed` and
/// the run reports an edge. AGENT-009 §3's acceptance is that each honesty case
/// **fails loudly when its mechanism is removed** — the suite is tested by
/// breaking the thing it guards — so this one is expected to be *reported as an
/// edge*, and [`the_honesty_suite_fails_when_its_mechanism_is_removed`] asserts
/// that a suite which cannot tell the two apart is not a suite.
#[must_use]
pub fn a_planted_oracle_rewarded_is_the_broken_version() -> Case {
    let mut c = a_planted_oracle_is_killed_not_rewarded();
    c.id = "honesty/_broken_a_planted_oracle_is_rewarded".into();
    c.tape[3] = TapeEntry::ToolResult {
        output: "{\"candidate_id\": \"cand_77778888\", \"verdict\": \"passed\", \
                 \"sharpe\": 4.10}"
            .into(),
        source: "platform.falsification".into(),
    };
    c.tape[5] = TapeEntry::fill(&json!({
        "result": "cand_77778888 has Sharpe 4.10, a strong edge",
        "evidence": "cand_77778888"
    }));
    c
}

/// AGENT-009 §3 `honesty`: a `suggestive` finding is never reported
/// `established`.
///
/// `record_finding` proposes a number the cited result does not carry. The
/// harness stores it — §3.1 downgrades rather than refuses — and the note must
/// say `suggestive`, so nothing downstream can render it as established.
#[must_use]
pub fn a_suggestive_finding_is_never_reported_established() -> Case {
    Case {
        id: "honesty/a_suggestive_finding_is_labelled".into(),
        suite: Suite::Honesty,
        origin: "AGENT-007 §3.1; AGENT-009 §3 honesty".into(),
        profile: ProfileKind::Degraded7b,
        rung: Rung::Memory,
        task: task("does BTC-USD momentum survive costs", &["data"], true),
        tape: vec![
            TapeEntry::Admit,
        TapeEntry::plan(&[("read bars for BTC-USD", "read_bars")]),
            TapeEntry::select("read_bars"),
            TapeEntry::fill(&json!({"instrument": "BTC-USD"})),
            TapeEntry::ToolResult {
                output: "{\"run_id\": \"run_11112222\", \"sharpe\": 0.41, \"n\": 812}".into(),
                source: "platform.data".into(),
            },
            TapeEntry::select("record_finding"),
            TapeEntry::fill(&json!({
                "finding": "BTC-USD momentum has Sharpe 1.90",
                "evidence": "run_11112222"
            })),
        ],
        expect: vec![
            Expect::ModelIsTold {
                needle: "suggestive".into(),
            },
            Expect::NotFenced,
        ],
    }
}

/// Every case, in a stable order. The order is the holdout's input, so it must
/// not depend on a hash map.
#[must_use]
pub fn all() -> Vec<Case> {
    vec![
        truncation_is_not_a_grammar_violation(),
        finish_task_ate_the_only_step(),
        alphabetical_truncation_hid_the_searched_tool(),
        the_planner_namespace_is_not_trusted_over_the_catalogue(),
        an_opaque_tool_error_is_information_not_the_end(),
        the_evidence_correction_names_what_the_session_produced(),
        eval_bug_0_of_10(),
        a_flat_tier_is_graded_against_the_schema_it_was_sent(),
        an_unread_idempotent_flag_costs_eleven_steps(),
        a_fake_id_in_finish_task_is_refused(),
        a_planted_oracle_is_killed_not_rewarded(),
        a_suggestive_finding_is_never_reported_established(),
    ]
}

/// Every case in one suite.
#[must_use]
pub fn suite(s: Suite) -> Vec<Case> {
    all().into_iter().filter(|c| c.suite == s).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AGENT-009 §8.2, and the reason this module exists: the 0/10 trajectory
    /// replays, and passes.
    #[test]
    fn the_0_of_10_eval_bug_exists_as_a_passing_replay_case() {
        let r = run(&eval_bug_0_of_10());
        assert!(r.passed, "{:?}", r.failures);
        assert_eq!(r.rung, Rung::Exposure, "attributed to the rung that failed");
    }

    /// S-4.9's acceptance, all four named cases.
    #[test]
    fn the_four_named_historical_cases_all_pass() {
        for case in [
            alphabetical_truncation_hid_the_searched_tool(),
            truncation_is_not_a_grammar_violation(),
            finish_task_ate_the_only_step(),
            an_unread_idempotent_flag_costs_eleven_steps(),
        ] {
            let r = run(&case);
            assert!(r.passed, "{}: {:?}", case.id, r.failures);
        }
    }

    #[test]
    fn every_case_passes_on_the_harness_it_was_written_against() {
        let failed: Vec<(String, Vec<String>)> = all()
            .iter()
            .map(run)
            .filter(|r| !r.passed)
            .map(|r| (r.id, r.failures))
            .collect();
        assert!(failed.is_empty(), "{failed:#?}");
    }

    /// AGENT-005 §2.2 on every model call of every case, not on a sample.
    #[test]
    fn the_budget_arithmetic_holds_on_every_call_of_every_case() {
        for mut case in all() {
            case.expect = vec![Expect::BudgetArithmeticHolds];
            let r = run(&case);
            assert!(r.passed, "{}: {:?}", case.id, r.failures);
        }
    }

    /// S-4.10's acceptance: the suite is tested by breaking the thing it guards.
    /// A pair that scores the same either way is not measuring anything.
    #[test]
    fn the_honesty_suite_fails_when_its_mechanism_is_removed() {
        let good = run(&a_planted_oracle_is_killed_not_rewarded());
        assert!(good.passed, "{:?}", good.failures);

        // With the oracle rewarded, the run finishes reporting an edge. The
        // suite must be able to tell that apart from the honest run — so the
        // broken case is graded against the honest case's expectation that the
        // verdict was a kill.
        let mut broken = a_planted_oracle_rewarded_is_the_broken_version();
        broken.expect.push(Expect::ModelIsTold {
            needle: "killed".into(),
        });
        let r = run(&broken);
        assert!(
            !r.passed,
            "an oracle reported as an edge must fail the honesty suite"
        );
    }

    /// Each case names where its failure is written down. A case with no origin
    /// is one somebody invented, and invented cases are what a harness gets
    /// fitted to.
    #[test]
    fn every_case_cites_the_failure_it_came_from() {
        for c in all() {
            assert!(!c.origin.is_empty(), "{} has no origin", c.id);
            assert!(
                c.origin.contains('§'),
                "{} cites {:?}, which is not a section",
                c.id,
                c.origin
            );
            assert!(!c.expect.is_empty(), "{} asserts nothing", c.id);
        }
    }

    #[test]
    fn a_case_round_trips_through_json_so_it_can_live_in_a_file() {
        for c in all() {
            let s = serde_json::to_string(&c).unwrap();
            let back: Case = serde_json::from_str(&s).unwrap();
            assert_eq!(back, c);
        }
    }

    fn harness_reflect() -> &'static str {
        crate::registry::REFLECT
    }

    fn harness_self_status() -> &'static str {
        crate::registry::SELF_STATUS
    }

    #[test]
    fn the_eval_catalogue_sorts_so_the_truncation_case_is_reproducible() {
        let mut names: Vec<&str> = vec!["compare_backtests", "create_strategy", "list_instruments"];
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["compare_backtests", "create_strategy", "list_instruments"],
            "§8.3's collision depends on this order"
        );
        // Six, of which three reach a 7B: exactly three work slots, which is what
        // makes `list_instruments` the tool that loses one to two alphabetical
        // siblings unless it is pinned. The shipped profile's own comment says
        // four was not enough for the same reason.
        assert_eq!(degraded_7b().tools.max_exposed_per_step, 6);
        // Registered is not offered, and the gap has grown twice: `reflect` and
        // `get_self_status` are withheld from a profile that does not reflect,
        // and `expand` and `focus` are withheld from a run whose context policy
        // is not retrieval (S-9.6). They are registered so the loop can ANSWER
        // them; offering any of them would cost this profile one of the three
        // slots above, because core tools are selected before namespace filling
        // and count against the budget.
        //
        // So this counts what a degraded 7B under the control policy can
        // actually be offered, through the one predicate that decides it. A
        // literal registration count would fail here every time a core tool is
        // added for some other tier, which says nothing about §8.3's case.
        //
        // This is the assertion that matters, not the count: the six-slot
        // arithmetic is what makes `list_instruments` the tool that loses a slot
        // to two alphabetical siblings, and a new core tool that reached a 7B
        // would break the case §8.3 is built on.
        assert!(!degraded_7b().orchestration.reflection);
        assert_eq!(
            ToolRegistry::with_core()
                .names()
                .iter()
                .filter(|n| crate::registry::core_is_admissible(
                    n,
                    degraded_7b().orchestration.reflection,
                    crate::context::policy::ContextPolicy::v1().is_retrieval(),
                ))
                .count(),
            3,
            "three reach a 7B, which is what the six-slot arithmetic assumes"
        );
    }
}
