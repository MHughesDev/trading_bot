//! The effect log IS the run, and a killed process folds it back (AGENT-010 §5, P3).
//!
//! Set S tasks S-2.5 and S-2.6.
//!
//! # The shape
//!
//! The driver writes every [`Effect`] down **before** executing it and every
//! resulting [`Input`] **before** feeding it back. On restart, a fresh [`Loop`] of
//! the same `harness_version` is fed the recorded inputs — the loop is deterministic
//! given its inputs, so it lands in exactly the state it was in — and then carries
//! on. Nothing in this module reads a database; it decides, from a log, what the
//! driver must do next.
//!
//! # The one case that is not automatic
//!
//! Replaying the inputs makes the loop re-emit the effect it was interrupted on.
//! For a model call and for a read that is what you want: re-issue it, re-run it,
//! nothing is lost. For a **write** it is precisely what you must not do — the tool
//! may have created the experiment before the process died, and a resume that runs
//! it again has silently made two.
//!
//! So the plan carries [`Plan::refuse_repeat`]: the one dispatch the driver must
//! answer with a `ToolError` instead of performing. The model reads
//! `"interrupted; not retried because it writes"` and decides for itself whether to
//! check and retry — which is the honest answer, because the harness genuinely does
//! not know whether the write landed.
//!
//! The guard is load-bearing rather than defensive, and the test says so: a driver
//! that ignores it produces two writes from one instruction. That is asserted, not
//! assumed.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::drive::{Effect, Input};
use crate::registry::Risk;

/// The version of the loop's *decisions*.
///
/// Bump it when a change would make a fresh loop, fed the same inputs, do something
/// different. A run recorded under one version cannot be folded back by another: the
/// replay would diverge silently, which is worse than refusing.
pub const HARNESS_VERSION: &str = "1.0.0";

/// The version of the effect log's *shape*.
///
/// Deliberately separate from [`HARNESS_VERSION`] and deliberately slower-moving
/// (AGENT-010 §5). A run suspended on a forty-minute sweep holds no process, so a
/// deploy inside that window is likely rather than exceptional; discarding the sweep
/// because the binary changed underneath it would make the platform's most expensive
/// operation its least reliable. A suspended run resumes on schema compatibility
/// alone.
pub const EFFECT_LOG_SCHEMA_VERSION: u32 = 1;

/// What a recorded effect was, in the form resume needs.
///
/// A projection of [`Effect`] rather than the thing itself: the log has to
/// round-trip through JSON and back, and the fields that matter for resuming are
/// few. Keeping it small is also what keeps the log readable by the UI timeline and
/// by AGENT-009's replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum LoggedEffect {
    CallModel {
        step: u32,
    },
    ExecuteTool {
        step: u32,
        name: String,
        arguments: Map<String, Value>,
        risk: Risk,
        idempotent: bool,
    },
    AskHuman {
        step: u32,
        tool: String,
    },
    Await {
        step: u32,
        job_id: String,
    },
    Note,
    Done,
}

impl LoggedEffect {
    #[must_use]
    pub fn of(effect: &Effect) -> Self {
        match effect {
            Effect::CallModel(c) => LoggedEffect::CallModel { step: c.step },
            Effect::ExecuteTool {
                step,
                name,
                arguments,
                risk,
                idempotent,
                ..
            } => LoggedEffect::ExecuteTool {
                step: *step,
                name: name.clone(),
                arguments: arguments.clone(),
                risk: *risk,
                idempotent: *idempotent,
            },
            Effect::AskHuman { step, tool, .. } => LoggedEffect::AskHuman {
                step: *step,
                tool: tool.clone(),
            },
            Effect::Await { step, job_id, .. } => LoggedEffect::Await {
                step: *step,
                job_id: job_id.clone(),
            },
            Effect::Note(_) => LoggedEffect::Note,
            Effect::Done(_) => LoggedEffect::Done,
        }
    }

    /// Whether this effect is one the loop is *waiting on* — the kind that can be
    /// left unanswered by a process that dies.
    ///
    /// `Note` and `Done` are not: a note is written and forgotten, and a run that
    /// reached `Done` has nothing to resume.
    #[must_use]
    pub fn is_blocking(&self) -> bool {
        !matches!(self, LoggedEffect::Note | LoggedEffect::Done)
    }
}

/// One row of the log, in order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum Row {
    /// What the loop asked for, written before it was done.
    Effect(LoggedEffect),
    /// What came back, written before it was fed in.
    Input(Input),
}

/// The dispatch a resumed driver must refuse.
#[derive(Debug, Clone, PartialEq)]
pub struct RefuseRepeat {
    pub step: u32,
    pub name: String,
    pub arguments: Map<String, Value>,
}

impl RefuseRepeat {
    /// Whether a re-emitted effect is the one that was interrupted.
    ///
    /// Matched on the arguments too: a resumed loop that legitimately calls the same
    /// write tool again, with different arguments, is doing new work and must not be
    /// refused.
    #[must_use]
    pub fn matches(&self, effect: &Effect) -> bool {
        matches!(
            effect,
            Effect::ExecuteTool { step, name, arguments, .. }
                if *step == self.step && *name == self.name && *arguments == self.arguments
        )
    }

    /// What the model is told. It says what happened and why, and leaves the
    /// decision where the knowledge is — the harness does not know whether the write
    /// landed, and pretending either way is worse than saying so.
    #[must_use]
    pub fn tool_error(&self) -> Input {
        Input::ToolError {
            detail: format!(
                "interrupted; not retried because it writes. The call to {} may or may \
                 not have completed before the platform restarted. Check before calling \
                 it again.",
                self.name
            ),
        }
    }
}

/// What the driver should do with a log.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Feed these to a fresh loop, in order, before anything else.
    pub inputs: Vec<Input>,
    /// The one dispatch to answer with [`RefuseRepeat::tool_error`] rather than
    /// perform. `None` on every other path.
    pub refuse_repeat: Option<RefuseRepeat>,
    /// The job this run is suspended on, if it is. The driver re-registers interest
    /// rather than restarting the work.
    pub waiting_on: Option<String>,
    /// Set when the run resumed across a `harness_version` change because it was
    /// suspended and the log's schema was compatible. Recorded as a `Note` so a
    /// replay can see it (AGENT-010 §5).
    pub crossed_versions: Option<String>,
}

/// Why a run could not be folded back.
#[derive(Debug, Clone, PartialEq)]
pub enum Interrupted {
    /// The loop's decisions changed under a run that was mid-step.
    HarnessVersion { recorded: String, current: String },
    /// The log itself cannot be read by this build.
    LogSchema { recorded: u32, current: u32 },
}

impl Interrupted {
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Interrupted::HarnessVersion { recorded, current } => format!(
                "this run was recorded by harness {recorded} and this platform is \
                 {current}; replaying it could take different decisions, so it is \
                 preserved rather than continued"
            ),
            Interrupted::LogSchema { recorded, current } => format!(
                "the run's effect log is schema v{recorded} and this platform reads \
                 v{current}"
            ),
        }
    }
}

/// The versions a log was written under.
#[derive(Debug, Clone, PartialEq)]
pub struct Recorded {
    pub harness_version: String,
    pub log_schema: u32,
}

/// Decide how to continue a run from its log.
///
/// # Errors
///
/// Returns [`Interrupted`] when the run must be preserved rather than continued.
/// The caller keeps the working state and offers it to the user as a new turn; for a
/// suspended run it also attaches the job's result as an artifact, so the compute is
/// never lost.
pub fn plan(rows: &[Row], recorded: &Recorded) -> Result<Plan, Interrupted> {
    if recorded.log_schema != EFFECT_LOG_SCHEMA_VERSION {
        return Err(Interrupted::LogSchema {
            recorded: recorded.log_schema,
            current: EFFECT_LOG_SCHEMA_VERSION,
        });
    }

    // The last blocking effect, and whether anything came back after it.
    let mut last_blocking: Option<&LoggedEffect> = None;
    let mut answered = true;
    for row in rows {
        match row {
            Row::Effect(e) if e.is_blocking() => {
                last_blocking = Some(e);
                answered = false;
            }
            Row::Effect(_) => {}
            Row::Input(_) => answered = true,
        }
    }

    let suspended = matches!(last_blocking, Some(LoggedEffect::Await { .. })) && !answered;

    let version_ok = recorded.harness_version == HARNESS_VERSION;
    let mut crossed_versions = None;
    if !version_ok {
        if !suspended {
            return Err(Interrupted::HarnessVersion {
                recorded: recorded.harness_version.clone(),
                current: HARNESS_VERSION.to_string(),
            });
        }
        // Suspended, and the log's shape is unchanged. The run holds no process and
        // its job is still running; throwing it away would cost the most expensive
        // thing the platform does.
        crossed_versions = Some(format!(
            "resumed across a harness version change ({} -> {}): the run was suspended \
             on a job and the effect log schema (v{}) is unchanged",
            recorded.harness_version, HARNESS_VERSION, EFFECT_LOG_SCHEMA_VERSION
        ));
    }

    let inputs: Vec<Input> = rows
        .iter()
        .filter_map(|r| match r {
            Row::Input(i) => Some(i.clone()),
            Row::Effect(_) => None,
        })
        .collect();

    let refuse_repeat = match (answered, last_blocking) {
        (
            false,
            Some(LoggedEffect::ExecuteTool {
                step,
                name,
                arguments,
                idempotent,
                risk,
            }),
        ) if !*idempotent || *risk != Risk::Read => Some(RefuseRepeat {
            step: *step,
            name: name.clone(),
            arguments: arguments.clone(),
        }),
        _ => None,
    };

    let waiting_on = match (answered, last_blocking) {
        (false, Some(LoggedEffect::Await { job_id, .. })) => Some(job_id.clone()),
        _ => None,
    };

    Ok(Plan {
        inputs,
        refuse_repeat,
        waiting_on,
        crossed_versions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(k: &str, v: &str) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(k.to_string(), json!(v));
        m
    }

    fn write_effect(step: u32, name: &str) -> LoggedEffect {
        LoggedEffect::ExecuteTool {
            step,
            name: name.to_string(),
            arguments: args("goal", "momentum"),
            risk: Risk::Write,
            idempotent: false,
        }
    }

    fn read_effect(step: u32) -> LoggedEffect {
        LoggedEffect::ExecuteTool {
            step,
            name: "get_bars".to_string(),
            arguments: args("instrument", "BTC-USD"),
            risk: Risk::Read,
            idempotent: true,
        }
    }

    fn recorded() -> Recorded {
        Recorded {
            harness_version: HARNESS_VERSION.to_string(),
            log_schema: EFFECT_LOG_SCHEMA_VERSION,
        }
    }

    /// The acceptance, at the level this module can state it: an interrupted WRITE
    /// is not retried, and the model is told why.
    #[test]
    fn an_unfinished_write_is_refused_on_resume() {
        let rows = vec![
            Row::Effect(LoggedEffect::CallModel { step: 1 }),
            Row::Input(Input::ModelReply {
                raw: r#"{"name":"create_experiment"}"#.into(),
            }),
            Row::Effect(write_effect(1, "create_experiment")),
            // and then the process died.
        ];
        let plan = plan(&rows, &recorded()).unwrap();
        let refuse = plan.refuse_repeat.expect("the write must be refused");
        assert_eq!(refuse.name, "create_experiment");
        let Input::ToolError { detail } = refuse.tool_error() else {
            panic!("the model is told, in its own channel");
        };
        assert!(detail.contains("not retried because it writes"));
        assert!(
            detail.contains("may or may not have completed"),
            "the harness does not know, and says so: {detail}"
        );
    }

    /// An interrupted READ is re-run. Nothing is lost by doing it twice, and
    /// refusing it would cost the step for no reason.
    #[test]
    fn an_unfinished_read_is_simply_re_run() {
        let rows = vec![
            Row::Effect(LoggedEffect::CallModel { step: 1 }),
            Row::Input(Input::ModelReply {
                raw: r#"{"name":"get_bars"}"#.into(),
            }),
            Row::Effect(read_effect(1)),
        ];
        assert!(plan(&rows, &recorded()).unwrap().refuse_repeat.is_none());
    }

    /// A write that DID return is not refused: it is answered from the log like
    /// everything else, because its result was recorded before the crash.
    #[test]
    fn a_write_that_answered_is_replayed_not_refused() {
        let rows = vec![
            Row::Effect(write_effect(1, "create_experiment")),
            Row::Input(Input::ToolResult {
                output: "exp_1".into(),
                source: "platform.research".into(),
            }),
            Row::Effect(LoggedEffect::CallModel { step: 2 }),
        ];
        let plan = plan(&rows, &recorded()).unwrap();
        assert!(plan.refuse_repeat.is_none());
        assert_eq!(plan.inputs.len(), 1);
    }

    /// Adversarial on the matching rule: the same write tool, called again with
    /// different arguments, is new work and must NOT be refused.
    #[test]
    fn the_refusal_is_for_that_call_not_that_tool() {
        let refuse = RefuseRepeat {
            step: 1,
            name: "create_experiment".into(),
            arguments: args("goal", "momentum"),
        };
        let same = Effect::ExecuteTool {
            step: 1,
            name: "create_experiment".into(),
            namespace: "research".into(),
            arguments: args("goal", "momentum"),
            risk: Risk::Write,
            idempotent: false,
        };
        let different = Effect::ExecuteTool {
            step: 1,
            name: "create_experiment".into(),
            namespace: "research".into(),
            arguments: args("goal", "mean reversion"),
            risk: Risk::Write,
            idempotent: false,
        };
        assert!(refuse.matches(&same));
        assert!(!refuse.matches(&different));
    }

    /// A run mid-step under a changed harness is preserved, not continued.
    #[test]
    fn a_harness_version_change_interrupts_a_running_run() {
        let rows = vec![Row::Effect(LoggedEffect::CallModel { step: 1 })];
        let err = plan(
            &rows,
            &Recorded {
                harness_version: "0.9.0".into(),
                log_schema: EFFECT_LOG_SCHEMA_VERSION,
            },
        )
        .unwrap_err();
        assert_eq!(
            err,
            Interrupted::HarnessVersion {
                recorded: "0.9.0".into(),
                current: HARNESS_VERSION.into()
            }
        );
        assert!(err.reason().contains("preserved rather than continued"));
    }

    /// The exception, and the reason it exists: a run suspended on a forty-minute
    /// sweep survives a deploy. It holds no process, the sweep is still running, and
    /// the log's shape has not changed.
    #[test]
    fn a_suspended_run_survives_a_harness_version_change() {
        let rows = vec![
            Row::Effect(write_effect(1, "run_sweep")),
            Row::Input(Input::JobAccepted {
                job_id: "job-7".into(),
            }),
            Row::Effect(LoggedEffect::Await {
                step: 1,
                job_id: "job-7".into(),
            }),
        ];
        let plan = plan(
            &rows,
            &Recorded {
                harness_version: "0.9.0".into(),
                log_schema: EFFECT_LOG_SCHEMA_VERSION,
            },
        )
        .unwrap();
        assert_eq!(plan.waiting_on.as_deref(), Some("job-7"));
        assert!(plan.crossed_versions.is_some(), "and it is recorded as a note");
        assert!(plan.refuse_repeat.is_none());
    }

    /// But an unreadable log interrupts even a suspended run — and the caller's job
    /// then is to attach the result as an artifact rather than drop it.
    #[test]
    fn an_incompatible_log_schema_interrupts_even_a_suspended_run() {
        let rows = vec![Row::Effect(LoggedEffect::Await {
            step: 1,
            job_id: "job-7".into(),
        })];
        let err = plan(
            &rows,
            &Recorded {
                harness_version: HARNESS_VERSION.into(),
                log_schema: EFFECT_LOG_SCHEMA_VERSION + 7,
            },
        )
        .unwrap_err();
        assert!(matches!(err, Interrupted::LogSchema { .. }));
    }

    /// The log has to survive the trip through the database.
    #[test]
    fn rows_round_trip_through_json() {
        let rows = vec![
            Row::Effect(write_effect(3, "create_experiment")),
            Row::Input(Input::ToolResult {
                output: "exp_1".into(),
                source: "platform.research".into(),
            }),
            Row::Input(Input::Approval {
                granted: true,
                note: "fine".into(),
            }),
            Row::Input(Input::JobDone {
                job_id: "job-7".into(),
                output: "done".into(),
                source: "platform.research".into(),
                ok: true,
            }),
        ];
        let text = serde_json::to_string(&rows).unwrap();
        let back: Vec<Row> = serde_json::from_str(&text).unwrap();
        assert_eq!(back.len(), rows.len());
        let replanned = plan(&back, &recorded()).unwrap();
        assert_eq!(replanned.inputs.len(), 3);
    }
}
