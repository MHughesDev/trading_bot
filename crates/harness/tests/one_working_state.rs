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
        step(&mut l, i, &format!("{{\"bars\": {}, \"run\": \"run_{i}\"}}", 500 + i));
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
        l.working().decided.iter().any(|d| d.question.contains("get_bars")),
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
    assert_eq!(GoalKind::classify("how many 1m bars do we hold for BTC-USD", &[]), GoalKind::Lookup);
    assert_eq!(
        GoalKind::classify("backtest an ema cross on BTC-USD and report the sharpe", &[]),
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
    assert_eq!(GoalKind::classify("what is the sharpe of run_42", &[]), GoalKind::Lookup);

    // The default, and what it costs.
    assert_eq!(GoalKind::classify("something entirely unfamiliar", &[]), GoalKind::Lookup);
    assert!(!GoalKind::Lookup.may_spend_trials(), "a lookup never spends a trial");
    assert_eq!(GoalKind::Lookup.max_tools(), 3, "and sees at most three tools");
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
            .any(|a| a.contains("every number reported is in a tool result")),
        "and every kind carries the citation criterion: {:?}",
        l.working().acceptance
    );
}
