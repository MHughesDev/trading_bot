//! Waiting on a job costs nothing (AGENT-010 §8, P5; Set S S-2.11).
//!
//! The thing being replaced is two polling loops inside the hosted driver that held
//! a model connection and wrote a status row every thirty seconds for as long as a
//! sweep took. The acceptance is not "the answer arrives" — a polling loop produces
//! the right answer too. It is that **no tokens are spent while waiting**, and the
//! only way to assert that is to show the loop emits no model call at all.

use serde_json::json;

use harness::drive::{Effect, Input, Loop, Task};
use harness::hardware::{Admission, TaskDemand};
use harness::profile::{Profile, ProfileSet};
use harness::registry::{self, Risk, ToolDef, ToolRegistry};

fn profile() -> Profile {
    let mut p = ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
        .expect("the shipped profiles load")
        .get("qwen2.5:7b-instruct")
        .expect("the degraded profile ships")
        .clone();
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.max_steps = 20;
    p
}

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::with_core();
    reg.register(ToolDef {
        name: "run_sweep".into(),
        namespace: "research".into(),
        description: "Sweep a parameter grid. Use when the step needs a search.".into(),
        risk: Risk::Write,
        core: false,
        idempotent: false,
        input_schema: json!({
            "type": "object",
            "properties": {"strategy": {"type": "string"}},
            "required": ["strategy"],
        }),
    })
    .unwrap();
    reg
}

fn suspended() -> (Loop, Vec<Effect>) {
    let mut l = Loop::new(
        profile(),
        registry(),
        Task {
            goal: "sweep the momentum parameters".into(),
            charter: "You are a research agent.".into(),
            demand: TaskDemand::MultiStep,
            namespaces: vec!["research".into()],
        },
    );
    l.next(Input::Admission(Admission::Admit));
    l.next(Input::ModelReply {
        raw: r#"{"name":"run_sweep"}"#.into(),
    });
    l.next(Input::ModelReply {
        raw: r#"{"strategy":"ema_cross"}"#.into(),
    });
    // The driver dispatched it, the platform started a job and handed back a handle.
    let fx = l.next(Input::JobAccepted {
        job_id: "job-7".into(),
    });
    (l, fx)
}

fn has_model_call(fx: &[Effect]) -> bool {
    fx.iter().any(|e| matches!(e, Effect::CallModel(_)))
}

/// The acceptance, stated directly.
#[test]
fn a_run_waiting_on_a_job_emits_no_model_call() {
    let (l, fx) = suspended();
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Await { job_id, tool, .. }
                if job_id == "job-7" && tool == "run_sweep")),
        "the loop suspends on the job it was handed: {fx:?}"
    );
    assert!(
        !has_model_call(&fx),
        "suspending must not cost a model call: {fx:?}"
    );
    assert_eq!(l.state(), harness::drive::State::Waiting);
}

/// Adversarial: nothing a driver can send while waiting buys a model call.
///
/// A loop that could be *nudged* into a call while suspended would spend tokens for
/// a run that is doing nothing, which is exactly the failure being removed. The only
/// input that restarts it is the job's completion.
#[test]
fn nothing_short_of_the_job_finishing_restarts_a_waiting_run() {
    for input in [
        Input::ToolResult {
            output: "done".into(),
            source: "platform.research".into(),
        },
        Input::ToolError {
            detail: "still going".into(),
        },
        Input::ModelReply {
            raw: r#"{"name":"finish_task"}"#.into(),
        },
        Input::ModelTruncated { raw: String::new() },
        Input::ModelError {
            detail: "blip".into(),
            retryable: true,
        },
        Input::Approval {
            granted: true,
            note: String::new(),
        },
        Input::JobAccepted {
            job_id: "job-8".into(),
        },
        Input::Admission(Admission::Admit),
    ] {
        let (mut l, _) = suspended();
        let name = format!("{input:?}");
        let fx = l.next(input);
        assert!(
            !has_model_call(&fx),
            "{name} bought a model call from a suspended run: {fx:?}"
        );
    }
}

/// A result from the wrong job must not be folded in as though it were this run's.
///
/// It would attribute one question's numbers to another and never say so, which is
/// worse than failing.
#[test]
fn another_jobs_result_does_not_resume_this_run() {
    let (mut l, _) = suspended();
    let fx = l.next(Input::JobDone {
        job_id: "job-999".into(),
        output: "sharpe 3.0".into(),
        source: "platform.research".into(),
        ok: true,
    });
    assert!(!has_model_call(&fx));
    assert!(
        matches!(
            l.outcome(),
            Some(harness::drive::Outcome::Fenced(
                harness::drive::Degradation::BackendFailure { .. }
            ))
        ),
        "got {:?}",
        l.outcome()
    );
}

/// And when it does finish, the run continues exactly as if the tool had answered
/// immediately: same attribution, same plan advance, same evidence.
#[test]
fn the_job_result_is_the_tool_call_s_result_arriving_late() {
    let (mut l, _) = suspended();
    let fx = l.next(Input::JobDone {
        job_id: "job-7".into(),
        output: "sweep-00001234 best sharpe 1.2".into(),
        source: "platform.research".into(),
        ok: true,
    });
    assert!(
        has_model_call(&fx),
        "the completion is what buys the next model call: {fx:?}"
    );
    assert!(
        l.working().can_cite("sweep-00001234"),
        "the job's ids are citable, attributed to the tool that started it"
    );
    assert_eq!(
        l.working().evidence[0].tool,
        "run_sweep",
        "attributed to the call, not to the job service"
    );
}

/// A failed job is information, not a dead run — the same rule as a failed tool.
#[test]
fn a_failed_job_is_handed_to_the_model() {
    let (mut l, _) = suspended();
    let fx = l.next(Input::JobDone {
        job_id: "job-7".into(),
        output: "the worker died".into(),
        source: "platform.research".into(),
        ok: false,
    });
    assert!(has_model_call(&fx), "{fx:?}");
    assert!(l.outcome().is_none(), "a failed job does not end the run");
}

/// The list the driver and the loop have to agree on.
#[test]
fn the_long_running_tools_are_the_four_the_spec_names() {
    for t in [
        "run_sweep",
        "run_study",
        "train_model",
        "run_shadow_session",
    ] {
        assert!(registry::is_long_running(t), "{t}");
    }
    assert!(!registry::is_long_running("get_bars"));
    assert!(!registry::is_long_running("finish_task"));
}
