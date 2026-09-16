//! What to do with a run the process died in the middle of (AGENT-010 §5;
//! Set S task S-2.6).
//!
//! Today a restart marks every unfinished run `failed` with "interrupted by platform
//! restart". That is honest and it is expensive: a forty-minute sweep that was
//! nineteen minutes in loses nineteen minutes, and the model's findings — which are
//! the compressed form of everything it read — go with them.
//!
//! The effect log (S-2.5) makes a better answer possible, because it records what
//! the loop was *about to do* rather than only what it did. The three cases it
//! distinguishes are the three that matter:
//!
//! * the log ends on an **input** — the loop had consumed the world's answer and was
//!   about to think. Replaying the inputs rebuilds exactly that state;
//! * the log ends on an **idempotent effect** — decided, possibly done, safe either
//!   way. Re-run it;
//! * the log ends on a **non-idempotent effect** — decided, and *unknowable* whether
//!   it happened. This is the case the whole mechanism exists for.
//!
//! ## Why the third case is not retried
//!
//! It cannot be known from here whether the write landed. The log says the loop
//! decided to call `create_experiment`; the process died; the row may or may not be
//! in the database. Re-running it risks a second experiment, and a second experiment
//! is a second counted trial — which corrupts the one number every selection-adjusted
//! statistic on this platform depends on.
//!
//! So the run resumes with a `ToolError` the model can read, saying the call was
//! interrupted and not retried *because it writes*. The model then checks, or reports
//! that it cannot. That is strictly better than both alternatives: failing the run
//! loses the work, and retrying silently corrupts the evidence.
//!
//! ## Version
//!
//! A replay by a different harness is not a replay. `harness_version` is on every
//! row, and a mismatch interrupts the run **with its working state preserved** rather
//! than pretending — a run that looks resumed and is not is worse than one that says
//! it stopped.

use serde_json::Value;

/// What a restart should do with one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// Replay these inputs through a fresh `Loop`, then continue.
    Replay {
        /// The inputs, in order, as they were recorded.
        inputs: Vec<RecordedInput>,
        /// An effect that was decided and never answered, if the log ends on one.
        pending: Option<PendingEffect>,
    },
    /// The log was written by a different harness. Stop, and keep what is there.
    Interrupted { reason: String },
    /// Nothing to resume: the run finished, or never started.
    Nothing { reason: String },
}

/// One input, as the log holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedInput {
    pub seq: i32,
    pub kind: String,
    pub payload: Value,
}

/// An effect the loop decided on and whose answer never arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEffect {
    pub seq: i32,
    pub kind: String,
    pub payload: Value,
    /// Whether re-running it is safe.
    ///
    /// Derived from what the log recorded at the time, not from the registry now: a
    /// tool that was idempotent when the call was made and is not any more must be
    /// treated as it was, because that is what actually ran.
    pub idempotent: bool,
}

impl PendingEffect {
    /// The tool call to answer with a failure instead of re-running.
    ///
    /// Only for the case that cannot be known: a write that may or may not have
    /// landed. The message is addressed to the **model**, because the model is the
    /// only party that can resolve it — by checking, or by saying it could not.
    #[must_use]
    pub fn interrupted_error(&self) -> String {
        let tool = self
            .payload
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("the call");
        format!(
            "`{tool}` was interrupted by a platform restart and has NOT been retried, \
             because it writes. It may or may not have completed. Check before assuming \
             either — and if you cannot check, say so rather than repeating it."
        )
    }
}

/// One row of the log, as read back.
#[derive(Debug, Clone)]
pub struct LogRow {
    pub seq: i32,
    pub direction: String,
    pub kind: String,
    pub payload: Value,
    pub harness_version: String,
}

/// Decide what to do with a run, from its log alone.
///
/// A pure function on purpose. The decision is the part that has to be right — it is
/// what stands between a restart and a duplicated write — and a pure function over
/// rows can be tested against the exact shapes that matter without a process to kill.
///
/// `current_version` is the harness doing the resuming.
#[must_use]
pub fn decide(rows: &[LogRow], current_version: &str) -> Resume {
    if rows.is_empty() {
        return Resume::Nothing {
            reason: "the run has no effect log; there is nothing to replay".into(),
        };
    }

    // Version first. Everything below reads the log as if this harness wrote it, and
    // that is only true if it did.
    if let Some(other) = rows
        .iter()
        .find(|r| r.harness_version != current_version)
    {
        return Resume::Interrupted {
            reason: format!(
                "the log was written by harness {} and this is {current_version}; \
                 a replay by a different harness is not a replay. The run is marked \
                 interrupted with its working state kept.",
                other.harness_version
            ),
        };
    }

    // A finished run has nothing to resume, however its log ends.
    if rows
        .iter()
        .any(|r| r.direction == "effect" && r.kind == "Done")
    {
        return Resume::Nothing {
            reason: "the run reached an outcome before the restart".into(),
        };
    }

    let inputs: Vec<RecordedInput> = rows
        .iter()
        .filter(|r| r.direction == "input")
        .map(|r| RecordedInput {
            seq: r.seq,
            kind: r.kind.clone(),
            payload: r.payload.clone(),
        })
        .collect();

    // The trailing effect, if the log ends on one. `Note` is excluded: a note is
    // something the loop said, not something it was waiting on, so a log ending on
    // one ends on whatever preceded it.
    let pending = rows
        .iter()
        .rev()
        .find(|r| !(r.direction == "effect" && r.kind == "Note"))
        .filter(|r| r.direction == "effect")
        .map(|r| PendingEffect {
            seq: r.seq,
            kind: r.kind.clone(),
            payload: r.payload.clone(),
            // Absent means NOT idempotent. The conservative reading: an effect whose
            // safety was not recorded is one whose safety is unknown, and unknown
            // must not be re-run.
            idempotent: r
                .payload
                .get("idempotent")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });

    Resume::Replay { inputs, pending }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const V: &str = "0.1.0";

    fn row(seq: i32, direction: &str, kind: &str, payload: Value) -> LogRow {
        LogRow {
            seq,
            direction: direction.into(),
            kind: kind.into(),
            payload,
            harness_version: V.into(),
        }
    }

    fn write_effect(seq: i32, name: &str) -> LogRow {
        row(
            seq,
            "effect",
            "ExecuteTool",
            json!({ "name": name, "arguments": { "slug": "ema_v1" }, "idempotent": false }),
        )
    }

    /// The case the whole mechanism exists for: a write that was decided and may or
    /// may not have landed.
    ///
    /// It cannot be known from here. Re-running risks a second experiment, and a
    /// second experiment is a second counted trial — which corrupts the one number
    /// every selection-adjusted statistic on this platform depends on.
    #[test]
    fn an_interrupted_write_is_not_retried() {
        let rows = vec![
            row(1, "effect", "CallModel", json!({ "step": 1 })),
            row(2, "input", "ModelReply", json!({ "raw": "{}" })),
            write_effect(3, "create_experiment"),
        ];
        let Resume::Replay { inputs, pending } = decide(&rows, V) else {
            panic!("a run with a trailing effect resumes");
        };
        assert_eq!(inputs.len(), 1, "the one answer it had received");
        let p = pending.expect("the trailing effect is reported");
        assert!(!p.idempotent, "create_experiment writes");
        let err = p.interrupted_error();
        assert!(err.contains("create_experiment"), "{err}");
        assert!(err.contains("NOT been retried"), "{err}");
        assert!(
            err.contains("because it writes"),
            "the model is told WHY, or it will just call it again: {err}"
        );
    }

    /// An idempotent read is safe to re-run, and re-running it is better than
    /// handing the model an error for something that would simply have worked.
    #[test]
    fn an_interrupted_read_is_safe_to_re_run() {
        let rows = vec![
            row(1, "input", "Admission", json!({})),
            row(
                2,
                "effect",
                "ExecuteTool",
                json!({ "name": "get_bars", "arguments": {}, "idempotent": true }),
            ),
        ];
        let Resume::Replay { pending, .. } = decide(&rows, V) else {
            panic!("resumes");
        };
        assert!(pending.expect("a trailing effect").idempotent);
    }

    /// An effect whose safety was never recorded is treated as unsafe.
    ///
    /// The conservative reading, and the direction matters: being wrong this way
    /// costs one message to the model, and being wrong the other way costs a
    /// duplicated write nobody can see.
    #[test]
    fn an_effect_with_no_recorded_safety_is_treated_as_a_write() {
        let rows = vec![row(
            1,
            "effect",
            "ExecuteTool",
            json!({ "name": "mystery", "arguments": {} }),
        )];
        let Resume::Replay { pending, .. } = decide(&rows, V) else {
            panic!("resumes");
        };
        assert!(
            !pending.expect("a trailing effect").idempotent,
            "unknown safety is not safety"
        );
    }

    /// A log ending on an INPUT has no pending effect: the loop had the answer and
    /// was about to think, and replaying the inputs rebuilds exactly that.
    #[test]
    fn a_log_ending_on_an_answer_has_nothing_outstanding() {
        let rows = vec![
            write_effect(1, "create_experiment"),
            row(2, "input", "ToolResult", json!({ "output": "{}" })),
        ];
        let Resume::Replay { inputs, pending } = decide(&rows, V) else {
            panic!("resumes");
        };
        assert!(pending.is_none(), "the write was answered, so it completed");
        assert_eq!(inputs.len(), 1);
    }

    /// A trailing `Note` is not something the loop was waiting on, so it does not
    /// hide the effect that actually is.
    #[test]
    fn a_trailing_note_does_not_mask_the_outstanding_effect() {
        let rows = vec![
            write_effect(1, "create_experiment"),
            row(2, "effect", "Note", json!({ "note": "Entered" })),
        ];
        let Resume::Replay { pending, .. } = decide(&rows, V) else {
            panic!("resumes");
        };
        let p = pending.expect("the write is still outstanding");
        assert_eq!(p.kind, "ExecuteTool");
        assert_eq!(p.seq, 1);
    }

    /// A replay by a different harness is not a replay. The run stops and keeps what
    /// it has, because a run that LOOKS resumed and is not is worse than one that
    /// says it stopped.
    #[test]
    fn a_log_from_another_harness_interrupts_rather_than_pretending() {
        let mut rows = vec![row(1, "input", "Admission", json!({}))];
        rows[0].harness_version = "0.0.9".into();
        let Resume::Interrupted { reason } = decide(&rows, V) else {
            panic!("a version mismatch must not replay");
        };
        assert!(reason.contains("0.0.9") && reason.contains(V), "{reason}");
        assert!(
            reason.contains("working state kept"),
            "and the work is not thrown away: {reason}"
        );
    }

    /// A finished run has nothing to resume, whatever its log ends on.
    #[test]
    fn a_run_that_reached_an_outcome_is_not_resumed() {
        let rows = vec![
            row(1, "input", "Admission", json!({})),
            row(2, "effect", "Done", json!({ "outcome": "Finished" })),
        ];
        assert!(matches!(decide(&rows, V), Resume::Nothing { .. }));
    }

    #[test]
    fn a_run_with_no_log_has_nothing_to_replay() {
        assert!(matches!(decide(&[], V), Resume::Nothing { .. }));
    }
}
