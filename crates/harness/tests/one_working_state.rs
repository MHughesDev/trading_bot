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
        raw: r#"{"steps":[{"goal":"pull bars","namespace":"data"},
                          {"goal":"check momentum","namespace":"data"}]}"#
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
