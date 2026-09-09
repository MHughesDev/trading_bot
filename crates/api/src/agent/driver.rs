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

use llm::{ChatRequest, ChatResponse, LlmClient, LlmError, Message, StopReason, ToolDef};
use mcp_server_lib::{dispatch_tool, tool_definitions_for, ApiClient, McpContext, ToolProfile};

use super::prompt;

pub struct DriverParams {
    pub run_id: Uuid,
    pub user_id: Uuid,
    pub goal: String,
    pub model: String,
    pub constraints: Value,
    pub max_iterations: i32,
    pub max_total_tokens: Option<i64>,
    pub wallclock_budget_secs: i64,
    pub llm_max_tokens_per_call: u32,
}

/// Tool results larger than this are truncated before entering the transcript.
const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;
/// Rough transcript size cap (chars) before old tool results are elided.
const MAX_TRANSCRIPT_CHARS: usize = 400_000;
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
    tool_definitions_for(ToolProfile::InternalAgent)
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
        .collect()
}

fn truncate_result(result: &Value) -> String {
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
fn compact_transcript(messages: &mut [Message]) {
    let total: usize = messages
        .iter()
        .map(|m| match m {
            Message::User { content } => content.len(),
            Message::Assistant { content, .. } => content.as_deref().map_or(0, str::len),
            Message::ToolResult { content, .. } => content.len(),
        })
        .sum();
    if total <= MAX_TRANSCRIPT_CHARS {
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

/// Parse the FINAL: reply per the termination contract.
fn parse_final(content: &str) -> (Option<String>, Option<Uuid>) {
    let mut strategy = None;
    let mut backtest = None;
    for line in content.lines() {
        let lower = line.trim().to_lowercase();
        if let Some(rest) = lower.strip_prefix("strategy:") {
            let slug = rest.trim().trim_matches('`').to_string();
            if !slug.is_empty() {
                strategy = Some(slug);
            }
        }
        if let Some(rest) = lower.strip_prefix("backtest_id:") {
            if let Ok(id) = Uuid::parse_str(rest.trim().trim_matches('`')) {
                backtest = Some(id);
            }
        }
    }
    (strategy, backtest)
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
    deadline: Instant,
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
                .post(&format!("/api/research/sweeps/{sweep_id}/cancel"), json!({}))
                .await;
            break json!({ "cancelled": true, "sweep_id": sweep_id, "note": "run cancelled while sweeping" });
        }
        if Instant::now() > deadline {
            break json!({
                "timed_out": true,
                "sweep_id": sweep_id,
                "note": "run wall-clock budget exhausted while sweeping",
            });
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
    deadline: Instant,
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
        if Instant::now() > deadline {
            break json!({
                "timed_out": true,
                "note": "run wall-clock budget exhausted while waiting for the backtest",
            });
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
    let mut ctx = RunCtx {
        pg: pg.clone(),
        run_id: params.run_id,
        seq: 0,
    };
    let deadline = Instant::now() + Duration::from_secs(params.wallclock_budget_secs as u64);

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
        // ── Budget gates (checked before every LLM call) ──────────────────
        if cancel.load(Ordering::SeqCst) {
            ctx.finish("cancelled", None, None, None, None).await;
            return;
        }
        if Instant::now() > deadline {
            ctx.finish(
                "failed",
                Some("budget_exhausted: wall-clock"),
                None,
                None,
                None,
            )
            .await;
            return;
        }
        if iterations >= params.max_iterations {
            ctx.finish(
                "failed",
                Some("budget_exhausted: max_iterations"),
                None,
                None,
                None,
            )
            .await;
            return;
        }
        if let Some(cap) = params.max_total_tokens {
            if tokens_in + tokens_out >= cap {
                ctx.finish("failed", Some("budget_exhausted: tokens"), None, None, None)
                    .await;
                return;
            }
        }

        iterations += 1;
        compact_transcript(&mut messages);
        let req = ChatRequest {
            model: params.model.clone(),
            system: Some(system.clone()),
            messages: messages.clone(),
            tools: tools.clone(),
            max_tokens: params.llm_max_tokens_per_call,
            temperature: None,
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
            let trimmed = content.trim_start();
            if trimmed.starts_with("FINAL") {
                let (strategy, backtest) = parse_final(&content);
                ctx.append("final", json!({ "content": content })).await;
                ctx.finish(
                    "completed",
                    None,
                    Some(&content),
                    strategy.as_deref(),
                    backtest,
                )
                .await;
                return;
            }
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
                    content: "Continue working with your tools, or if you are done reply with \
                              plain text starting with FINAL: per the termination contract."
                        .into(),
                });
                continue;
            }
            // Second bare reply — accept it as the summary rather than looping.
            ctx.append("final", json!({ "content": content })).await;
            ctx.finish("completed", None, Some(&content), None, None)
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
                    wait_for_backtest_inline(&mut ctx, &api, &backtest_id, deadline, &cancel).await
                }
            } else if call.name == "run_sweep" {
                run_sweep_inline(&mut ctx, &api, &call.arguments, deadline, &cancel).await
            } else {
                dispatch_tool(&mcp_ctx, &call.name, &call.arguments, None).await
            };

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
    fn parse_final_extracts_slug_and_backtest_id() {
        let content = "FINAL: built an EMA cross that beats hold on BTC-USD 1h.\n\
                       strategy: ema_cross_v3\n\
                       backtest_id: 6f9619ff-8b86-d011-b42d-00c04fc964ff\n\
                       Next I would try a volatility filter.";
        let (slug, id) = parse_final(content);
        assert_eq!(slug.as_deref(), Some("ema_cross_v3"));
        assert!(id.is_some());
    }

    #[test]
    fn parse_final_tolerates_missing_lines() {
        let (slug, id) = parse_final("FINAL: nothing beat buy-and-hold.");
        assert!(slug.is_none());
        assert!(id.is_none());
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
        compact_transcript(&mut messages);
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
