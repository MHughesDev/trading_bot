//! The loop owns ONE working state, and rewrites it (AGENT-008 §2; Set S S-2.x).
//!
//! `state.rs` proved the *type* does not grow. That was the smaller half: the type
//! could be perfect while the loop went on pushing a `Section::WorkingState` block
//! at nine separate sites, which is what it did. These tests are about the loop —
//! they read the prompt the model is actually sent, because that is the only place
//! the claim is true or false.
//!
//! Each test names the append site it replaces.

use serde_json::json;

use harness::drive::{Decode, Effect, Input, Loop, ModelCall, Task};
use harness::hardware::{Admission, TaskDemand};
use harness::profile::{Profile, ProfileSet};
use harness::registry::{Risk, ToolDef, ToolRegistry};
use harness::state::Detail;

fn profile() -> Profile {
    ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
        .expect("the shipped profiles load")
        .get("qwen2.5:7b-instruct")
        .expect("the degraded profile ships")
        .clone()
}

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::with_core();
    reg.register(ToolDef {
        name: "get_bars".into(),
        namespace: "data".into(),
        description: "Read price bars. Use when the step needs price history.".into(),
        risk: Risk::Read,
        core: false,
        idempotent: true,
        input_schema: json!({
            "type": "object",
            "properties": {"instrument": {"type": "string"}, "nonce": {"type": "integer"}},
            "required": ["instrument"],
        }),
    })
    .unwrap();
    reg
}

fn task() -> Task {
    Task {
        goal: "find whether BTC-USD has momentum at 1h".into(),
        charter: "You are a research agent.".into(),
        demand: TaskDemand::MultiStep,
        namespaces: vec!["data".into()],
    }
}

fn started() -> (Loop, Vec<Effect>) {
    let mut p = profile();
    // Freeform: a plan is a separate concern and this file is about the state card.
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.max_steps = 400;
    let mut l = Loop::new(p, registry(), task());
    let fx = l.next(Input::Admission(Admission::Admit));
    (l, fx)
}

/// A loop on the unconstrained (native) path.
///
/// Corrections are only reachable there: under a grammar, a reply that violates the
/// schema is a FENCE, not a retry — the sampler cannot emit it, so it is evidence
/// about the backend rather than about the model. A test that wants to see the
/// correction slot has to be on the path where corrections exist.
fn native_started() -> (Loop, Vec<Effect>) {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.max_steps = 400;
    p.output.constrained_decoding = false;
    let mut l = Loop::new(p, registry(), task());
    let fx = l.next(Input::Admission(Admission::Admit));
    (l, fx)
}

fn call(fx: &[Effect]) -> &ModelCall {
    fx.iter()
        .find_map(|e| match e {
            Effect::CallModel(c) => Some(&**c),
            _ => None,
        })
        .expect("a model call")
}

fn prompt(fx: &[Effect]) -> String {
    call(fx).prompt.clone()
}

/// Run one whole step: select, fill, and a tool result with the ids it returned.
fn step(l: &mut Loop, nonce: u32, output: &str) -> Vec<Effect> {
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    l.next(Input::ModelReply {
        raw: format!(r#"{{"instrument":"BTC-USD","nonce":{nonce}}}"#),
    });
    l.next(Input::ToolResult {
        output: output.to_string(),
        source: "platform.data".into(),
    })
}

/// Site 3 — the correction. It used to be pushed once per rejection.
///
/// Adversarial: three rejections in one step, and the prompt must carry ONE
/// standing correction. Three corrections in context is three contradictory
/// instructions, and a model reading them answers the corrections instead of the
/// goal.
#[test]
fn three_rejections_leave_one_correction_in_the_prompt() {
    let (mut l, _) = native_started();
    // Each of these fails the ladder (`instrument` is required), so it rejects and
    // re-asks with a correction.
    let mut last = Vec::new();
    for _ in 0..2 {
        last = l.next(Input::ModelReply {
            raw: r#"{"name":"get_bars","arguments":{"nonce":1}}"#.into(),
        });
    }
    let body = prompt(&last);
    assert_eq!(
        body.matches("LAST REJECTION").count(),
        1,
        "the state carries one correction, not one per attempt:\n{body}"
    );
}

/// A correction is rendered once and then gone (AGENT-008 §2, `Correction`).
///
/// Carried across a step boundary it reads as a standing instruction about the run
/// rather than about the reply that earned it.
#[test]
fn a_correction_does_not_survive_the_step_that_earned_it() {
    let (mut l, _) = native_started();
    let rejected = l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars","arguments":{"nonce":1}}"#.into(),
    });
    assert!(prompt(&rejected).contains("LAST REJECTION"));

    // Now answer properly and finish the step.
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars","arguments":{"instrument":"BTC-USD","nonce":1}}"#.into(),
    });
    let next_step = l.next(Input::ToolResult {
        output: "ts,close\n1,100".into(),
        source: "platform.data".into(),
    });
    assert!(
        !prompt(&next_step).contains("LAST REJECTION"),
        "a rejection from the previous step is not a standing instruction:\n{}",
        prompt(&next_step)
    );
}

/// Site 6 — the findings ledger. It used to push the WHOLE list again per finding,
/// so five findings cost fifteen lines and n findings cost n(n+1)/2.
#[test]
fn five_findings_render_one_ledger_not_five() {
    let (mut l, _) = started();
    let mut last = Vec::new();
    for i in 0..5 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"record_finding"}"#.into(),
        });
        last = l.next(Input::ModelReply {
            raw: format!(r#"{{"finding":"fact number {i} about the data","evidence":"read it"}}"#),
        });
    }
    let body = prompt(&last);
    assert_eq!(
        body.matches("ESTABLISHED SO FAR").count(),
        1,
        "the ledger is rewritten, not appended:\n{body}"
    );
    assert_eq!(
        body.matches("fact number 0 about the data").count(),
        1,
        "and the first finding appears once, not once per later finding"
    );
    assert_eq!(l.working().findings.len(), 5);
}

/// The property the whole refactor exists for, measured on the loop rather than on
/// the type: the harness-authored part of the prompt does not grow with the run.
///
/// Adversarial in the way that matters — it drives sixty real steps, each producing
/// a *new* tool result with a *new* id, which is precisely the traffic that made the
/// old nine-site prompt grow without bound. What may grow is the observation
/// transcript (that is what compaction is for). What may not grow is the state.
#[test]
fn sixty_steps_later_the_state_card_is_the_same_size() {
    let (mut l, _) = started();

    let size_at = |l: &Loop| l.working().card(Detail::Flat).len();

    // Four steps in, not one: the card names its three most recent evidence ids, so
    // it reaches its steady size at the third. Measuring from a card that has not
    // filled yet would measure the fill, not the growth.
    for i in 0..4 {
        step(&mut l, i, &format!("bt-{i:08} close {}", 100 + i));
    }
    let early = size_at(&l);
    for i in 4..60 {
        step(&mut l, i, &format!("bt-{i:08} close {}", 100 + i));
    }
    let late = size_at(&l);

    assert!(
        late.abs_diff(early) < 40,
        "the card went from {early} to {late} bytes over sixty steps — it is \
         accumulating, which is the bug the nine append sites had"
    );
    assert_eq!(
        l.working().evidence.len(),
        60,
        "the ids themselves are all still known — a fixed-size VIEW over an \
         unbounded history, not a shorter history"
    );
    // And the whole point of keeping them: an id read at step 1 can still be cited
    // at step 60, after compaction has long since dropped the block it arrived in.
    assert!(l.working().can_cite("bt-00000000"));
}

/// Site 5 — an approval refusal. It used to be a note appended to the transcript;
/// it is a decision the run has made, and the card answers it for the rest of the
/// run rather than leaving it to be re-derived.
#[test]
fn a_refused_approval_becomes_a_decision_not_a_message() {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    let mut reg = registry();
    reg.register(ToolDef {
        name: "delete_study".into(),
        namespace: "admin".into(),
        description: "Delete a study and everything under it.".into(),
        risk: Risk::Destructive,
        core: false,
        idempotent: false,
        input_schema: json!({
            "type": "object",
            "properties": {"study_id": {"type": "string"}},
            "required": ["study_id"],
        }),
    })
    .unwrap();
    let mut l = Loop::new(
        p,
        reg,
        Task {
            namespaces: vec!["admin".into()],
            ..task()
        },
    );
    l.next(Input::Admission(Admission::Admit));
    l.next(Input::ModelReply {
        raw: r#"{"name":"delete_study"}"#.into(),
    });
    let fx = l.next(Input::ModelReply {
        raw: r#"{"study_id":"s1"}"#.into(),
    });
    assert!(
        fx.iter().any(|e| matches!(e, Effect::AskHuman { .. })),
        "a destructive call asks"
    );
    let fx = l.next(Input::Approval {
        granted: false,
        note: "not that study".into(),
    });
    let decided = l
        .working()
        .already_decided("call delete_study")
        .expect("the refusal is on the card as a decision");
    assert!(decided.choice.contains("a human refused"));
    assert_eq!(decided.reason, "not that study");
    assert!(
        prompt(&fx).contains("ALREADY DECIDED"),
        "and the model reads it there: {}",
        prompt(&fx)
    );
}

/// Sites 1 and 2 — the two wrap-up directives. Both used to push a block; a run that
/// went barren AND then ran out of steps carried both.
#[test]
fn a_run_carries_one_wrap_up_directive_however_many_fire() {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.max_steps = 8;
    let mut l = Loop::new(p, registry(), task());
    l.next(Input::Admission(Admission::Admit));

    // Three barren steps — a tool error learns nothing, by design — buys the
    // "stop and report" turn.
    let mut last = Vec::new();
    for i in 0..3 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"get_bars"}"#.into(),
        });
        // A different `nonce` each time, so these are three DIFFERENT calls: an
        // identical repeat is short-circuited by the idempotence rule and never
        // reaches a tool, which is a different behaviour from the one under test.
        l.next(Input::ModelReply {
            raw: format!(r#"{{"instrument":"BTC-USD","nonce":{i}}}"#),
        });
        last = l.next(Input::ToolError {
            detail: "the backend is down".into(),
        });
    }
    let body = prompt(&last);
    assert!(
        body.contains("Stop and report"),
        "three barren steps buy a wrap-up turn:\n{body}"
    );

    // Now fail the wrap-up itself: `finish_task` with nothing citable is rejected,
    // so the run holds a directive AND a correction at once. Each has exactly one
    // slot, which is the property — before, both were pushes.
    let chose = l.next(Input::ModelReply {
        raw: r#"{"name":"finish_task"}"#.into(),
    });
    assert!(
        chose.iter().any(|e| matches!(e, Effect::CallModel(_))),
        "selecting finish_task asks for its arguments: {chose:?}"
    );
    let last = l.next(Input::ModelReply {
        raw: r#"{"result":"","evidence":""}"#.into(),
    });
    assert!(
        last.iter().any(|e| matches!(e, Effect::CallModel(_))),
        "an unevidenced finish is a correction, not a terminal: {last:?}"
    );
    let body = prompt(&last);
    assert_eq!(
        body.matches("Stop and report").count() + body.matches("This is your last turn").count(),
        1,
        "one directive, whatever fired it:\n{body}"
    );
    assert_eq!(
        body.matches("LAST REJECTION").count(),
        1,
        "and one correction beside it:\n{body}"
    );
}

/// The anchor and the card cannot disagree, because there is only one cursor now.
#[test]
fn the_anchor_and_the_card_read_the_same_plan_step() {
    let p = profile(); // planner_executor
    assert_eq!(
        p.orchestration.mode,
        harness::profile::Mode::PlannerExecutor
    );
    let mut l = Loop::new(p, registry(), task());
    l.next(Input::Admission(Admission::Admit));
    let fx = l.next(Input::ModelReply {
        raw: r#"{"steps":[{"goal":"pull bars","tool":"get_bars"},
                          {"goal":"check momentum","tool":"get_bars"}]}"#
            .into(),
    });
    let body = prompt(&fx);
    assert!(
        body.contains("THIS STEP (plan step 1 of 2): pull bars"),
        "{body}"
    );
    assert!(body.contains("NOW: step 1 of 2 — pull bars"), "{body}");

    // Finish step one; both renderings must move together.
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    l.next(Input::ModelReply {
        raw: r#"{"instrument":"BTC-USD"}"#.into(),
    });
    let fx = l.next(Input::ToolResult {
        output: "ts,close\n1,100".into(),
        source: "platform.data".into(),
    });
    let body = prompt(&fx);
    assert!(
        body.contains("THIS STEP (plan step 2 of 2): check momentum"),
        "{body}"
    );
    assert!(body.contains("NOW: step 2 of 2 — check momentum"), "{body}");
    assert_eq!(
        l.working().plan[0].status,
        harness::state::StepStatus::Done,
        "the finished step carries its own status — there is no second cursor to \
         disagree with it"
    );
}

/// The tool schemas leave the body on the way to the model, so the body is budgeted
/// against what is left of the window, not against all of it (AGENT-005 §4.2).
#[test]
fn the_body_is_budgeted_against_what_the_schemas_leave() {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.output.constrained_decoding = false; // native: the schemas really are sent
    p.tools.parallel_calls = false;
    let mut l = Loop::new(p.clone(), registry(), task());
    let fx = l.next(Input::Admission(Admission::Admit));
    assert!(matches!(call(&fx).decode, Decode::Native { .. }));

    // A single enormous result, then check the assembled prompt still fits the
    // window with the schemas charged against it.
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars","arguments":{"instrument":"BTC-USD"}}"#.into(),
    });
    let fx = l.next(Input::ToolResult {
        output: "x".repeat(4_000_000),
        source: "platform.data".into(),
    });
    let c = call(&fx);
    let schema_tokens: u32 = match &c.decode {
        Decode::Native { tools } => tools
            .iter()
            .map(|t| harness::context::estimate_tokens(&t.to_string()))
            .sum(),
        other => panic!("expected native, got {other:?}"),
    };
    let body_tokens = harness::context::estimate_tokens(&c.prompt);
    assert!(
        body_tokens + schema_tokens <= p.input_budget_tokens(),
        "body {body_tokens} + schemas {schema_tokens} exceeds the window {}",
        p.input_budget_tokens()
    );
}

// ── S-1.13: the bound, proved of the loop rather than of the type ────────────

/// The acceptance criterion S-1.13 was unticked for: **no working-state content is
/// appended across steps**, asserted on the prompt the model is actually sent.
///
/// `state.rs` proves a `WorkingState` does not grow under 200 updates. That is a
/// statement about the type. This runs the loop for fifty whole steps and measures
/// the state block in the rendered prompt — which is where nine append sites used
/// to accumulate, and where the claim is either true or false.
///
/// Observations are excluded on purpose and that is not a loophole: an observation
/// is evidence, it is what compaction drops, and the section it lives in is the one
/// with a budget. The card is the part that has no budget and therefore must not
/// grow.
#[test]
fn the_state_card_does_not_grow_across_fifty_steps() {
    let (mut l, fx) = started();
    let first = l.working().card(Detail::Rich).len();
    assert!(first > 0, "the card is rendered from the start");
    assert!(prompt(&fx).contains("GOAL:"), "and it reaches the prompt");

    for i in 0..50u32 {
        step(
            &mut l,
            i,
            &format!("{{\"bars\": {}, \"run\": \"run_{i}\"}}", 500 + i),
        );
    }

    let after = l.working().card(Detail::Rich).len();
    assert!(
        after <= first + 400,
        "the state card grew from {first} to {after} bytes over fifty steps; \
         it is a fixed-size view over an unbounded history, not a transcript"
    );

    // And the flat card stays inside its tier's budget however long the run is.
    let flat = l.working().card(Detail::Flat);
    assert!(
        harness::context::estimate_tokens(&flat) <= 400,
        "the flat card is {} tokens after fifty steps",
        harness::context::estimate_tokens(&flat)
    );
}

/// The anchor derives from the state rather than being built beside it — the third
/// clause of S-1.13's acceptance. Two sources of truth for "where am I" is how the
/// card and the anchor come to disagree, and the model reads both.
#[test]
fn the_anchor_is_derived_from_the_same_state_the_card_renders() {
    let (mut l, _) = started();
    for i in 0..3u32 {
        step(&mut l, i, "{\"bars\": 500}");
    }
    let anchor = l.working().anchor();
    let card = l.working().card(Detail::Rich);
    assert!(
        card.contains("find whether BTC-USD has momentum"),
        "the card names the goal: {card}"
    );
    assert!(
        anchor.contains("find whether BTC-USD has momentum"),
        "and so does the anchor, from the same field: {anchor}"
    );
}

// ── S-1.15: decisions are remembered ────────────────────────────────────────

/// The dominant waste in the local traces, reproduced and then stopped.
///
/// The observed shape: a 7B lists the instruments, does not act on the answer,
/// lists them again, and keeps going until the step budget runs out. Every call is
/// legal, every call is idempotent, and the run learns nothing after the first.
///
/// Three calls, three different answers from the loop, and the escalation is the
/// design. The first dispatches. The second is answered from what the session
/// already holds — free, and it puts the answer back in front of the model. The
/// third is a **correction naming the step it was settled at**, because a model
/// that did not take the answer twice will not take it a third time, and by then a
/// repeat is a cycle rather than a slip.
#[test]
fn re_asking_a_settled_question_is_corrected_with_the_step_it_was_decided_at() {
    // The constrained path, deliberately: a redecision is a SEMANTIC rejection, not
    // a schema violation, so it retries on either path — and the constrained one is
    // where the waste this reproduces was actually observed.
    let (mut l, _) = started();

    // 1. The first call dispatches.
    let fx = step(&mut l, 1, "{\"instruments\": [\"BTC-USD\"]}");
    assert!(
        fx.iter().any(|e| matches!(e, Effect::CallModel(_))),
        "the first call runs and the loop asks for the next step"
    );

    // 2. The second, identical, is answered from the session.
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    let fx = l.next(Input::ModelReply {
        raw: r#"{"instrument":"BTC-USD","nonce":1}"#.into(),
    });
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::ExecuteTool { .. })),
        "an idempotent read the session already made is not made again"
    );
    assert!(
        l.working()
            .decided
            .iter()
            .any(|d| d.question.contains("get_bars")),
        "and the decision is on the card, not only in a comment"
    );

    // 3. The third is corrected, and the correction says when it was settled.
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    let fx = l.next(Input::ModelReply {
        raw: r#"{"instrument":"BTC-USD","nonce":1}"#.into(),
    });
    let correction = fx
        .iter()
        .find_map(|e| match e {
            Effect::Note(harness::drive::Note::Rejected { code, message, .. })
                if &**code == "drive.question_already_decided" =>
            {
                Some(message.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("the third ask is corrected, got {fx:?}"));
    assert!(
        correction.contains("decided at step"),
        "the correction names the step: {correction}"
    );
    assert!(
        correction.contains("say why it should change or move on"),
        "and leaves changing it possible: {correction}"
    );
}

// ── S-1.16: the goal is classified before it is planned ─────────────────────

/// The charter's plea becomes a mechanism.
///
/// "Many goals are lookups; do not plan five steps for one" was a sentence in the
/// charter, read every turn and acted on never. Classification makes it something
/// the loop does before the planner is asked anything, at zero model calls.
///
/// The direction of the default is the design and is asserted here: an
/// unrecognised goal is a `Lookup`, which withholds the trial budget. Being wrong
/// that way costs a run that could have spent trials and did not; being wrong the
/// other way spends trials on a question that never needed one.
#[test]
fn a_goal_is_classified_before_planning_and_the_default_withholds_trials() {
    use harness::state::GoalKind;

    // The four kinds, from the words a goal actually uses.
    assert_eq!(
        GoalKind::classify("how many 1m bars do we hold for BTC-USD", &[]),
        GoalKind::Lookup
    );
    assert_eq!(
        GoalKind::classify(
            "backtest an ema cross on BTC-USD and report the sharpe",
            &[]
        ),
        GoalKind::Research
    );
    assert_eq!(
        GoalKind::classify("why did the gate 3 verdict flip overnight", &[]),
        GoalKind::Diagnose
    );
    assert_eq!(
        GoalKind::classify("compare the 1h and 4h variants on the same window", &[]),
        GoalKind::Compare
    );

    // The narrower reading wins where two apply: "compare these two backtests" is a
    // comparison, and reading it as research loses the thing that makes it checkable.
    assert_eq!(
        GoalKind::classify("compare these two backtests", &[]),
        GoalKind::Compare
    );

    // The hard pair, and why one keyword cannot decide it: same sentence shape,
    // different kinds. The first has to be measured; the second is a stored fact.
    assert_eq!(
        GoalKind::classify("find whether BTC-USD has momentum at 1h", &[]),
        GoalKind::Research
    );
    assert_eq!(
        GoalKind::classify("find whether we hold 1m bars for BTC-USD", &[]),
        GoalKind::Lookup
    );
    // And a market property on its own is not enough: this reads a number the
    // platform already holds.
    assert_eq!(
        GoalKind::classify("what is the sharpe of run_42", &[]),
        GoalKind::Lookup
    );

    // The default, and what it costs.
    assert_eq!(
        GoalKind::classify("something entirely unfamiliar", &[]),
        GoalKind::Lookup
    );
    assert!(
        !GoalKind::Lookup.may_spend_trials(),
        "a lookup never spends a trial"
    );
    assert_eq!(
        GoalKind::Lookup.max_tools(),
        3,
        "and sees at most three tools"
    );
    assert!(GoalKind::Research.may_spend_trials());
}

/// And the loop uses it: the classification and its acceptance criteria are on the
/// state before the first model call, not derived later from what the run did.
#[test]
fn the_loop_classifies_the_goal_and_derives_acceptance_before_the_first_call() {
    use harness::state::GoalKind;

    let (l, _) = started();
    // `task()`'s goal is "find whether BTC-USD has momentum at 1h" — a question that
    // needs evidence produced, on a `MultiStep` demand.
    assert_eq!(l.working().kind, GoalKind::Research);
    assert!(
        !l.working().acceptance.is_empty(),
        "acceptance is derived from the goal, not asked of the model"
    );
    assert!(
        l.working()
            .acceptance
            .iter()
            .any(|a| a.text.contains("every number reported is in a tool result")),
        "and every kind carries the citation criterion: {:?}",
        l.working().acceptance
    );
}

// ── S-1.11: what is budgeted is what is sent ────────────────────────────────

/// The whole prompt fits the window: system, schemas and body, summed.
///
/// `assemble_reserving` budgets the body against the window MINUS what leaves it
/// on the wire. The gap this closes is the other end: `ask` sent `task.charter`
/// raw while the assembly had already decided what the Charter section could be,
/// so the compaction that made room was performed on a body that then travelled
/// beside the untruncated original. Budgeted and sent were two different prompts.
#[test]
fn the_system_the_schemas_and_the_body_fit_the_window_together() {
    use harness::context::estimate_tokens;

    let (mut l, _) = started();
    let p = profile();

    // A long run, so compaction has actually had to make choices.
    for i in 0..40u32 {
        step(
            &mut l,
            i,
            &format!("{{\"bars\": {}, \"run\": \"run_{i}\"}}", 500 + i),
        );
    }
    let fx = l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    let call = call(&fx);

    let system = estimate_tokens(&call.system);
    let body = estimate_tokens(&call.prompt);
    let schemas: u32 = match &call.decode {
        Decode::SelectTool { candidates, .. } => {
            candidates.iter().map(|c| estimate_tokens(c)).sum::<u32>()
        }
        Decode::Native { tools } => tools
            .iter()
            .map(|t| estimate_tokens(&t.to_string()))
            .sum::<u32>(),
        _ => 0,
    };
    assert!(
        system + schemas + body <= p.input_budget_tokens(),
        "system {system} + schemas {schemas} + body {body} exceeds the window {}",
        p.input_budget_tokens()
    );
    // And the BUDGET the body was assembled against is still a usable share of the
    // window — the 25% floor. The rendered body is smaller than its budget
    // whenever there is less to say, which is the normal case and not a failure;
    // what must not happen is the reservation eating the allowance.
    let budget = p.input_budget_tokens() - schemas.min(p.input_budget_tokens());
    assert!(
        budget >= 3_500,
        "the body was budgeted {budget} tokens after reserving {schemas} for schemas;          the 25% floor is 3,500"
    );
    let _ = fx;
}

// ── S-1.12: one findings block, however many findings ───────────────────────

/// Twenty findings, one block, twenty lines.
///
/// The shape this replaces pushed a new block per `record_finding`, each holding
/// every finding so far — so twenty calls cost two hundred and ten renderings of
/// a finding, and the section grew as the square of the run.
#[test]
fn twenty_findings_render_as_one_block_of_twenty_lines() {
    let (mut l, _) = started();
    // A tool first: `record_finding` is held to the same evidence standard as
    // `finish_task`, so a finding citing a tool this run never called is refused —
    // which is the point, and means the fixture has to run one.
    step(&mut l, 0, "{\"bars\": 500}");
    for i in 0..20u32 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"record_finding"}"#.into(),
        });
        l.next(Input::ModelReply {
            raw: serde_json::json!({
                "finding": format!("finding number {i} about BTC-USD"),
                "evidence": "get_bars"
            })
            .to_string(),
        });
    }
    assert_eq!(l.working().findings.len(), 20, "all twenty were recorded");

    let fx = l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    let prompt = prompt(&fx);
    assert_eq!(
        prompt.matches("ESTABLISHED SO FAR").count(),
        1,
        "exactly one findings block, not one per finding"
    );
    for i in 0..20u32 {
        assert!(
            prompt.contains(&format!("finding number {i} ")),
            "finding {i} is listed"
        );
    }
}

// ── S-1.20: the two loops the protocol could sustain ────────────────────────

/// The re-plan allowance is enforced where a plan is ACCEPTED, and exhausting it
/// wraps the run up rather than fencing it.
///
/// Re-planning looks like progress to the barren-step breaker — a new plan is a
/// step that learned something — which is exactly why it was the loop nothing
/// bounded, and why the bound has to be its own counter.
///
/// **Driven with the allowance set to zero, because this `Loop` has no re-plan
/// trigger.** The plan is made once at admission and never re-made; when it is
/// exhausted `active_step()` returns `None` and the loop carries on without one.
/// So the guard is real and tested, and the four-plans-in-a-row case it is written
/// for cannot be reached from here until that trigger exists. Stated rather than
/// asserted around: a test that pretends to exercise a path the code does not have
/// is worse than one that says which half it covers.
///
/// What it does prove: the counter is consulted, the run wraps up instead of
/// fencing, the last turn can only report, and the working state survives — which
/// is the difference between reporting what defeated the plan and reporting
/// nothing.
#[test]
fn exhausting_the_replan_allowance_wraps_up_holding_the_working_state() {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::PlannerExecutor;
    p.orchestration.max_replans_per_run = 0;
    let mut l = Loop::new(p, registry(), task());
    l.next(Input::Admission(Admission::Admit));

    let fx = l.next(Input::ModelReply {
        raw: r#"{"steps":[{"goal":"pull bars","tool":"get_bars"}]}"#.into(),
    });
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Note(harness::drive::Note::Planned { .. }))),
        "a plan past the allowance is not served"
    );
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Done(_))),
        "and it is not a fence: the run still has a report to make"
    );
    assert_eq!(
        l.exposed().iter().collect::<Vec<_>>(),
        vec!["finish_task"],
        "the wrap-up turn can only report"
    );
    assert_eq!(
        l.working().goal,
        "find whether BTC-USD has momentum at 1h",
        "and it wraps up holding its working state, not empty-handed"
    );
}

/// Three consecutive failures from one tool withdraw it for the rest of the run,
/// and the model is told once.
///
/// An agent burning fifteen steps on a tool the platform has broken is the
/// platform's fault, and absorbing that silently charges a platform defect to the
/// agent's budget.
#[test]
fn a_tool_that_fails_three_times_running_is_withdrawn_and_announced_once() {
    let (mut l, _) = started();

    let mut notices = 0;
    for i in 0..4 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"get_bars"}"#.into(),
        });
        l.next(Input::ModelReply {
            raw: format!(r#"{{"instrument":"BTC-USD","nonce":{i}}}"#),
        });
        let fx = l.next(Input::ToolError {
            detail: "http_500".into(),
        });
        notices += fx
            .iter()
            .filter(|e| match e {
                Effect::Note(harness::drive::Note::Degraded(d)) => {
                    format!("{d:?}").contains("withdrawn")
                }
                _ => false,
            })
            .count();
        if i < 2 {
            assert!(
                l.exposed().contains("get_bars"),
                "two failures is bad luck, not a broken tool (failure {i})"
            );
        }
    }

    assert_eq!(notices, 1, "told once, not once per step");
    assert!(
        !l.exposed().contains("get_bars"),
        "the withdrawn tool is gone from exposure: {:?}",
        l.exposed()
    );
}

/// And one success clears the run. What the breaker looks for is a tool that
/// cannot work, not one that sometimes does not.
#[test]
fn a_success_resets_the_failure_run() {
    let (mut l, _) = started();
    for i in 0..2 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"get_bars"}"#.into(),
        });
        l.next(Input::ModelReply {
            raw: format!(r#"{{"instrument":"BTC-USD","nonce":{i}}}"#),
        });
        l.next(Input::ToolError {
            detail: "http_500".into(),
        });
    }
    step(&mut l, 99, "{\"bars\": 500}");
    for i in 2..4 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"get_bars"}"#.into(),
        });
        l.next(Input::ModelReply {
            raw: format!(r#"{{"instrument":"BTC-USD","nonce":{i}}}"#),
        });
        l.next(Input::ToolError {
            detail: "http_500".into(),
        });
    }
    assert!(
        l.exposed().contains("get_bars"),
        "two before a success and two after is not three in a row"
    );
}

// ── S-1.17: reflect costs a call, not a step ────────────────────────────────

/// A reflection does not decrement the step budget, and is capped.
///
/// The mechanism is the whole point: a fifteen-step budget that spends 7% of
/// itself on noticing a wrong turn is a budget that teaches the model not to
/// notice, and the runs that went wrong went wrong early and kept going.
///
/// Capped because the opposite failure is cheap to reach — reflection costs a
/// model call and produces no evidence, so a run that can always take one more is
/// a run that thinks about thinking.
#[test]
fn a_reflection_costs_a_call_but_not_a_step_and_runs_out() {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.reflection = true;
    p.orchestration.max_reflections_per_run = 2;
    p.orchestration.max_steps = 40;
    let mut l = Loop::new(p, registry(), task());
    l.next(Input::Admission(Admission::Admit));

    let before = l.step();
    for i in 0..2 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"reflect"}"#.into(),
        });
        let fx = l.next(Input::ModelReply {
            raw: serde_json::json!({ "doubt": format!("is 1h the right timeframe, take {i}") })
                .to_string(),
        });
        assert!(
            fx.iter().any(|e| matches!(e, Effect::CallModel(_))),
            "a reflection costs a model call"
        );
        assert_eq!(l.step(), before, "and does not spend a step");
    }
    assert_eq!(
        l.working().open_questions.len(),
        2,
        "the doubt lands on the card as an OPEN question, not as a result"
    );

    // The third is refused, and refusing is not a failure state.
    l.next(Input::ModelReply {
        raw: r#"{"name":"reflect"}"#.into(),
    });
    let fx = l.next(Input::ModelReply {
        raw: r#"{"doubt":"one more"}"#.into(),
    });
    let refused = fx.iter().any(|e| {
        matches!(e, Effect::Note(harness::drive::Note::Rejected { code, .. })
            if &**code == "drive.reflection_exhausted")
    });
    assert!(refused, "the third reflection is refused: {fx:?}");
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Done(_))),
        "and refusing does not end the run"
    );
}

/// A profile that does not reflect is never offered the tool.
///
/// It is registered so the loop can ANSWER it; offering it to a 7B would spend one
/// of six exposure slots on a tool whose handler refuses — and on that profile
/// that is the slot `list_instruments` loses to two alphabetical siblings.
#[test]
fn a_profile_that_does_not_reflect_is_not_offered_the_tool() {
    let (l, _) = started();
    assert!(
        !profile().orchestration.reflection,
        "the 7B does not reflect"
    );
    assert!(
        !l.exposed().contains("reflect"),
        "and is not offered it: {:?}",
        l.exposed()
    );
}

/// The worked examples reach the prompt on a profile that asked for them, and the
/// count is the count.
#[test]
fn few_shot_examples_reach_the_prompt_on_the_profiles_that_declare_them() {
    let (_, fx) = started();
    let p = profile();
    assert_eq!(p.orchestration.few_shot_examples, 2, "the 7B declares two");

    let prompt = prompt(&fx);
    assert!(
        prompt.contains("WORKED EXAMPLES"),
        "a declared value that never reaches a prompt is a bug: {prompt}"
    );
    assert_eq!(
        prompt.matches("you are asked to choose").count(),
        2,
        "two means two"
    );
}

/// And a frontier profile gets none: two examples in the prefix is two examples
/// paid for on every turn, and a frontier model does not need the format help.
#[test]
fn a_frontier_profile_gets_no_examples() {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.few_shot_examples = 0;
    let mut l = Loop::new(p, registry(), task());
    let fx = l.next(Input::Admission(Admission::Admit));
    assert!(!prompt(&fx).contains("WORKED EXAMPLES"));
}

// ── S-1.19: a wrong call is corrected, not retried ──────────────────────────

/// A tool error spends no retry budget, whatever it says.
///
/// The division is the point. The retry budget governs how many times the MODEL
/// may be corrected; the breaker governs how long the run tolerates a TOOL that
/// does not work. Charging a platform failure to the model's allowance is how a
/// run ends unable to answer because the platform was broken — measured once as a
/// degraded-tier session that called one tool eleven times and died with its
/// budget spent on the platform's mistake.
#[test]
fn a_non_retryable_tool_error_spends_no_retry_budget() {
    let (mut l, _) = started();

    // Two typed, non-retryable failures. Neither is a retry.
    for i in 0..2 {
        l.next(Input::ModelReply {
            raw: r#"{"name":"get_bars"}"#.into(),
        });
        l.next(Input::ModelReply {
            raw: format!(r#"{{"instrument":"BTC-USD","nonce":{i}}}"#),
        });
        let fx = l.next(Input::ToolError {
            detail: r#"{"error":{"code":"invalid_request","message":"no such instrument","fix":"name one from list_instruments","retryable":false}}"#.into(),
        });
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::Done(_))),
            "a tool failing is an observation, not the end of the run"
        );
    }

    // The budget was never touched, so a genuine SEMANTIC rejection still has its
    // full allowance and is corrected rather than fenced.
    //
    // A semantic one deliberately: a schema violation under a grammar fences by
    // design, because the sampler could not have produced it and that says
    // something about the backend rather than about the model. A fabricated
    // citation is the model being wrong, which is what the retry budget is for.
    l.next(Input::ModelReply {
        raw: r#"{"name":"record_finding"}"#.into(),
    });
    let fx = l.next(Input::ModelReply {
        raw: r#"{"finding":"BTC-USD has 500 bars","evidence":"run_deadbeef01"}"#.into(),
    });
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::Done(_))),
        "the retry budget is intact, so the model is corrected rather than fenced: {fx:?}"
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Note(harness::drive::Note::Rejected { .. }))),
        "and the correction is the thing it gets: {fx:?}"
    );
}

/// And the flag reaches the model, because being told "this will fail again" is
/// the whole reason a tool declares it.
#[test]
fn a_non_retryable_error_tells_the_model_not_to_repeat_it() {
    let (mut l, _) = started();
    l.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    l.next(Input::ModelReply {
        raw: r#"{"instrument":"NOPE"}"#.into(),
    });
    let fx = l.next(Input::ToolError {
        detail: r#"{"error":{"code":"invalid_request","message":"no such instrument","fix":"name one from list_instruments","retryable":false}}"#.into(),
    });
    let typed = prompt(&fx);
    assert!(
        typed.contains("This will fail again with these arguments"),
        "the advice reaches the prompt: {typed}"
    );

    // An untyped error says nothing about retrying, and the harness does not
    // invent an answer — most of the catalogue is still untyped.
    let (mut l2, _) = started();
    l2.next(Input::ModelReply {
        raw: r#"{"name":"get_bars"}"#.into(),
    });
    l2.next(Input::ModelReply {
        raw: r#"{"instrument":"NOPE"}"#.into(),
    });
    let fx = l2.next(Input::ToolError {
        detail: "http_500".into(),
    });
    let untyped = prompt(&fx);
    assert!(!untyped.contains("This will fail again"));
    assert!(!untyped.contains("may be transient"));
}

// ── S-2.4: session-scoped exposure is append-only ───────────────────────────

/// A registry with a skill tool the goal never mentions, so only a search finds it.
fn registry_with_skills() -> ToolRegistry {
    let mut reg = registry();
    for (name, ns, desc) in [
        (
            "run_skill",
            "skills",
            "Run a saved Python skill against a payload. Use it to run python.",
        ),
        (
            "list_skills",
            "skills",
            "Every saved skill with its record.",
        ),
        (
            "get_funnel",
            "research",
            "The gate funnel for an experiment.",
        ),
        ("advance_gate", "research", "Advance an experiment's gate."),
        ("list_lanes", "discovery", "Which lanes exist."),
    ] {
        reg.register(ToolDef {
            name: name.into(),
            namespace: ns.into(),
            description: desc.into(),
            risk: Risk::Read,
            core: false,
            idempotent: true,
            input_schema: json!({"type": "object", "properties": {}}),
        })
        .unwrap();
    }
    reg
}

fn session_loop(budget: usize) -> (Loop, Vec<Effect>) {
    let mut p = profile();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.max_steps = 40;
    p.tools.exposure_scope = harness::profile::ExposureScope::Session;
    p.tools.max_exposed_per_step = budget;
    p.tools.search_reserve_slots = 1;
    let mut l = Loop::new(p, registry_with_skills(), task());
    let fx = l.next(Input::Admission(Admission::Admit));
    (l, fx)
}

/// Within a turn the slice only grows, and its order is a strict PREFIX of the
/// previous call's.
///
/// The order is load-bearing rather than cosmetic: a cached prefix matches by
/// bytes, so reordering the tools array invalidates it exactly as thoroughly as
/// removing an entry. A slice that re-sorted itself every step would pay the full
/// prompt on all thirty iterations while looking, from the outside, like caching.
#[test]
fn a_session_slice_is_append_only_and_each_order_is_a_prefix_of_the_next() {
    let (mut l, _) = session_loop(8);
    let mut previous: Vec<String> = l.exposed().iter().cloned().collect();
    assert!(!previous.is_empty());

    for i in 0..6u32 {
        step(&mut l, i, "{\"bars\": 500}");
        let now: Vec<String> = l.exposed().iter().cloned().collect();
        assert!(
            now.len() >= previous.len(),
            "the slice shrank: {previous:?} -> {now:?}"
        );
        for name in &previous {
            assert!(
                now.contains(name),
                "`{name}` left the slice mid-turn; eviction invalidates the cached prefix"
            );
        }
        previous = now;
    }
}

/// A goal that never says "skill", a search for "run python", and the tool is in
/// the slice on the next call — one round-trip, no eviction.
#[test]
fn a_searched_tool_joins_the_session_slice_in_one_round_trip() {
    let (mut l, _) = session_loop(8);
    assert!(
        !l.exposed().contains("run_skill"),
        "the goal never mentions skills, so routing did not offer it: {:?}",
        l.exposed()
    );

    l.next(Input::ModelReply {
        raw: r#"{"name":"search_tools"}"#.into(),
    });
    l.next(Input::ModelReply {
        raw: r#"{"query":"run python"}"#.into(),
    });

    assert!(
        l.exposed().contains("run_skill"),
        "the searched tool is in the slice on the very next call: {:?}",
        l.exposed()
    );
}

/// A full slice says so and keeps what it has.
///
/// Dropping a tool to make room for a better one is a trade that looks free at the
/// moment it is made and is paid for by every later call in the turn.
#[test]
fn a_full_session_slice_reports_rather_than_evicting() {
    // Small enough that core plus one routed tool fills it.
    let (mut l, _) = session_loop(5);
    let before: Vec<String> = l.exposed().iter().cloned().collect();

    let mut reported = false;
    for query in ["run python", "the gate funnel", "which lanes exist"] {
        l.next(Input::ModelReply {
            raw: r#"{"name":"search_tools"}"#.into(),
        });
        let fx = l.next(Input::ModelReply {
            raw: json!({ "query": query }).to_string(),
        });
        reported |= fx.iter().any(|e| {
            matches!(e, Effect::Note(harness::drive::Note::CoreTool { name, .. })
                if name == "exposure_full")
        });
    }

    assert!(l.exposed().len() <= 5, "the cap held: {:?}", l.exposed());
    for name in &before {
        assert!(
            l.exposed().contains(name),
            "`{name}` was evicted to make room, which is what this forbids"
        );
    }
    assert!(reported, "a full slice reports `exposure_full`");
}

/// And a step-scoped profile still re-routes, because it has no cache to protect
/// and six slots to spend on the step in front of it.
#[test]
fn a_step_scoped_profile_still_reroutes_every_step() {
    let (l, _) = started();
    assert_eq!(
        profile().tools.exposure_scope,
        harness::profile::ExposureScope::Step,
        "the 7B is step-scoped"
    );
    assert!(!l.exposed().is_empty());
}

// ── S-2.2: one loop, two transports ───────────────────────────────────────/// The same goal on both transports asks the world for the same things.
///
/// This is the property that makes "one runtime" true rather than aspirational. If
/// the two produced different sequences of world-facing effects, there would be two
/// loops wearing one name, and every guarantee proved on one would have to be
/// re-proved on the other.
///
/// **`CallModel` is excluded, and that is the point rather than a weakening.** The
/// two transports differ in exactly one place by design: `select_fill` spends two
/// model calls per step (choose a tool, then fill its arguments) where `native`
/// spends one, because a grammar cannot constrain a choice and an object at the same
/// time. That difference is the adapter's whole job. What must be identical is the
/// sequence of things the loop asks the *world* to do — the tools it runs, the
/// approvals it seeks, the outcome it reaches — because those are what a guarantee
/// is about.
#[test]
fn both_transports_ask_the_world_for_the_same_things() {
    fn world_effects(constrained: bool) -> Vec<String> {
        let mut p = profile();
        p.orchestration.mode = harness::profile::Mode::Freeform;
        p.orchestration.max_steps = 40;
        p.output.constrained_decoding = constrained;
        let mut l = Loop::new(p, registry(), task());

        let mut seen: Vec<String> = Vec::new();
        let mut record = |fx: &[Effect]| {
            for e in fx {
                match e {
                    // The one legitimate difference, and notes are the loop talking
                    // to its own trace rather than to the world.
                    // `Remember` joins them: it is the loop handing the
                    // platform a row to store, not a decision about the world,
                    // and a transport that produced a different set of *those*
                    // would not be a transport difference this test is about.
                    // `Spill` joins them for the same reason as `Remember`: it
                    // is the loop asking the platform to hold some bytes, and
                    // residency is a property of how long a run has been going
                    // rather than of which transport it is on.
                    Effect::CallModel(_)
                    | Effect::Note(_)
                    | Effect::Remember(_)
                    | Effect::Spill { .. } => {}
                    Effect::ExecuteTool { name, .. } => seen.push(format!("run:{name}")),
                    Effect::AskHuman { tool, .. } => seen.push(format!("ask:{tool}")),
                    Effect::Await { tool, .. } => seen.push(format!("await:{tool}")),
                    Effect::Done(_) => seen.push("done".into()),
                }
            }
        };

        record(&l.next(Input::Admission(Admission::Admit)));
        // Each transport's own reply shape. A native reply is one object carrying
        // the name and the arguments together; `select_fill` is two constrained
        // objects, because a grammar cannot pin a choice and an object at once.
        if constrained {
            record(&l.next(Input::ModelReply {
                raw: r#"{"name":"get_bars"}"#.into(),
            }));
            record(&l.next(Input::ModelReply {
                raw: r#"{"instrument":"BTC-USD"}"#.into(),
            }));
        } else {
            record(&l.next(Input::ModelReply {
                raw: r#"{"name":"get_bars","arguments":{"instrument":"BTC-USD"}}"#.into(),
            }));
        }
        record(&l.next(Input::ToolResult {
            output: "{\"bars\": 500}".into(),
            source: "platform.data".into(),
        }));
        seen
    }

    let native = world_effects(false);
    let select_fill = world_effects(true);
    assert_eq!(
        native, select_fill,
        "the two transports must ask the world for the same things in the same order;
           native:      {native:?}
  select_fill: {select_fill:?}"
    );
    assert!(
        native.iter().any(|e| e == "run:get_bars"),
        "and the run actually got somewhere: {native:?}"
    );
}

// ── S-2.14: steering ────────────────────────────────────────────────────────

/// A redirection re-plans and **keeps the findings**.
///
/// The split is the design. A finding was true about the *data*, and changing the
/// question does not make it false — throwing findings away on every steer would
/// teach the user not to steer. The plan is the opposite: it was built for the old
/// question, and every remaining step serves it.
#[test]
fn a_redirection_replans_and_keeps_what_was_established() {
    use harness::state::GoalKind;

    let (mut l, _) = started();
    step(&mut l, 0, "{\"bars\": 500}");
    l.next(Input::ModelReply {
        raw: r#"{"name":"record_finding"}"#.into(),
    });
    l.next(Input::ModelReply {
        raw: r#"{"finding":"BTC-USD has 500 bars at 1h","evidence":"get_bars"}"#.into(),
    });
    assert_eq!(l.working().findings.len(), 1, "one thing established");

    let fx = l.next(Input::Steer {
        text: "actually, forget BTC — do the same for ETH-USD".into(),
    });

    assert_eq!(
        l.working().findings.len(),
        1,
        "the finding survives: it was true about the data, not about the goal"
    );
    assert!(
        l.working().goal.contains("ETH-USD"),
        "and the goal is the new one: {}",
        l.working().goal
    );
    assert!(
        l.working().plan.is_empty(),
        "while the plan, which served the old question, is dropped"
    );
    assert_eq!(l.working().kind, GoalKind::Lookup, "and re-classified");

    // The old goal and the reason survive, so the run can say what it was doing.
    assert!(
        l.working()
            .decided
            .iter()
            .any(|d| d.question.contains("momentum") && d.reason.contains("redirected")),
        "the old goal and why it changed are on the card: {:?}",
        l.working().decided
    );
    assert!(fx.iter().any(|e| matches!(e, Effect::CallModel(_))));
}

/// A constraint narrows without re-planning. Re-planning a healthy run because the
/// user said "only 1h" costs a step and loses the plan for nothing.
#[test]
fn a_constraint_narrows_without_dropping_the_plan() {
    use harness::state::Steer;
    assert_eq!(Steer::classify("only use 1h bars"), Steer::Constraint);
    assert_eq!(Steer::classify("focus on BTC-USD"), Steer::Constraint);
    assert_eq!(
        Steer::classify("just use the coinbase data"),
        Steer::Constraint
    );

    let (mut l, _) = started();
    let goal = l.working().goal.clone();
    l.next(Input::Steer {
        text: "only use 1h bars".into(),
    });
    assert_eq!(l.working().goal, goal, "same question, smaller");
}

/// Ambiguity resolves to `Redirection`, and the asymmetry is the whole design: an
/// unnecessary re-plan costs one step, and a missed one costs a run aimed at the
/// wrong question.
#[test]
fn an_ambiguous_steer_is_read_as_a_redirection_and_says_so() {
    use harness::state::Steer;

    let odd = "what about volatility clustering";
    assert_eq!(Steer::classify(odd), Steer::Redirection);
    let correction = Steer::Redirection.correction(odd);
    assert!(
        correction.contains("If this was only a narrowing"),
        "the model is told which reading was taken, so a wrong call is one sentence \
         to correct rather than something to discover at the end: {correction}"
    );
    assert!(
        correction.contains("findings so far are KEPT"),
        "and told the findings survive, or it will re-derive them: {correction}"
    );
}

/// A comment has to be recognised, not defaulted to. Treating "looking good" as a
/// redirection would re-plan a healthy run.
#[test]
fn encouragement_is_not_a_new_goal() {
    use harness::state::Steer;
    for text in [
        "looking good",
        "nice, keep going",
        "thanks",
        "how's it going",
    ] {
        assert_eq!(Steer::classify(text), Steer::Comment, "{text}");
    }
    assert_eq!(
        Steer::classify("   "),
        Steer::Comment,
        "silence is not a steer"
    );
}
