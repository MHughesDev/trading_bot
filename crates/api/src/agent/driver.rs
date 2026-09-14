//! The agent driver loop: LLM ↔ tools ↔ backtest waits.
//!
//! State machine: queued → running ⇄ waiting_backtest → completed | failed |
//! cancelled. Every step is persisted to `agent_messages` for the UI timeline;
//! counters on `agent_runs` update each iteration so spend is visible live.
//!
//! Backtest waits happen here (server-side polling via the platform API), so
//! an hour-long simulation costs zero LLM tokens.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use sqlx::PgPool;
use tokio::time::Instant;
use uuid::Uuid;

use harness::registry::FINISH_TASK;
use llm::{ChatRequest, ChatResponse, LlmClient, LlmError, Message, StopReason, ToolDef};
use mcp_server_lib::{dispatch_tool, tool_definitions_for, ApiClient, McpContext, ToolProfile};

use super::prompt;

pub struct DriverParams {
    pub run_id: Uuid,
    pub user_id: Uuid,
    /// The user's message for this turn.
    pub goal: String,
    pub model: String,
    pub constraints: Value,
    /// What earlier turns concluded. Empty on the first turn.
    pub prior: String,
    /// The conversation this turn belongs to, and therefore whose workspace it uses.
    pub conversation_id: Option<Uuid>,
    pub workspace_root: std::path::PathBuf,
    /// The model's usable input, from its capability profile. `None` falls back to a
    /// fixed cap.
    pub input_budget_tokens: Option<u32>,
    pub llm_max_tokens_per_call: u32,
}

/// Tool results larger than this are truncated before entering the transcript.
const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;
/// Transcript size (chars) at which old tool results are elided, when no profile
/// says otherwise. ~4 bytes per token against a 100k-token window.
const DEFAULT_TRANSCRIPT_CHARS: usize = 400_000;

/// Where compaction starts, as a share of the model's usable input.
///
/// The same trigger the harness context manager uses, read from the same constant so
/// the two tiers cannot drift apart. Compaction is a response to pressure: below this
/// the transcript is left alone, because eliding a transcript that fits is deletion
/// with no benefit.
fn transcript_limit(budget_tokens: Option<u32>) -> usize {
    match budget_tokens {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        Some(t) if t > 0 => {
            // Rounded, not truncated: `f32` 0.95 is 0.94999998, and a threshold that
            // lands a byte under the intended one for that reason is just confusing.
            (f64::from(t) * 4.0 * f64::from(harness::context::COMPACTION_TRIGGER)).round() as usize
        }
        _ => DEFAULT_TRANSCRIPT_CHARS,
    }
}
/// Keep this many most-recent tool results verbatim when eliding.
const KEEP_RECENT_TOOL_RESULTS: usize = 6;
/// Seconds between status messages while waiting on a backtest.
const WAIT_STATUS_EVERY_SECS: u64 = 30;
/// Seconds between backtest polls while waiting.
const WAIT_POLL_SECS: u64 = 10;

struct RunCtx {
    pg: PgPool,
    run_id: Uuid,
    seq: i32,
}

impl RunCtx {
    async fn append(&mut self, kind: &str, content: Value) {
        self.seq += 1;
        if let Err(e) = sqlx::query(
            "INSERT INTO agent_messages (run_id, seq, kind, content_json) VALUES ($1, $2, $3, $4)",
        )
        .bind(self.run_id)
        .bind(self.seq)
        .bind(kind)
        .bind(&content)
        .execute(&self.pg)
        .await
        {
            tracing::warn!(error = %e, run_id = %self.run_id, "agent message insert failed");
        }
    }

    async fn set_status(&self, status: &str) {
        let _ = sqlx::query("UPDATE agent_runs SET status = $2 WHERE run_id = $1")
            .bind(self.run_id)
            .bind(status)
            .execute(&self.pg)
            .await;
    }

    async fn bump_counters(&self, iterations: i32, tokens_in: i64, tokens_out: i64) {
        let _ = sqlx::query(
            "UPDATE agent_runs SET iterations = $2, tokens_in = $3, tokens_out = $4
             WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(iterations)
        .bind(tokens_in)
        .bind(tokens_out)
        .execute(&self.pg)
        .await;
    }

    async fn finish(
        &mut self,
        status: &str,
        error: Option<&str>,
        summary: Option<&str>,
        final_strategy: Option<&str>,
        best_backtest: Option<Uuid>,
    ) {
        if let Some(err) = error {
            self.append("error", json!({ "error": err })).await;
        }
        let _ = sqlx::query(
            "UPDATE agent_runs SET status = $2, error = $3, summary = $4,
                 final_strategy_id = $5, best_backtest_id = $6, finished_at = now()
             WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(status)
        .bind(error)
        .bind(summary)
        .bind(final_strategy)
        .bind(best_backtest)
        .execute(&self.pg)
        .await;
    }
}

fn tool_defs() -> Vec<ToolDef> {
    let mut defs: Vec<ToolDef> = tool_definitions_for(ToolProfile::InternalAgent)
        .as_array()
        .expect("tool definitions array")
        .iter()
        .map(|t| ToolDef {
            name: t["name"].as_str().unwrap_or_default().to_string(),
            description: t["description"].as_str().unwrap_or_default().to_string(),
            input_schema: t
                .get("inputSchema")
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
        })
        .collect();
    defs.push(finish_task_def());
    defs
}

/// Typed termination (harness guide 5.1, ADR-0032).
///
/// This replaced a prose contract - "reply with text starting with `FINAL:`" - and
/// the reason is not tidiness. **A model that can end a task by writing the right
/// words can end it by accident**: quoting the contract back, describing what it
/// plans to do, or summarising a tool result that happens to begin that way. The
/// harness decides when a task is done, and it decides it from a validated call with
/// checkable evidence.
///
/// Same name and same required fields as `harness::registry::FINISH_TASK`, so the
/// frontier tier and the local tier terminate through one contract rather than two
/// that have to be kept in step.
fn finish_task_def() -> ToolDef {
    ToolDef {
        name: FINISH_TASK.to_string(),
        description: "End the research task and report the result with its evidence. \
                      Call this when the acceptance criteria are met, or when you have \
                      established they cannot be. `evidence` must name real ids."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "result": {
                    "type": "string",
                    "description": "One paragraph: what you built and how it performed."
                },
                "evidence": {
                    "type": "string",
                    "description": "The ids that support it - experiment_id, study ids, gate ledger entries. A claim with no ids is not a finding."
                },
                "assessment": {
                    "type": "string",
                    "description": "What is fragile, citing the surface and the trial count, and what you would try next."
                },
                "strategy_id": {"type": "string", "description": "Slug of the best strategy."},
                "experiment_id": {"type": "string", "description": "UUID of the Experiment."},
                "backtest_id": {"type": "string", "description": "UUID of a cited member run."}
            },
            "required": ["result", "evidence"]
        }),
    }
}

/// What a validated `finish_task` carried.
#[derive(Debug)]
struct Finish {
    summary: String,
    strategy: Option<String>,
    backtest: Option<Uuid>,
}

/// The semantic rung for termination.
///
/// Returns the corrective sentence when the call is not a real conclusion. An empty
/// `evidence` is the case that matters: it is how "I am done" becomes
/// indistinguishable from "I would like to stop".
fn parse_finish(args: &Value) -> Result<Finish, String> {
    let field = |k: &str| {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let Some(result) = field("result") else {
        return Err("finish_task needs a non-empty `result` saying what you concluded".into());
    };
    let Some(evidence) = field("evidence") else {
        return Err(
            "finish_task needs `evidence` naming the ids that support the result - an \
                    experiment_id, study ids, or gate ledger entries. A conclusion with no ids \
                    cannot be checked, so it is not a finding."
                .into(),
        );
    };
    let mut summary = format!("{result}\n\nEvidence: {evidence}");
    if let Some(a) = field("assessment") {
        summary.push_str(&format!("\n\nAssessment: {a}"));
    }
    Ok(Finish {
        summary,
        strategy: field("strategy_id"),
        backtest: field("backtest_id").and_then(|b| Uuid::parse_str(&b).ok()),
    })
}

/// Truncates an oversized tool result, marking it so a model can tell the
/// difference between "the answer is short" and "the answer was cut".
///
/// Public because the auditor suite (AGENT-004 §4) runs a fixture against it: a
/// silent truncation is a leak of a different kind — the model reasons over a
/// prefix and reports as though it saw the whole thing.
pub fn truncate_result(result: &Value) -> String {
    let text = serde_json::to_string(result).unwrap_or_default();
    if text.len() <= MAX_TOOL_RESULT_BYTES {
        return text;
    }
    let mut end = MAX_TOOL_RESULT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}… [truncated {} of {} bytes — request summary detail instead]",
        &text[..end],
        text.len() - end,
        text.len()
    )
}

/// Elide old tool-result bodies when the transcript gets too big.
fn compact_transcript(messages: &mut [Message], limit: usize) {
    let total: usize = messages
        .iter()
        .map(|m| match m {
            Message::User { content } => content.len(),
            Message::Assistant { content, .. } => content.as_deref().map_or(0, str::len),
            Message::ToolResult { content, .. } => content.len(),
        })
        .sum();
    if total <= limit {
        return;
    }
    let result_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m, Message::ToolResult { .. }))
        .map(|(i, _)| i)
        .collect();
    let elide_until = result_indices
        .len()
        .saturating_sub(KEEP_RECENT_TOOL_RESULTS);
    for &i in &result_indices[..elide_until] {
        if let Message::ToolResult { content, .. } = &mut messages[i] {
            if content.len() > 200 {
                *content = format!("[elided — was {} bytes]", content.len());
            }
        }
    }
}

async fn chat_with_retry(
    llm_client: &LlmClient,
    req: &ChatRequest,
) -> Result<ChatResponse, LlmError> {
    let mut delay = Duration::from_secs(2);
    for attempt in 0..3 {
        match llm_client.chat(req).await {
            Ok(resp) => return Ok(resp),
            Err(e) if e.is_retryable() && attempt < 2 => {
                let wait = match &e {
                    LlmError::RateLimited {
                        retry_after_secs: Some(s),
                    } => Duration::from_secs((*s).min(120)),
                    _ => delay,
                };
                tracing::warn!(error = %e, attempt, "agent LLM call retrying");
                tokio::time::sleep(wait).await;
                delay *= 4;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("retry loop always returns")
}

/// Start (or resume) a sweep and wait for it inside the driver — the same
/// zero-token contract as [`wait_for_backtest_inline`]: no per-call cap, status
/// rows for the timeline, cancel-aware (cancels the sweep too).
async fn run_sweep_inline(
    ctx: &mut RunCtx,
    api: &ApiClient,
    args: &Value,
    _deadline: Option<Instant>,
    cancel: &AtomicBool,
) -> Value {
    let sweep_id = match args.get("sweep_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let started = match api.post("/api/research/sweeps", args.clone()).await {
                Ok(v) => v,
                Err(e) => return e.to_tool_error(),
            };
            match started.get("sweep_id").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => return started,
            }
        }
    };
    ctx.set_status("waiting_backtest").await;
    let mut last_status_emit: Option<Instant> = None;
    let result = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = api
                .post(
                    &format!("/api/research/sweeps/{sweep_id}/cancel"),
                    json!({}),
                )
                .await;
            break json!({ "cancelled": true, "sweep_id": sweep_id, "note": "run cancelled while sweeping" });
        }
        match api.get(&format!("/api/research/sweeps/{sweep_id}")).await {
            Ok(snapshot) => {
                let status = snapshot
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let due = last_status_emit
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(WAIT_STATUS_EVERY_SECS));
                if due {
                    ctx.append(
                        "status",
                        json!({
                            "phase": "sweeping",
                            "sweep_id": sweep_id,
                            "sweep_status": status,
                            "done": snapshot.get("done").cloned().unwrap_or(Value::Null),
                            "planned": snapshot.get("planned").cloned().unwrap_or(Value::Null),
                            "note": snapshot.get("note").cloned().unwrap_or(Value::Null),
                        }),
                    )
                    .await;
                    last_status_emit = Some(Instant::now());
                }
                if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
                    break snapshot;
                }
            }
            Err(e) => {
                if matches!(e.status, Some(s) if s < 500) {
                    break e.to_tool_error();
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(WAIT_POLL_SECS)).await;
    };
    ctx.set_status("running").await;
    result
}

/// Wait on a backtest inside the driver: no per-call timeout cap (bounded by
/// the run's wall-clock budget), status messages for the UI, cancel-aware.
async fn wait_for_backtest_inline(
    ctx: &mut RunCtx,
    api: &ApiClient,
    backtest_id: &str,
    _deadline: Option<Instant>,
    cancel: &AtomicBool,
) -> Value {
    ctx.set_status("waiting_backtest").await;
    // None = no status message emitted yet, so the first poll always emits one.
    let mut last_status_emit: Option<Instant> = None;
    let result = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = api
                .post(&format!("/api/backtests/{backtest_id}/stop"), json!({}))
                .await;
            break json!({ "cancelled": true, "note": "run cancelled while waiting" });
        }
        match api.get(&format!("/api/backtests/{backtest_id}")).await {
            Ok(snapshot) => {
                let status = snapshot
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let progress = snapshot
                    .get("progress")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let due = last_status_emit
                    .is_none_or(|t| t.elapsed() >= Duration::from_secs(WAIT_STATUS_EVERY_SECS));
                if due {
                    ctx.append(
                        "status",
                        json!({
                            "phase": "waiting_backtest",
                            "backtest_id": backtest_id,
                            "backtest_status": status,
                            "progress": progress,
                        }),
                    )
                    .await;
                    last_status_emit = Some(Instant::now());
                }
                if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
                    break snapshot;
                }
            }
            Err(e) => {
                if matches!(e.status, Some(s) if s < 500) {
                    break e.to_tool_error();
                }
                // Transient — keep waiting.
            }
        }
        tokio::time::sleep(Duration::from_secs(WAIT_POLL_SECS)).await;
    };
    ctx.set_status("running").await;
    result
}

pub async fn drive(
    pg: &PgPool,
    api: ApiClient,
    llm_client: LlmClient,
    params: DriverParams,
    cancel: Arc<AtomicBool>,
) {
    // Every platform tool call is recorded before it is authorised and after it
    // is answered (SPEC §15, ADR-P2-21). This driver dispatches directly rather
    // than through `bridge::execute`, so it opens the record itself.
    let audit_trail = super::audit::AuditTrail::for_agent(pg.clone(), params.user_id, params.run_id);
    let mut ctx = RunCtx {
        pg: pg.clone(),
        run_id: params.run_id,
        seq: 0,
    };
    // No deadline, and no iteration cap.
    //
    // The agent runs until it calls `finish_task`. A wall clock could not tell the
    // difference between a session that was stuck and one that was waiting on a
    // forty-minute sweep, so it killed both — throwing away the work rather than
    // bounding it. What remains as a stop condition is the set of things that mean
    // something: the user cancels, the same failure survives its retries, or the
    // backend breaks a guarantee (`Degradation`). Those catch a runaway; a timer
    // only caught slowness.

    let _ = sqlx::query(
        "UPDATE agent_runs SET status = 'running', started_at = now() WHERE run_id = $1",
    )
    .bind(params.run_id)
    .execute(pg)
    .await;
    ctx.append(
        "status",
        json!({ "phase": "started", "model": params.model }),
    )
    .await;

    // The agent's tools run through the platform's own API on loopback with a
    // run-scoped token — identical code path to the external MCP front door.
    let mcp_ctx = McpContext::new(api.clone());
    let tools = tool_defs();
    let system = prompt::system_prompt(&params.constraints);

    let mut messages: Vec<Message> = vec![Message::User {
        content: params.goal.clone(),
    }];
    let mut iterations: i32 = 0;
    let mut tokens_in: i64 = 0;
    let mut tokens_out: i64 = 0;
    let mut nudged_final = false;
    let mut nudged_max_tokens = false;

    loop {
        // The only stop conditions left: the user cancels, or the loop finishes.
        if cancel.load(Ordering::SeqCst) {
            ctx.finish("cancelled", None, None, None, None).await;
            return;
        }

        iterations += 1;
        compact_transcript(&mut messages, transcript_limit(params.input_budget_tokens));
        let req = ChatRequest {
            model: params.model.clone(),
            system: Some(system.clone()),
            messages: messages.clone(),
            tools: tools.clone(),
            max_tokens: params.llm_max_tokens_per_call,
            temperature: None,
            schema: None,
            num_ctx: None,
            keep_alive: None,
            tool_choice: None,
        };

        let resp = match chat_with_retry(&llm_client, &req).await {
            Ok(r) => r,
            Err(e) => {
                let msg = match &e {
                    LlmError::Auth(_) => format!("{e} — update the provider key in Settings"),
                    LlmError::ToolsUnsupported(_) => {
                        format!("{e} — pick a tool-capable model (e.g. qwen2.5, llama3.1)")
                    }
                    _ => e.to_string(),
                };
                ctx.finish("failed", Some(&msg), None, None, None).await;
                return;
            }
        };

        tokens_in += resp.usage.input_tokens as i64;
        tokens_out += resp.usage.output_tokens as i64;
        ctx.bump_counters(iterations, tokens_in, tokens_out).await;
        ctx.append(
            "assistant",
            json!({
                "content": resp.content,
                "tool_calls": resp.tool_calls,
                "stop_reason": resp.stop_reason,
            }),
        )
        .await;
        messages.push(Message::Assistant {
            content: resp.content.clone(),
            tool_calls: resp.tool_calls.clone(),
        });

        // ── No tool calls → the model is talking to us ────────────────────
        if resp.tool_calls.is_empty() {
            let content = resp.content.clone().unwrap_or_default();
            if resp.stop_reason == StopReason::MaxTokens && !nudged_max_tokens {
                nudged_max_tokens = true;
                messages.push(Message::User {
                    content: "Your reply was cut off. Be more concise and continue.".into(),
                });
                continue;
            }
            if !nudged_final {
                nudged_final = true;
                messages.push(Message::User {
                    content: "Continue working with your tools. When you are done, call \
                              finish_task with `result` and `evidence` - prose does not end \
                              the task."
                        .into(),
                });
                continue;
            }
            // Second bare reply. The prose is kept as the summary so nothing is lost,
            // but the run did NOT meet the termination contract, and recording it as
            // `completed` would be a false report: the conclusion carries no evidence
            // anyone can check.
            ctx.append("final", json!({ "content": content })).await;
            ctx.finish(
                "failed",
                Some(
                    "the run ended without calling finish_task, so its conclusion carries no \
                     checkable evidence",
                ),
                Some(&content),
                None,
                None,
            )
            .await;
            return;
        }

        // ── Execute tool calls ────────────────────────────────────────────
        for call in &resp.tool_calls {
            if cancel.load(Ordering::SeqCst) {
                ctx.finish("cancelled", None, None, None, None).await;
                return;
            }
            ctx.append(
                "tool_call",
                json!({ "id": call.id, "name": call.name, "arguments": call.arguments }),
            )
            .await;

            // Termination is answered here, never dispatched: `finish_task` is the
            // harness deciding the task is over, not a platform call.
            if call.name == FINISH_TASK {
                let parsed = parse_finish(&call.arguments);
                let recorded = match &parsed {
                    Ok(_) => json!({ "finished": true }),
                    Err(correction) => json!({ "error": correction }),
                };
                mcp_ctx.record_step(FINISH_TASK, &call.arguments, &recorded, 0, None).await;
                match parsed {
                    Ok(f) => {
                        ctx.append("final", json!({ "content": f.summary })).await;
                        ctx.finish(
                            "completed",
                            None,
                            Some(&f.summary),
                            f.strategy.as_deref(),
                            f.backtest,
                        )
                        .await;
                        return;
                    }
                    Err(correction) => {
                        ctx.append(
                            "tool_result",
                            json!({ "tool_call_id": call.id, "name": call.name,
                                    "is_error": true, "content": correction }),
                        )
                        .await;
                        messages.push(Message::ToolResult {
                            tool_call_id: call.id.clone(),
                            name: call.name.clone(),
                            content: correction,
                            is_error: true,
                        });
                        continue;
                    }
                }
            }

            let started = std::time::Instant::now();
            let inline = call.name == "wait_for_backtest" || call.name == "run_sweep";
            let result = if call.name == "wait_for_backtest" {
                let backtest_id = call
                    .arguments
                    .get("backtest_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if backtest_id.is_empty() {
                    json!({ "error": "missing_field", "field": "backtest_id" })
                } else {
                    wait_for_backtest_inline(&mut ctx, &api, &backtest_id, None, &cancel).await
                }
            } else if call.name == "run_sweep" {
                run_sweep_inline(&mut ctx, &api, &call.arguments, None, &cancel).await
            } else {
                let opened = audit_trail.pre(&call.name, &call.arguments).await;
                let envelope = ledger::audit::Envelope::for_action(&call.name);
                if envelope.requires_approval() {
                    // This driver has no pause: it is the older SDK loop, and its
                    // state machine cannot hold a call across a human answer. So a
                    // §15 action is **refused** here rather than executed
                    // unapproved. The GOVERNOR loop (`local_driver`) is the path
                    // that pauses and resumes; a session that needs one of these
                    // four belongs on it.
                    audit_trail
                        .post(opened, ledger::audit::AuditVerdict::Denied)
                        .await;
                    json!({
                        "error": {
                            "code": "policy_denied",
                            "envelope": envelope.as_str(),
                            "fix": "this action needs a human approval; run it from a session that can pause"
                        }
                    })
                } else {
                    let raw = dispatch_tool(&mcp_ctx, &call.name, &call.arguments, None).await;
                    audit_trail.post(opened, super::audit::verdict_of(&raw)).await;
                    raw
                }
            };

            if inline {
                // Answered by the driver itself, so dispatch never saw it.
                mcp_ctx.record_step(&call.name, &call.arguments, &result, started.elapsed().as_millis(), None).await;
            }
            let is_error = result.get("error").is_some();
            let text = truncate_result(&result);
            ctx.append(
                "tool_result",
                json!({
                    "tool_call_id": call.id,
                    "name": call.name,
                    "is_error": is_error,
                    "content": result,
                }),
            )
            .await;
            messages.push(Message::ToolResult {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                content: text,
                is_error,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_finish_call_carries_its_evidence_and_ids() {
        let f = parse_finish(&json!({
            "result": "an EMA cross beats hold on BTC-USD 1h",
            "evidence": "experiment 7f0c, studies s1 s2, gate 3 passed",
            "assessment": "fragile below 4h; the surface is a spike, not a plateau",
            "strategy_id": "ema_cross_v3",
            "backtest_id": "6f9619ff-8b86-d011-b42d-00c04fc964ff"
        }))
        .unwrap();
        assert_eq!(f.strategy.as_deref(), Some("ema_cross_v3"));
        assert!(f.backtest.is_some());
        assert!(f.summary.contains("Evidence:"));
        assert!(f.summary.contains("Assessment:"));
    }

    /// The rung that makes typed termination worth having. "I am done" and "I would
    /// like to stop" are the same sentence without ids attached.
    #[test]
    fn a_conclusion_with_no_evidence_is_refused_with_a_correction() {
        let err = parse_finish(&json!({"result": "it works", "evidence": "  "})).unwrap_err();
        assert!(err.contains("evidence"));
        assert!(err.contains("cannot be checked"));
        assert!(parse_finish(&json!({"evidence": "exp_1"})).is_err());
    }

    /// The whole point of the change: prose can no longer end a run, at any tier.
    #[test]
    fn termination_is_a_tool_the_model_can_actually_call() {
        let names: Vec<String> = tool_defs().into_iter().map(|t| t.name).collect();
        assert!(
            names.iter().any(|n| n == FINISH_TASK),
            "the contract names a tool, so the tool has to be offered"
        );
        let def = finish_task_def();
        let required = def.input_schema["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "result"));
        assert!(required.iter().any(|r| r == "evidence"));
    }

    /// Same name, same required fields as the local tier's. Two spellings of "done"
    /// would be two termination contracts to keep in step, and the one that drifted
    /// would be the one nobody was testing.
    #[test]
    fn the_two_tiers_terminate_through_the_same_contract() {
        let local = harness::registry::ToolRegistry::with_core();
        let theirs = local.get(FINISH_TASK).unwrap();
        let ours = finish_task_def();
        assert_eq!(ours.name, theirs.name);
        for field in ["result", "evidence"] {
            assert!(theirs.input_schema["properties"].get(field).is_some());
            assert!(ours.input_schema["properties"].get(field).is_some());
        }
    }

    /// Both tiers compact at the same point, from the same constant. A transcript
    /// that fits is left alone: eliding it would be deletion with no benefit.
    #[test]
    fn the_transcript_limit_follows_the_model_rather_than_a_fixed_number() {
        // 95% of a 100k-token window, at ~4 bytes a token.
        assert_eq!(transcript_limit(Some(100_000)), 380_000);
        // A smaller local window compacts sooner, without anyone tuning a constant.
        assert!(transcript_limit(Some(20_000)) < transcript_limit(Some(100_000)));
        // No profile: the fixed fallback.
        assert_eq!(transcript_limit(None), DEFAULT_TRANSCRIPT_CHARS);
        assert_eq!(transcript_limit(Some(0)), DEFAULT_TRANSCRIPT_CHARS);
    }

    #[test]
    fn compact_transcript_elides_old_tool_results_only() {
        let big = "x".repeat(120_000);
        let mut messages: Vec<Message> = (0..8)
            .map(|i| Message::ToolResult {
                tool_call_id: format!("c{i}"),
                name: "t".into(),
                content: big.clone(),
                is_error: false,
            })
            .collect();
        messages.insert(
            0,
            Message::User {
                content: "goal".into(),
            },
        );
        compact_transcript(&mut messages, DEFAULT_TRANSCRIPT_CHARS);
        let elided = messages
            .iter()
            .filter(|m| matches!(m, Message::ToolResult { content, .. } if content.starts_with("[elided")))
            .count();
        assert_eq!(elided, 2); // 8 results, keep 6 most recent
                               // User message untouched.
        assert!(matches!(&messages[0], Message::User { content } if content == "goal"));
    }

    #[test]
    fn truncate_result_marks_oversized_payloads() {
        let huge = json!({ "data": "y".repeat(MAX_TOOL_RESULT_BYTES * 2) });
        let text = truncate_result(&huge);
        assert!(text.len() < MAX_TOOL_RESULT_BYTES + 200);
        assert!(text.contains("truncated"));
    }
}
