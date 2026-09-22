//! Killing a run mid-write and resuming it produces ONE write, not two.
//!
//! AGENT-010 §5 and §14.3; Set S S-2.6. This is the acceptance that is easiest to
//! fake — every resume implementation "works" on the happy path, because the happy
//! path is the one where nothing was in flight — so this file builds the unhappy one
//! deliberately: a driver that has written the effect down, dispatched the write,
//! and died before the result came back.
//!
//! The fake driver here is the real protocol, in miniature: log the effect before
//! doing it, log the input before feeding it back, and on resume replay the inputs
//! through a fresh loop. Nothing about it is generous to the implementation — in
//! particular `duplicates_the_write_without_the_guard` runs the same resume with the
//! guard removed and asserts it produces two, which is what makes the guard
//! load-bearing rather than decorative.

use std::collections::VecDeque;

use serde_json::json;

use harness::drive::{Effect, Input, Loop, Outcome, Task};
use harness::hardware::{Admission, TaskDemand};
use harness::profile::{Profile, ProfileSet};
use harness::registry::{Risk, ToolDef, ToolRegistry};
use harness::resume::{self, LoggedEffect, RefuseRepeat, Row};

fn profile() -> Profile {
    let mut p = ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
        .expect("the shipped profiles load")
        .get("qwen2.5:7b-instruct")
        .expect("the degraded profile ships")
        .clone();
    // Freeform so the scripted replies are select/fill pairs with no plan turn, and
    // chat_assistant's attendance is what lets a write run without an approval —
    // the approval path is a different test.
    p.orchestration.mode = harness::profile::Mode::Freeform;
    p.orchestration.max_steps = 20;
    p
}

fn registry() -> ToolRegistry {
    let mut reg = ToolRegistry::with_core();
    reg.register(ToolDef {
        name: "create_experiment".into(),
        namespace: "research".into(),
        description: "Create an experiment. Use when the step needs one recorded.".into(),
        risk: Risk::Write,
        core: false,
        idempotent: false,
        input_schema: json!({
            "type": "object",
            "properties": {"goal": {"type": "string"}},
            "required": ["goal"],
        }),
    })
    .unwrap();
    reg.register(ToolDef {
        name: "get_bars".into(),
        namespace: "data".into(),
        description: "Read price bars. Use when the step needs price history.".into(),
        risk: Risk::Read,
        core: false,
        idempotent: true,
        input_schema: json!({
            "type": "object",
            "properties": {"instrument": {"type": "string"}},
            "required": ["instrument"],
        }),
    })
    .unwrap();
    reg
}

fn task() -> Task {
    Task {
        goal: "record an experiment for the momentum question".into(),
        charter: "You are a research agent.".into(),
        demand: TaskDemand::MultiStep,
        namespaces: vec!["research".into()],
    }
}

/// The write-ahead protocol, small enough to read.
struct Driver {
    l: Loop,
    log: Vec<Row>,
    /// Every tool the driver actually performed, in order. The thing under test.
    dispatched: Vec<String>,
    replies: VecDeque<String>,
    /// Dispatch number at which the process dies — after the effect is logged and
    /// the tool has run, before its result is recorded. The worst moment, chosen on
    /// purpose.
    die_after_dispatch: Option<usize>,
    refuse: Option<RefuseRepeat>,
    dead: bool,
}

impl Driver {
    fn new(replies: &[&str]) -> Self {
        Self {
            l: Loop::new(profile(), registry(), task()),
            log: Vec::new(),
            dispatched: Vec::new(),
            replies: replies.iter().map(|s| (*s).to_string()).collect(),
            die_after_dispatch: None,
            refuse: None,
            dead: false,
        }
    }

    /// Write the input down, then feed it in. Both halves, in that order.
    fn feed(&mut self, input: Input) -> Vec<Effect> {
        self.log.push(Row::Input(input.clone()));
        self.l.next(input)
    }

    fn pump(&mut self, mut effects: Vec<Effect>) {
        loop {
            if self.dead {
                return;
            }
            let mut blocking = None;
            for e in &effects {
                // Write-ahead: the effect is durable before anything acts on it.
                self.log.push(Row::Effect(LoggedEffect::of(e)));
                match e {
                    Effect::Note(_) => {}
                    Effect::Done(_) => return,
                    other => blocking = Some(other.clone()),
                }
            }
            let Some(blocking) = blocking else { return };

            let input = match blocking {
                Effect::CallModel(_) => Input::ModelReply {
                    raw: self.replies.pop_front().expect("a scripted reply"),
                },
                ref exec @ Effect::ExecuteTool { ref name, .. } => {
                    if self.refuse.as_ref().is_some_and(|r| r.matches(exec)) {
                        // The one dispatch a resumed driver must not perform.
                        let refusal = self.refuse.take().expect("checked").tool_error();
                        effects = self.feed(refusal);
                        continue;
                    }
                    self.dispatched.push(name.clone());
                    if self.die_after_dispatch == Some(self.dispatched.len() - 1) {
                        // The process ends here. The effect is in the log; the
                        // result never reaches it.
                        self.dead = true;
                        return;
                    }
                    Input::ToolResult {
                        output: format!("{name} ok: exp-00001234"),
                        source: "platform.research".into(),
                    }
                }
                other => panic!("this fixture does not drive {other:?}"),
            };
            effects = self.feed(input);
        }
    }

    fn start(&mut self) {
        let fx = self.feed(Input::Admission(Admission::Admit));
        self.pump(fx);
    }

    /// A fresh process folding the log back, exactly as `AgentManager` will.
    fn resume_from(log: &[Row], replies: &[&str], honour_guard: bool) -> Driver {
        let plan = resume::plan(
            log,
            &resume::Recorded {
                harness_version: resume::HARNESS_VERSION.to_string(),
                log_schema: resume::EFFECT_LOG_SCHEMA_VERSION,
            },
        )
        .expect("the same build folds its own log back");

        let mut d = Driver::new(replies);
        d.refuse = honour_guard.then_some(plan.refuse_repeat.clone()).flatten();
        let mut effects = Vec::new();
        for input in plan.inputs {
            // The replayed inputs are not re-logged: they are already in the log.
            effects = d.l.next(input);
        }
        d.pump(effects);
        d
    }
}

const REPLIES: &[&str] = &[
    r#"{"name":"create_experiment"}"#,
    r#"{"goal":"momentum"}"#,
    r#"{"name":"finish_task"}"#,
    r#"{"result":"recorded the experiment","evidence":"exp-00001234"}"#,
];

/// The acceptance, stated as the spec states it.
#[test]
fn a_killed_run_resumes_with_no_duplicated_write_effect() {
    let mut first = Driver::new(REPLIES);
    first.die_after_dispatch = Some(0);
    first.start();
    assert_eq!(
        first.dispatched,
        vec!["create_experiment"],
        "the write went out once before the process died"
    );

    // A fresh process, the same log. The model's next move after being told the
    // call was interrupted: report what it has.
    let resumed = Driver::resume_from(
        &first.log,
        &[
            r#"{"name":"finish_task"}"#,
            r#"{"result":"the experiment may already exist","evidence":"create_experiment was interrupted"}"#,
        ],
        true,
    );

    let writes = first.dispatched.len() + resumed.dispatched.len();
    assert_eq!(
        writes, 1,
        "one instruction produced {writes} writes; resumed driver dispatched {:?}",
        resumed.dispatched
    );
    assert!(
        matches!(resumed.l.outcome(), Some(Outcome::Finished { .. })),
        "and the run still finishes: {:?}",
        resumed.l.outcome()
    );
}

/// The guard is load-bearing, and this is the proof.
///
/// Same log, same replay, guard removed — and the write goes out a second time. If
/// this test ever passes with one write, the resume is not being protected by the
/// mechanism this phase claims; something else is accidentally suppressing it, and
/// the accident would not survive the next refactor.
#[test]
fn duplicates_the_write_without_the_guard() {
    let mut first = Driver::new(REPLIES);
    first.die_after_dispatch = Some(0);
    first.start();

    let unguarded = Driver::resume_from(&first.log, REPLIES, false);
    assert_eq!(
        first.dispatched.len() + unguarded.dispatched.len(),
        2,
        "a resume that ignores the guard must be shown to duplicate, or the guard \
         is not what is preventing it"
    );
}

/// The model is told, in its own channel, and can act on it.
#[test]
fn the_model_is_told_the_call_was_interrupted_because_it_writes() {
    let mut first = Driver::new(REPLIES);
    first.die_after_dispatch = Some(0);
    first.start();

    let plan = resume::plan(
        &first.log,
        &resume::Recorded {
            harness_version: resume::HARNESS_VERSION.to_string(),
            log_schema: resume::EFFECT_LOG_SCHEMA_VERSION,
        },
    )
    .unwrap();
    let Input::ToolError { detail } = plan
        .refuse_repeat
        .expect("an unfinished write is the case")
        .tool_error()
    else {
        panic!("it reaches the model as a tool error, not as a silent skip");
    };
    assert!(detail.contains("interrupted; not retried because it writes"));
}

/// A read interrupted the same way IS re-run, and nothing is lost by it.
#[test]
fn an_interrupted_read_is_re_run_rather_than_refused() {
    let mut first = Driver::new(&[r#"{"name":"get_bars"}"#, r#"{"instrument":"BTC-USD"}"#]);
    first.die_after_dispatch = Some(0);
    first.start();
    for r in &first.log {
        eprintln!("ROW {r:?}");
    }
    assert_eq!(first.dispatched, vec!["get_bars"]);

    let resumed = Driver::resume_from(
        &first.log,
        &[
            r#"{"name":"finish_task"}"#,
            r#"{"result":"read the bars","evidence":"get_bars ok: exp-00001234"}"#,
        ],
        true,
    );
    assert_eq!(
        resumed.dispatched,
        vec!["get_bars"],
        "a read that may not have completed is simply done again"
    );
}

/// Replay is a fold, not a re-run: the model is not asked anything it was already
/// asked, and the work already recorded is still recorded.
#[test]
fn resuming_does_not_re_ask_the_model_what_it_already_answered() {
    let mut first = Driver::new(REPLIES);
    first.die_after_dispatch = Some(0);
    first.start();
    let model_calls_before = first
        .log
        .iter()
        .filter(|r| matches!(r, Row::Effect(LoggedEffect::CallModel { .. })))
        .count();

    // Only two replies are supplied to the resumed driver. If the replay re-asked
    // the questions the first process already answered, it would run out of replies
    // and panic — which is a sharper assertion than counting.
    let resumed = Driver::resume_from(
        &first.log,
        &[
            r#"{"name":"finish_task"}"#,
            r#"{"result":"done","evidence":"create_experiment was interrupted"}"#,
        ],
        true,
    );
    assert_eq!(model_calls_before, 2, "select and fill");
    assert!(resumed.l.outcome().is_some());
}
