//! Trajectories, derived from the effect log (AGENT-009 §2; Set S task S-4.8).
//!
//! # The gap this closes
//!
//! `harness::trace` is 422 lines over nine tests — `Span`, `Event`,
//! `is_replayable`, `unreplayable_reason` — and **nothing in the platform
//! produces a single span**. Searched across `crates/` and `apps/`: there is no
//! `Span::new` outside the module's own tests.
//!
//! So S-4.8's acceptance ("every live run's trajectory replays to identical
//! effects") was blocked twice over, and the plan recorded only one of them. The
//! recorded blocker is that there are no live runs, which is true and is gate
//! H-2's. The unrecorded one is that a live run would produce no trajectory
//! either: the thing that would have to replay is not written down.
//!
//! # Derived, not written a second time
//!
//! `agent_effect_log` already holds the ordered `(direction, kind, payload)`
//! rows for every run — it is what a resume reads, so it is already correct and
//! already maintained. A second write path for spans would be a second record of
//! one history, and this repository has already stated the rule for that in
//! `knowledge_jobs`: a derived table that drifts from its source is worse than
//! no derived table, because it is a second answer with no way to tell which is
//! stale.
//!
//! So spans are a **projection**. The effect log is the record; this is a view
//! over it, rebuilt on demand, and a trajectory that disagrees with the log is
//! impossible rather than merely unlikely.
//!
//! # What a replay actually needs
//!
//! `is_replayable` asks for at least one model call and every tool call carrying
//! its arguments. The effect log has both: `CallModel` records the prompt bytes
//! and the decode, `ExecuteTool` records the arguments **in full** — deliberately,
//! per `describe_effect`, because a hash cannot answer "was this the same call".
//! `Remember` and the core-tool notes carry what the harness answered itself.

use harness::trace::{Event, Span};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum TrajectoryError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

/// One run's trajectory, oldest first.
///
/// # Errors
/// [`TrajectoryError::Sqlx`] if the log cannot be read.
pub async fn spans_for_run(pool: &PgPool, run_id: Uuid) -> Result<Vec<Span>, TrajectoryError> {
    let rows = sqlx::query(
        "SELECT seq, direction, kind, payload_json FROM agent_effect_log \
          WHERE run_id = $1 ORDER BY seq ASC",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    let trace_id = run_id.to_string();
    let mut spans = Vec::new();
    let mut step: u32 = 0;

    for row in &rows {
        let seq: i32 = row.get("seq");
        let direction: String = row.get("direction");
        let kind: String = row.get("kind");
        let payload: Value = row.try_get("payload_json").unwrap_or(Value::Null);

        // The step the loop was on, when the row says. Carried forward
        // otherwise: a row without one belongs to the step that preceded it, and
        // defaulting to zero would put every un-stepped row at the start.
        if let Some(s) = payload.get("step").and_then(Value::as_u64) {
            step = u32::try_from(s).unwrap_or(step);
        }

        let Some(event) = event_for(&direction, &kind, &payload) else {
            continue;
        };
        spans.push(Span::new(
            trace_id.clone(),
            format!("{trace_id}:{seq}"),
            "orchestrator",
            step,
            event,
        ));
    }
    Ok(spans)
}

/// What one pass of the replayability sampler found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplaySample {
    pub sampled: usize,
    pub replayable: usize,
    /// Run id and why, for every sampled run whose trajectory cannot replay.
    pub unreplayable: Vec<(Uuid, String)>,
}

/// Sample the most recent runs that logged anything and ask `is_replayable` of
/// each trajectory (S-4.8: "`is_replayable` returns true for a nightly sample of
/// real runs").
///
/// This is the consumer `harness::trace` was missing. A run with no effect rows
/// at all is not in the sample (it never reached the log), and one that logged
/// only inputs projects to an empty trajectory, which `is_replayable` refuses by
/// naming the missing model call.
///
/// # Errors
/// [`TrajectoryError::Sqlx`] if the log cannot be read.
pub async fn sample_replayable(
    pool: &PgPool,
    limit: i64,
) -> Result<ReplaySample, TrajectoryError> {
    let runs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT run_id FROM agent_effect_log GROUP BY run_id           ORDER BY max(created_at) DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut out = ReplaySample::default();
    for run in runs {
        let spans = spans_for_run(pool, run).await?;
        out.sampled += 1;
        if harness::trace::is_replayable(&spans) {
            out.replayable += 1;
        } else {
            let why = harness::trace::unreplayable_reason(&spans)
                .unwrap_or_else(|| "not replayable".to_string());
            out.unreplayable.push((run, why));
        }
    }
    Ok(out)
}

/// The nightly loop: sample 20 recent runs and log the share that replay. Needs
/// one line in the platform's `main.rs`:
/// `api::trace_from_effects::spawn_replay_sampler(pg.clone());`.
pub fn spawn_replay_sampler(pool: PgPool) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(120)).await;
        loop {
            match sample_replayable(&pool, 20).await {
                Ok(r) => tracing::info!(
                    sampled = r.sampled,
                    replayable = r.replayable,
                    unreplayable = r.unreplayable.len(),
                    "nightly replayability sample"
                ),
                Err(e) => tracing::warn!(error = %e, "nightly replayability sample failed"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(24 * 3600)).await;
        }
    });
}

/// One log row as a trace event, or `None` for a row a replay does not need.
///
/// Rows are skipped rather than mapped to a placeholder: `is_replayable` counts
/// model calls and checks that every tool call carries arguments, and a
/// placeholder event would be a tool call with no arguments — which is exactly
/// the shape that check exists to catch. A trace that logged names without
/// arguments "looks complete in a dashboard and cannot reproduce anything".
fn event_for(direction: &str, kind: &str, payload: &Value) -> Option<Event> {
    // The parent's side of a delegation (S-2.12): a state row, because it is the
    // driver writing down that it dispatched a sub-run, not a decision to replay.
    if direction == "state" && kind == "Delegation" {
        return Some(Event::Delegation {
            child_trace_id: payload
                .get("child_trace_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            goal: payload
                .get("goal")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            tool_grant: Vec::new(),
            budget_usd: 0.0,
        });
    }
    if direction != "effect" {
        return None;
    }
    let text = |k: &str| {
        payload
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match kind {
        "CallModel" => Some(Event::ModelCall {
            provider: String::new(),
            model: String::new(),
            // The log records the prompt's size rather than the prompt: traces
            // outlive prompts, and `harness::trace` records a hash for the same
            // reason. What is available here is the length, which is what the
            // log kept.
            prompt_hash: format!(
                "bytes:{}",
                payload
                    .get("prompt_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            ),
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            latency_ms: 0,
        }),
        "ExecuteTool" => Some(Event::ToolCall {
            name: text("name"),
            namespace: text("namespace"),
            risk: harness::registry::Risk::Read,
            // In full, which is why the log keeps them in full.
            args: payload.get("arguments").cloned().unwrap_or(Value::Null),
            result_bytes: 0,
            truncated: false,
            // The log does not record provenance per call, and a platform tool
            // is what `ExecuteTool` dispatches — the untrusted lane is a
            // property of a *result*, which this row does not carry. Stated as
            // the narrow claim rather than guessed wider.
            provenance: harness::provenance::Provenance::ToolInternal,
            latency_ms: 0,
            ok: true,
        }),
        // What the harness answered itself. These never became `ToolCall`s
        // because they are never dispatched, and a trajectory missing them
        // cannot replay a run whose whole content was findings.
        "Note" => {
            let name = payload.get("core_tool").and_then(Value::as_str)?;
            Some(Event::CoreTool {
                name: name.to_string(),
                args: payload.get("args").cloned().unwrap_or(Value::Null),
                reply: text("detail"),
                accepted: true,
            })
        }
        "Remember" => Some(Event::CoreTool {
            name: "record_finding".into(),
            args: payload.clone(),
            reply: text("entry_id"),
            accepted: true,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A model call and a tool call with its arguments is a replayable pair.
    #[test]
    fn a_model_call_and_a_tool_call_replay() {
        let spans = vec![
            Span::new(
                "t",
                "t:1",
                "orchestrator",
                1,
                event_for(
                    "effect",
                    "CallModel",
                    &json!({"prompt_bytes": 4096, "step": 1}),
                )
                .expect("a model call maps"),
            ),
            Span::new(
                "t",
                "t:2",
                "orchestrator",
                1,
                event_for(
                    "effect",
                    "ExecuteTool",
                    &json!({"name": "get_bars", "namespace": "data",
                            "arguments": {"instrument": "BTC-USD"}, "step": 1}),
                )
                .expect("a tool call maps"),
            ),
        ];
        assert!(
            harness::trace::is_replayable(&spans),
            "{:?}",
            harness::trace::unreplayable_reason(&spans)
        );
    }

    /// A tool call whose arguments were lost is not replayable, and the
    /// projection must not manufacture them.
    #[test]
    fn a_tool_call_without_arguments_is_not_replayable() {
        let spans = vec![
            Span::new(
                "t",
                "t:1",
                "orchestrator",
                1,
                event_for("effect", "CallModel", &json!({"prompt_bytes": 10})).unwrap(),
            ),
            Span::new(
                "t",
                "t:2",
                "orchestrator",
                1,
                event_for("effect", "ExecuteTool", &json!({"name": "get_bars"})).unwrap(),
            ),
        ];
        assert!(
            !harness::trace::is_replayable(&spans),
            "a trace that logged a tool name without its arguments looks complete and \
             reproduces nothing"
        );
    }

    /// Harness-owned calls become `CoreTool` rows, which is ADR-P0-18's gap.
    #[test]
    fn a_core_tool_note_becomes_a_core_tool_event() {
        let e = event_for(
            "effect",
            "Note",
            &json!({"core_tool": "record_finding", "detail": "stored established"}),
        )
        .expect("a core-tool note maps");
        match e {
            Event::CoreTool { name, reply, .. } => {
                assert_eq!(name, "record_finding");
                assert_eq!(reply, "stored established");
            }
            other => panic!("expected CoreTool, got {other:?}"),
        }
    }

    /// Inputs are not events. The log records both directions; a trajectory is
    /// what the loop *did*, and folding the inputs in would double every step.
    #[test]
    fn inputs_are_not_projected() {
        assert!(event_for("input", "ModelReply", &json!({"raw": "..."})).is_none());
    }

    /// A row a replay does not need is skipped, not mapped to a placeholder.
    #[test]
    fn an_unmapped_effect_is_skipped_rather_than_faked() {
        assert!(event_for("effect", "Await", &json!({"job_id": "j1"})).is_none());
        // A note that is not a core tool carries nothing a replay can use.
        assert!(event_for("effect", "Note", &json!({"detail": "entered"})).is_none());
    }
}

#[cfg(test)]
mod projection_matches_the_log {
    use super::*;
    use serde_json::json;
    use harness::drive::{Effect, Note};

    /// The projection reads what the log actually writes.
    ///
    /// Two functions have to agree — `describe_effect` produces the row and
    /// `event_for` consumes it — and nothing in the type system connects them.
    /// This is that connection: the payload comes from the real producer rather
    /// than from a fixture, so a key renamed on one side fails here instead of
    /// silently emptying every trajectory.
    #[test]
    fn a_core_tool_note_survives_the_round_trip_through_the_log() {
        let effect = Effect::Note(Note::CoreTool {
            name: "record_finding".into(),
            detail: "BTC-USD momentum — stored suggestive".into(),
            args: json!({ "finding": "BTC-USD momentum", "evidence": "exp_1" }),
        });
        let (kind, payload) = crate::agent::local_driver::describe_effect_for_test(&effect);
        assert_eq!(kind, "Note");

        let event = event_for("effect", kind, &payload)
            .expect("the row the log writes is a row the projection reads");
        match &event {
            Event::CoreTool { name, reply, args, .. } => {
                assert_eq!(name, "record_finding");
                assert!(reply.contains("suggestive"));
                assert_eq!(args["evidence"], "exp_1", "the arguments reach the trace");
            }
            other => panic!("expected CoreTool, got {other:?}"),
        }
        // Was pinned broken (S-4.8): a core-tool row carried no arguments, so any
        // run that used `record_finding` or `finish_task` was reported
        // unreplayable. It is replayable now, which is what this holds.
        let spans = vec![
            Span::new(
                "t",
                "t:1",
                "orchestrator",
                1,
                event_for("effect", "CallModel", &json!({"prompt_bytes": 4096, "step": 1}))
                    .expect("a model call maps"),
            ),
            Span::new("t", "t:2", "orchestrator", 1, event),
        ];
        assert!(
            harness::trace::is_replayable(&spans),
            "a trajectory that used record_finding must be replayable: {:?}",
            harness::trace::unreplayable_reason(&spans)
        );
    }

    /// And a note that is not a core tool still logs, and still projects to
    /// nothing — it carries no arguments and a replay cannot use it.
    #[test]
    fn an_ordinary_note_logs_and_projects_to_nothing() {
        let effect = Effect::Note(Note::Entered {
            state: harness::drive::State::Admitting,
            step: 1,
        });
        let (kind, payload) = crate::agent::local_driver::describe_effect_for_test(&effect);
        assert_eq!(kind, "Note");
        assert!(payload.get("note").is_some(), "it is still recorded");
        assert!(
            event_for("effect", kind, &payload).is_none(),
            "and it is not projected into a trajectory"
        );
    }
}
