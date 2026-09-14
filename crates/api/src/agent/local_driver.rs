//! The IO half of the GOVERNOR loop (ADR-0032).
//!
//! [`harness::drive::Loop`] decides; this performs. The split is the whole design:
//! every decision lives in a synchronous state machine that can be driven by a
//! `Vec<Input>` on a machine with no accelerator, and everything here is a pump that
//! turns an [`Effect`] into an [`Input`].
//!
//! Which means the interesting rule about this file is what is **not** in it. No
//! policy, no validation, no budget arithmetic, no decision about when a task is
//! done. If a behaviour can be got wrong, it belongs on the other side of the
//! boundary where a test can reach it without a GPU.
//!
//! # The transcript lives here, not at the provider
//!
//! Each model call is a single-turn request: one system prompt, one user message,
//! rendered by the harness's context manager from blocks it owns. There is no
//! growing message array handed to the provider. That is what makes the context
//! budget enforceable — a transcript the provider accumulates is a transcript the
//! harness cannot compact (§4).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Map, Value};
use sqlx::PgPool;

use super::audit;
use tokio::time::Instant;
use uuid::Uuid;

use harness::adapter::{self, LocalExecutor};
use harness::canary::{self, Verdict};
use harness::drive::{Decode, Degradation, Effect, Input, Loop, Note, Outcome, Task};
use harness::hardware::{self, TaskDemand};
use harness::profile::Profile;
use llm::{ChatRequest, LlmClient, LlmError, Message, ToolDef};
use mcp_server_lib::{ApiClient, McpContext};

use super::{bridge, hardware_probe, workspace_tools};

/// How long a local backend should hold the weights resident between steps.
///
/// Measured on the dev box: without it a cold load costs ~100 s, and a loop that
/// takes 15 steps pays it 15 times. Not a performance tweak — a 25-minute tax on
/// every session.
const KEEP_ALIVE: &str = "30m";

/// How often to check whether a human has answered a pending approval.
const APPROVAL_POLL: Duration = Duration::from_secs(5);

pub struct LocalRunParams {
    pub run_id: Uuid,
    pub user_id: Uuid,
    /// The conversation this turn belongs to, and therefore whose workspace it uses.
    pub conversation_id: Uuid,
    /// The user's message for this turn.
    pub goal: String,
    pub charter: String,
    /// What earlier turns concluded. Empty on the first turn.
    pub prior: String,
    /// Where agent workspaces live. One folder per conversation underneath.
    pub workspace_root: std::path::PathBuf,
    pub profile: Profile,
    /// Namespaces the task starts from. `search_tools` adds more as it needs them.
    pub namespaces: Vec<String>,
    pub demand: TaskDemand,
    pub api_base: String,
    pub service_token: String,
}

// ── Persistence ─────────────────────────────────────────────────────────────

struct Recorder {
    pg: PgPool,
    run_id: Uuid,
    seq: i32,
    tokens_in: i64,
    tokens_out: i64,
}

impl Recorder {
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

    /// Timeline entries come from the loop already typed, so the UI and the auditor
    /// read the same record rather than two renderings of it.
    async fn note(&mut self, n: &Note) {
        let kind = match n {
            Note::Entered { .. } => "status",
            Note::Exposure { .. } => "exposure",
            Note::Rejected { .. } => "validation",
            Note::Ruled { .. } => "policy",
            Note::Compacted(_) => "compaction",
            Note::Degraded(_) => "degradation",
            Note::CoreTool { .. } => "core_tool",
            Note::Planned { .. } => "plan",
        };
        let payload = serde_json::to_value(n).unwrap_or_else(|_| json!({}));
        self.append(kind, payload).await;
    }

    /// One model decision, with its inputs.
    ///
    /// Deliberately the whole prompt rather than an excerpt. An excerpt is chosen by
    /// whoever wrote the excerpting rule, and the part that explains a surprising
    /// decision is exactly the part nobody thought worth keeping.
    async fn decision(
        &mut self,
        call: &harness::drive::ModelCall,
        raw: &str,
        resp: &llm::ChatResponse,
        latency_ms: u128,
    ) {
        let (phase, tool, schema) = match &call.decode {
            harness::drive::Decode::Plan { schema } => ("plan", None, Some(schema.clone())),
            harness::drive::Decode::SelectTool { schema, .. } => {
                ("select_tool", None, Some(schema.clone()))
            }
            harness::drive::Decode::FillArguments { tool, schema } => {
                ("fill_arguments", Some(tool.clone()), Some(schema.clone()))
            }
            harness::drive::Decode::Native { .. } => ("native", None, None),
        };
        self.append(
            "decision",
            json!({
                "step": call.step,
                "phase": phase,
                "tool": tool,
                // What the model read, exactly.
                "system": call.system,
                "prompt": call.prompt,
                // The grammar. Under constrained decoding this is not context, it is
                // the set of outputs that were physically reachable — which is often
                // the whole answer to "why did it pick that".
                "schema": schema,
                "raw_reply": raw,
                "settings": {
                    "temperature": call.temperature,
                    "max_tokens": call.max_tokens,
                    "num_ctx": call.num_ctx,
                },
                "usage": {
                    "input_tokens": resp.usage.input_tokens,
                    "output_tokens": resp.usage.output_tokens,
                    "latency_ms": u64::try_from(latency_ms).unwrap_or(u64::MAX),
                    "stop_reason": format!("{:?}", resp.stop_reason),
                },
            }),
        )
        .await;
    }

    /// Latency is recorded, not hidden. Local inference trades wall clock for cost,
    /// and §15.3 asks for that to be surfaced rather than made to look like a hang.
    async fn usage(&mut self, step: u32, input: u64, output: u64, latency_ms: u128) {
        self.tokens_in += input as i64;
        self.tokens_out += output as i64;
        self.append(
            "usage",
            json!({ "step": step, "input_tokens": input, "output_tokens": output,
                    "latency_ms": u64::try_from(latency_ms).unwrap_or(u64::MAX) }),
        )
        .await;
        let _ = sqlx::query(
            "UPDATE agent_runs SET iterations = $2, tokens_in = $3, tokens_out = $4
             WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(step as i32)
        .bind(self.tokens_in)
        .bind(self.tokens_out)
        .execute(&self.pg)
        .await;
    }

    async fn admitted(&self, profile: &Profile, hardware: &str) {
        let _ = sqlx::query(
            "UPDATE agent_runs SET profile_id = $2, tier = $3, hardware = $4, status = 'running',
                 started_at = COALESCE(started_at, now()) WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(&profile.model_id)
        .bind(profile.tier.as_str())
        .bind(hardware)
        .execute(&self.pg)
        .await;
    }

    /// Writes the typed terminal state.
    ///
    /// A fence is stored as its own status rather than folded into `failed`, because
    /// the two call for different actions: `failed` means look at the task, `fenced`
    /// means look at the **backend** — the harness is asserting it stopped getting
    /// the guarantee it was promised.
    async fn finish(&mut self, outcome: &Outcome) {
        let (status, summary, error) = match outcome {
            Outcome::Finished { result, evidence } => (
                "completed",
                Some(format!("{result}\n\nEvidence: {evidence}")),
                None,
            ),
            Outcome::Refused { reason, fix } => ("refused", None, Some(format!("{reason}. {fix}"))),
            Outcome::Escalated { reason, to } => {
                ("refused", None, Some(format!("{reason}; escalate to {to}")))
            }
            Outcome::Fenced(d) => ("fenced", None, Some(fence_message(d))),
            Outcome::Cancelled => ("cancelled", None, None),
        };
        let json = serde_json::to_value(outcome).unwrap_or_else(|_| json!({}));
        if let Some(err) = &error {
            self.append("error", json!({ "error": err })).await;
        }
        let _ = sqlx::query(
            "UPDATE agent_runs SET status = $2, summary = $3, error = $4, outcome_json = $5,
                 finished_at = now() WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(status)
        .bind(summary)
        .bind(error)
        .bind(&json)
        .execute(&self.pg)
        .await;
    }
}

/// The operator-facing form of a fence.
///
/// Deliberately blunt about where the fault is. Under constrained decoding the
/// harness is not guessing: the sampler cannot emit a token the grammar forbids, so a
/// violation is evidence about the backend, and burying that in "the model returned
/// unexpected output" sends the operator to debug a prompt.
fn fence_message(d: &Degradation) -> String {
    match d {
        Degradation::ConstraintIgnored {
            schema_for,
            detail,
            raw_excerpt,
        } => format!(
            "The backend ignored the grammar it was given for {schema_for}: {detail}. \
             Constrained decoding is not actually running, so this tier's guarantees are \
             absent and the session was stopped rather than continued without them. \
             Backend returned: {raw_excerpt}"
        ),
        Degradation::Stuck { code, attempts } => format!(
            "The same failure ({code}) survived {attempts} corrections. More retries \
             would spend the budget without changing the outcome."
        ),
        // Not usually a fence — the loop buys a wrap-up turn first — but it reaches
        // here when that turn is spent too, and it must not read as an exhausted
        // budget. The two call for different reactions: a spent budget means the task
        // was too big, this means the task stopped moving.
        Degradation::NoProgress { barren_steps } => format!(
            "The last {barren_steps} steps learned nothing new — no tool returned a \
             result and nothing was recorded as established. The run was stopped while \
             its findings were still worth reporting, rather than left to grind into \
             its step ceiling."
        ),
        Degradation::StepsExhausted { max } => {
            format!("The task did not finish within its {max}-step budget.")
        }
        Degradation::BackendFailure { detail } => format!("The backend is unusable: {detail}"),
        Degradation::OutputTruncated { reserve_for_output } => format!(
            "The model's replies kept being cut off at the {reserve_for_output}-token              output limit. This is a budget, not a model problem: raise              `context.reserve_for_output` in the profile, or give the task a tier whose              replies fit."
        ),
    }
}

// ── The canary, once per run ────────────────────────────────────────────────

/// Probes the backend and verifies the adapter against the profile.
///
/// Runs before the first task step. The cost is a handful of tiny completions; the
/// alternative is discovering four hours in that `constrained_decoding: true` was a
/// comment.
async fn run_canary(client: &LlmClient, profile: &Profile) -> Result<Vec<Verdict>, String> {
    let mut verdicts = Vec::new();
    for probe in canary::probes() {
        let req = ChatRequest {
            model: profile.model_id.clone(),
            system: Some("Answer the request.".into()),
            messages: vec![Message::User {
                content: probe.prompt.clone(),
            }],
            tools: Vec::new(),
            max_tokens: 256,
            temperature: Some(0.0),
            schema: Some(probe.schema.clone()),
            num_ctx: Some(2048),
            keep_alive: Some(KEEP_ALIVE.into()),
            tool_choice: None,
        };
        let verdict = match client.chat(&req).await {
            Ok(resp) => {
                let raw = resp.content.unwrap_or_default();
                canary::judge(&probe, Ok(&raw))
            }
            Err(e) => canary::judge(&probe, Err(&e.to_string())),
        };
        verdicts.push(verdict);
    }
    Ok(verdicts)
}

// ── The pump ────────────────────────────────────────────────────────────────

/// Runs one local-tier task to a terminal state.
#[allow(clippy::too_many_lines)]
pub async fn run(pg: PgPool, client: LlmClient, params: LocalRunParams, cancel: Arc<AtomicBool>) {
    // Every platform tool call this session makes is recorded before it is
    // authorised and after it is answered (SPEC §15, ADR-P2-21). The principal
    // is stamped from the session the API authenticated, so "agent X acting for
    // user Y" is a fact the agent has no field to write.
    let audit_trail = audit::AuditTrail::for_agent(pg.clone(), params.user_id, params.run_id);
    let mut rec = Recorder {
        pg: pg.clone(),
        run_id: params.run_id,
        seq: 0,
        tokens_in: 0,
        tokens_out: 0,
    };
    // No deadline. The agent runs until it calls `finish_task`.
    //
    // A wall clock could not tell a stuck session from one waiting on a forty-minute
    // sweep, so it killed both — throwing the work away rather than bounding it. The
    // stop conditions that remain all mean something: the user cancels, the retry
    // budget is spent on the same failure, or the backend breaks a guarantee.

    // ── Registry ────────────────────────────────────────────────────────────
    let registry = match bridge::registry_with_workspace() {
        Ok(r) => r,
        Err(e) => {
            rec.finish(&Outcome::Refused {
                reason: e.to_string(),
                fix: "classify the tool in crates/mcp-server/src/taxonomy.rs".into(),
            })
            .await;
            return;
        }
    };

    // ── Hardware, then admission ────────────────────────────────────────────
    let hw = hardware_probe::detect().await;
    let description = hardware_probe::describe(&hw);
    rec.admitted(&params.profile, &description).await;
    rec.append(
        "status",
        json!({ "hardware": description, "tier": params.profile.tier.as_str() }),
    )
    .await;

    // Two independent reasons a task may not run here, and both are stated before
    // anything is spent: the model does not fit the hardware, or the task needs more
    // tool calling than the tier provides.
    if let Some(req) = &params.profile.requires {
        if let Err(unfit) = hardware::fits(&hw, req) {
            // One exemption, and only for the memory arm: the model is ALREADY
            // resident. Free device memory is the right thing to fit against — on a
            // workstation the total is a different number from what is available —
            // but it makes loaded weights read as *used*, so the second run of a
            // session would be refused for the memory its own model is occupying.
            // Asking the backend what it is holding turns that false refusal back
            // into the correct answer: the weights are on the card, so they fit.
            //
            // Narrow on purpose. An architecture refusal is not waived by residency,
            // and a backend that cannot answer returns an empty list, which is
            // absence of evidence and leaves the refusal standing.
            let resident = matches!(unfit, hardware::Unfit::Memory { .. })
                && client
                    .resident_models()
                    .await
                    .iter()
                    .any(|m| m == &params.profile.model_id);
            if resident {
                rec.append(
                    "status",
                    json!({
                        "memory_check": "waived",
                        "why": format!(
                            "{} is already resident in the backend, so it fits by demonstration",
                            params.profile.model_id
                        ),
                    }),
                )
                .await;
            } else {
                rec.finish(&Outcome::Refused {
                    reason: unfit.explain(),
                    fix: "run a smaller quantisation, pool a second card, or escalate".into(),
                })
                .await;
                return;
            }
        }
    }
    // A `multi_step` claim that rests on a recorded eval is only good on the hardware
    // that eval was taken on. Re-checking it here, rather than trusting the load-time
    // validation, is what stops a profile from carrying its promotion to a machine
    // that never earned it.
    let (capability, demotion) = params.profile.effective_tool_calling(&hw);
    if let Some(reason) = &demotion {
        tracing::warn!(run_id = %params.run_id, reason, "multi_step promotion not licensed here");
    }
    rec.append(
        "status",
        json!({
            "tool_calling_declared": params.profile.tool_calling.as_str(),
            "tool_calling_effective": capability.as_str(),
            "demand": params.demand,
            // Present only when a recorded promotion did not survive the hardware
            // re-check. It belongs in the run record rather than only in a log,
            // because it changes what the run was allowed to do.
            "tool_calling_demoted_because": demotion,
        }),
    )
    .await;
    let admission = hardware::admit(
        capability,
        params.demand,
        params.profile.escalate_to.as_deref(),
    );

    // A task this tier cannot take is answered now, before the canary. The loop
    // already guarantees nothing is spent before the tier is known
    // (`a_task_too_big_for_the_tier_escalates_rather_than_running`); probing a backend
    // for a task about to be handed to a different one would break that guarantee on
    // the driver's side, where no test could see it.
    if !matches!(admission, hardware::Admission::Admit) {
        let mut refused = Loop::new(
            params.profile.clone(),
            registry,
            Task {
                goal: params.goal.clone(),
                charter: params.charter.clone(),
                demand: params.demand,
                namespaces: params.namespaces.clone(),
            },
        );
        for effect in refused.next(Input::Admission(admission)) {
            match effect {
                Effect::Note(n) => rec.note(&n).await,
                Effect::Done(o) => rec.finish(&o).await,
                _ => {}
            }
        }
        return;
    }

    // ── The canary ──────────────────────────────────────────────────────────
    if params.profile.output.constrained_decoding {
        let verdicts = match run_canary(&client, &params.profile).await {
            Ok(v) => v,
            Err(e) => {
                rec.finish(&Outcome::Fenced(Degradation::BackendFailure { detail: e }))
                    .await;
                return;
            }
        };
        rec.append(
            "canary",
            serde_json::to_value(&verdicts).unwrap_or_else(|_| json!([])),
        )
        .await;
        let executor = LocalExecutor::probed(
            params.profile.provider.clone(),
            params.profile.model_id.clone(),
            verdicts,
        );
        if let Err(e) = adapter::verify(&executor, &params.profile) {
            rec.finish(&Outcome::Fenced(Degradation::BackendFailure {
                detail: e.to_string(),
            }))
            .await;
            return;
        }
    }

    // ── The agent's own folder ──────────────────────────────────────────────
    let ws = workspace_tools::workspace_for(&params.workspace_root, params.conversation_id);
    if let Err(e) = workspace_tools::ensure(&ws).await {
        tracing::warn!(error = %e, "could not create the agent workspace");
    }

    // ── The loop ────────────────────────────────────────────────────────────
    let mcp_ctx = McpContext::new(ApiClient::new(params.api_base.clone(), params.service_token.clone()));

    let mut governor = Loop::new(
        params.profile.clone(),
        registry,
        Task {
            goal: if params.prior.trim().is_empty() {
                params.goal.clone()
            } else {
                format!(
                    "{}\n\nThe user now asks: {}",
                    params.prior.trim(),
                    params.goal
                )
            },
            charter: format!(
                "{}\n\n\u{2500}\u{2500} YOUR WORKSPACE \u{2500}\u{2500}\n\n{}",
                params.charter,
                workspace_tools::areas_hint()
            ),
            demand: params.demand,
            namespaces: params.namespaces.clone(),
        },
    )
    // The four §15 envelopes. The list comes from the platform, which owns the
    // classification; the loop pauses on them regardless of tier, attendance or
    // session allowlist (ADR-P2-20).
    .with_gated_actions(ledger::audit::gated_actions());

    let mut effects = governor.next(Input::Admission(admission));
    loop {
        let mut blocking: Option<Effect> = None;
        for e in effects {
            match e {
                Effect::Note(n) => rec.note(&n).await,
                Effect::Done(o) => {
                    rec.finish(&o).await;
                    return;
                }
                other => blocking = Some(other),
            }
        }
        let Some(blocking) = blocking else {
            // The loop asked for nothing and did not finish. That is a bug in the
            // state machine, and inventing an input here would hide it.
            tracing::error!(run_id = %params.run_id, "the loop yielded no effect and no outcome");
            rec.finish(&Outcome::Fenced(Degradation::BackendFailure {
                detail: "the loop yielded no effect and no outcome".into(),
            }))
            .await;
            return;
        };

        if cancel.load(Ordering::SeqCst) {
            effects = governor.next(Input::Cancel);
            continue;
        }

        let input = match blocking {
            Effect::CallModel(call) => {
                let started = Instant::now();
                let req = build_request(&params.profile, &call);
                match client.chat(&req).await {
                    Ok(resp) => {
                        rec.usage(
                            call.step,
                            resp.usage.input_tokens,
                            resp.usage.output_tokens,
                            started.elapsed().as_millis(),
                        )
                        .await;
                        let raw = reply_text(&call.decode, &resp);
                        // The decision record: everything that went INTO this choice,
                        // beside what came out of it.
                        //
                        // The event stream already said what the agent did. It could
                        // not say why, because the one thing that explains a model
                        // decision — the text the model actually read, and the grammar
                        // it was decoding under — was never written down anywhere.
                        // `harness::trace` records a prompt HASH by design, which is
                        // right for a durable audit trail and useless for answering a
                        // question: a hash can prove a replay rebuilt the same prompt
                        // and can never tell you what was in it.
                        //
                        // So the content lives here, on the run, where it can be read
                        // back and where it ages out with the run rather than becoming
                        // a permanent record of every prompt the platform ever sent.
                        rec.decision(&call, &raw, &resp, started.elapsed().as_millis())
                            .await;
                        // A cut-off reply is its own event. Reporting it as a complete
                        // one makes the loop read a `max_tokens` stop as proof the
                        // backend ignored the grammar, and fence a healthy session.
                        if resp.stop_reason == llm::StopReason::MaxTokens {
                            Input::ModelTruncated { raw }
                        } else {
                            Input::ModelReply { raw }
                        }
                    }
                    Err(e) => Input::ModelError {
                        retryable: is_retryable(&e),
                        detail: e.to_string(),
                    },
                }
            }

            Effect::ExecuteTool {
                step,
                name,
                namespace,
                arguments,
                risk,
                ..
            } => {
                rec.append(
                    "tool_call",
                    json!({ "step": step, "name": name, "risk": risk.as_str(),
                            "arguments": arguments }),
                )
                .await;
                // `fs` is the agent's own folder, not a platform call. Routed here
                // rather than in the loop so the loop stays sans-IO and knows nothing
                // about filesystems.
                let out = if workspace_tools::is_workspace_tool(&namespace) {
                    let started = Instant::now();
                    let (text, is_error) = workspace_tools::execute(&ws, &name, &arguments).await;
                    let raw = if is_error { json!({ "error": text }) } else { json!({ "result": text }) };
                    mcp_ctx.record_step(&name, &Value::Object(arguments.clone()), &raw, started.elapsed().as_millis(), None).await;
                    bridge::Executed {
                        raw: json!({ "result": text }),
                        text,
                        is_error,
                    }
                } else {
                    bridge::execute(&mcp_ctx, &name, &arguments, Some(&audit_trail)).await
                };
                rec.append(
                    "tool_result",
                    json!({ "name": name, "is_error": out.is_error, "content": out.raw }),
                )
                .await;
                if out.is_error {
                    Input::ToolError { detail: out.text }
                } else {
                    Input::ToolResult {
                        output: out.text,
                        source: bridge::source_for(&namespace),
                    }
                }
            }

            Effect::AskHuman {
                tool,
                arguments,
                ruling,
                ..
            } => {
                match await_approval(&pg, params.run_id, &tool, &arguments, &ruling, &cancel).await
                {
                    Some((granted, note)) => Input::Approval { granted, note },
                    None => Input::Cancel,
                }
            }

            Effect::Note(_) | Effect::Done(_) => unreachable!("handled above"),
        };

        effects = governor.next(input);
    }
}

/// Builds one provider request from one [`harness::drive::ModelCall`].
///
/// The mapping is mechanical on purpose: every value here was decided by the loop,
/// and a driver that second-guessed one of them would be enforcing a policy in the
/// place the design put outside the tested boundary.
fn build_request(profile: &Profile, call: &harness::drive::ModelCall) -> ChatRequest {
    let (schema, tools) = match &call.decode {
        Decode::Native { tools } => (None, native_tools(tools)),
        other => (other.schema().cloned(), Vec::new()),
    };
    ChatRequest {
        model: profile.model_id.clone(),
        system: Some(call.system.clone()),
        messages: vec![Message::User {
            content: call.prompt.clone(),
        }],
        tools,
        max_tokens: call.max_tokens,
        temperature: Some(call.temperature),
        schema,
        // Sent explicitly, always. A backend defaulting to its own window truncates
        // from the LEFT, which eats the charter and the tool schemas first and reads
        // as the model having become stupid.
        num_ctx: Some(call.num_ctx),
        keep_alive: profile.tier.is_local().then(|| KEEP_ALIVE.to_string()),
        tool_choice: None,
    }
}

fn native_tools(schemas: &[Value]) -> Vec<ToolDef> {
    schemas
        .iter()
        .filter_map(|s| {
            Some(ToolDef {
                name: s.get("name")?.as_str()?.to_string(),
                description: s
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input_schema: s.get("inputSchema").cloned()?,
            })
        })
        .collect()
}

/// Normalises a provider reply to the one shape the loop parses.
///
/// Under a grammar the answer is conformant JSON in the content. On the native path
/// it is a structured tool call, which is re-serialised here rather than in the loop —
/// the loop must not learn what a provider's tool call looks like.
fn reply_text(decode: &Decode, resp: &llm::ChatResponse) -> String {
    if let Decode::Native { .. } = decode {
        if let Some(c) = resp.tool_calls.first() {
            return json!({"name": c.name, "arguments": c.arguments}).to_string();
        }
    }
    resp.content.clone().unwrap_or_default()
}

/// A transport problem is worth re-asking; a refusal is not.
///
/// Getting this wrong in the permissive direction turns a 4 GB model that cannot be
/// loaded into three identical failures; getting it wrong the other way turns a
/// dropped connection into a dead session.
fn is_retryable(e: &LlmError) -> bool {
    matches!(
        e,
        LlmError::Network(_) | LlmError::RateLimited { .. } | LlmError::Overloaded
    )
}

// ── Approvals ───────────────────────────────────────────────────────────────

/// Persists the request, then waits for a human.
///
/// The row is written **before** the wait, so an approval outlives this process: a
/// restart loses the wait, not the question (§14.3). Returns `None` if the run was
/// cancelled or ran out of wall clock while waiting.
async fn await_approval(
    pg: &PgPool,
    run_id: Uuid,
    tool: &str,
    arguments: &Map<String, Value>,
    ruling: &harness::Ruling,
    cancel: &AtomicBool,
) -> Option<(bool, String)> {
    let approval_id = Uuid::new_v4();
    let payload = json!({
        "tool": tool,
        "arguments": arguments,
        "reason": ruling.reason,
        "fix": ruling.fix,
        "code": ruling.code,
    });
    if let Err(e) = sqlx::query(
        "INSERT INTO approval_requests
             (approval_id, run_id, kind, payload, options, default_option, state)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending')",
    )
    .bind(approval_id)
    .bind(run_id)
    // One kind for every policy pause; the code that fired is in the payload, where
    // the UI reads it without needing a new kind per rule.
    .bind("tool_call")
    .bind(&payload)
    .bind(json!(["approve", "refuse"]))
    // No timeout default. An unanswered destructive action stays unanswered; a
    // default of "approve" would make the approval decorative and a default of
    // "refuse" would silently drop work a human meant to allow.
    .bind(Option::<String>::None)
    .execute(pg)
    .await
    {
        tracing::error!(error = %e, "could not persist the approval request");
        return Some((false, "the approval could not be recorded".into()));
    }

    // The run says out loud that it is blocked on a person. Left as `running`, a
    // session waiting overnight for an approval is indistinguishable from one that is
    // working, and `recover_orphans` would treat it the same way on a restart.
    let _ = sqlx::query("UPDATE agent_runs SET status = 'awaiting_approval' WHERE run_id = $1")
        .bind(run_id)
        .execute(pg)
        .await;

    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = sqlx::query(
                "UPDATE approval_requests SET state = 'cancelled' WHERE approval_id = $1
                 AND state = 'pending'",
            )
            .bind(approval_id)
            .execute(pg)
            .await;
            return None;
        }
        let row: Option<(String, Option<Value>)> =
            sqlx::query_as("SELECT state, answer FROM approval_requests WHERE approval_id = $1")
                .bind(approval_id)
                .fetch_optional(pg)
                .await
                .ok()
                .flatten();

        if let Some((state, answer)) = row {
            if state == "answered" || state == "defaulted" {
                let _ = sqlx::query(
                    "UPDATE agent_runs SET status = 'running' WHERE run_id = $1
                     AND status = 'awaiting_approval'",
                )
                .bind(run_id)
                .execute(pg)
                .await;
                let a = answer.unwrap_or_else(|| json!({}));
                let granted = a
                    .get("option")
                    .and_then(Value::as_str)
                    .is_some_and(|o| o == "approve");
                let note = a
                    .get("note")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                return Some((granted, note));
            }
            if state == "cancelled" {
                return None;
            }
        }
        tokio::time::sleep(APPROVAL_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::profile::ProfileSet;

    fn profile() -> Profile {
        ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
            .expect("the shipped profiles load")
            .get("qwen3.6-35b-a3b")
            .expect("the reference profile is shipped")
            .clone()
    }

    #[test]
    fn a_constrained_call_sends_the_schema_and_no_tools() {
        let p = profile();
        let call = harness::drive::ModelCall {
            step: 1,
            system: "charter".into(),
            prompt: "pick one".into(),
            decode: Decode::SelectTool {
                schema: json!({"type":"object","properties":{"name":{"enum":["a"]}}}),
                candidates: vec!["a".into()],
            },
            temperature: 0.0,
            max_tokens: 3000,
            num_ctx: 24_000,
        };
        let req = build_request(&p, &call);
        assert!(req.schema.is_some());
        assert!(
            req.tools.is_empty(),
            "under the two-step decode the grammar IS the tool interface"
        );
        assert_eq!(req.num_ctx, Some(24_000));
        assert_eq!(req.keep_alive.as_deref(), Some(KEEP_ALIVE));
        assert_eq!(req.temperature, Some(0.0));
    }

    /// The context window is never left to the backend's default. Omitting it is a
    /// silent left-truncation that removes the charter first.
    #[test]
    fn the_context_window_is_always_stated() {
        let p = profile();
        for decode in [
            Decode::Plan {
                schema: json!({"type":"object"}),
            },
            Decode::Native { tools: vec![] },
        ] {
            let call = harness::drive::ModelCall {
                step: 1,
                system: String::new(),
                prompt: String::new(),
                decode,
                temperature: 0.0,
                max_tokens: 100,
                num_ctx: 24_000,
            };
            assert_eq!(build_request(&p, &call).num_ctx, Some(24_000));
        }
    }

    #[test]
    fn a_native_tool_call_is_reserialised_into_the_shape_the_loop_parses() {
        let resp = llm::ChatResponse {
            content: None,
            tool_calls: vec![llm::ToolCall {
                id: "call_1".into(),
                name: "get_bars".into(),
                arguments: json!({"instrument": "BTC-USD"}),
            }],
            stop_reason: llm::StopReason::ToolUse,
            usage: llm::Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        };
        let raw = reply_text(&Decode::Native { tools: vec![] }, &resp);
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["name"], "get_bars");
        assert_eq!(v["arguments"]["instrument"], "BTC-USD");
    }

    /// A grammar puts the answer in the content, not in `tool_calls` — measured, and
    /// the reason the local path reads content at all.
    #[test]
    fn a_constrained_reply_is_read_from_the_content() {
        let resp = llm::ChatResponse {
            content: Some(r#"{"name":"get_bars"}"#.into()),
            tool_calls: vec![],
            stop_reason: llm::StopReason::EndTurn,
            usage: llm::Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        };
        let raw = reply_text(
            &Decode::SelectTool {
                schema: json!({}),
                candidates: vec![],
            },
            &resp,
        );
        assert_eq!(raw, r#"{"name":"get_bars"}"#);
    }

    #[test]
    fn a_dropped_connection_is_retried_and_a_missing_model_is_not() {
        assert!(is_retryable(&LlmError::Network("reset".into())));
        assert!(!is_retryable(&LlmError::Auth("no key".into())));
        assert!(!is_retryable(&LlmError::InvalidRequest(
            "no such model".into()
        )));
        assert!(
            !is_retryable(&LlmError::ContextTooLarge("32000 > 24000".into())),
            "a budget that does not match the model is a configuration error; retrying              the identical request cannot fix it"
        );
    }

    /// The operator-facing text has to name the backend, not the model. Sending
    /// someone to debug a prompt when the backend dropped the grammar wastes the one
    /// signal the fence exists to produce.
    #[test]
    fn a_fence_message_blames_the_backend_and_carries_the_evidence() {
        let m = fence_message(&Degradation::ConstraintIgnored {
            schema_for: "the tool selection".into(),
            detail: "\"purple\" is not in the enum".into(),
            raw_excerpt: "{\"colour\":\"purple\"}".into(),
        });
        assert!(m.contains("backend ignored the grammar"));
        assert!(m.contains("purple"));
    }
}
