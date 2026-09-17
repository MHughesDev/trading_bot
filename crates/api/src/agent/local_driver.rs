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
use tokio::time::Instant;
use uuid::Uuid;

use harness::adapter::{self, LocalExecutor};
use harness::canary::{self, Verdict};
use harness::drive::{Decode, Degradation, Effect, Input, Loop, Note, Outcome, Task};
use harness::hardware::{self, TaskDemand};
use harness::profile::Profile;
use llm::{ChatRequest, LlmClient, LlmError, Message, ToolDef};
use mcp_server_lib::{ApiClient, McpContext};

use super::{bridge, hardware_probe, prompt, subrun, workspace_tools};

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
    /// The run's constraints — instrument, timeframe, window, balance.
    ///
    /// Carried here so both drivers build their opening message the same way
    /// (S-1.6). `{}` when the caller has none, which is the chat path's normal
    /// case: an empty object renders no constraints block rather than a block
    /// saying nothing.
    pub constraints: serde_json::Value,
    /// Where agent workspaces live. One folder per conversation underneath.
    pub workspace_root: std::path::PathBuf,
    pub profile: Profile,
    /// Namespaces the task starts from. `search_tools` adds more as it needs them.
    pub namespaces: Vec<String>,
    /// Which stage skeleton this run follows (AGENT-013 §3; S-10.14).
    ///
    /// Carried rather than derived from the goal text, because the user may
    /// override it with a chip in the composer and a re-derivation would
    /// silently discard that.
    pub workflow: harness::state::Workflow,
    /// The tool slice the previous turn of this conversation ended with
    /// (AGENT-005 §21; S-9.9).
    ///
    /// Empty on the first turn. Carried so the next turn's prompt is a byte
    /// superset of this one's rather than a fresh assembly that happens to
    /// contain the same tools in a different order — which a provider cache
    /// cannot tell apart from a different tool list.
    pub prior_exposure: Vec<String>,
    /// Where this turn writes the slice it ended with, for the next turn.
    ///
    /// A shared slot rather than a return value because `run` returns from a
    /// dozen places — a fence, a cancel, an unresumable suspension — and a
    /// return value would have to be threaded through every one of them. The
    /// one that was forgotten would silently go back to paying a cold prefix,
    /// which is a defect that shows up as a bill and never as a failure.
    ///
    /// Written on every exposure note, so whichever way the run ends the slot
    /// holds the last slice it actually used.
    pub exposure_out: Arc<std::sync::Mutex<Vec<String>>>,
    pub demand: TaskDemand,
    pub api_base: String,
    pub service_token: String,
}

/// What this run has committed to, for the trial gate (AGENT-013 §7.2; S-10.3).
///
/// The head of the run's amendment chain — the row with the highest amendment
/// number, which is the one with no successor.
///
/// A Chat turn is [`Prereg::NotRequired`] rather than `Missing`, and the
/// difference is not cosmetic: a Chat turn is already refused by
/// `for_chat_turn`, and telling it to go and pre-register would point it at a
/// tool it is not offered. Two refusals for one situation, one of which names an
/// impossible fix, is worse than one.
///
/// A read that fails is `Missing`, deliberately. The alternative — treating a
/// database error as "no registration required" — would make an outage open the
/// gate, and the gate is the thing standing between a run and the trial budget.
async fn sealed_registration(
    pg: &PgPool,
    run_id: Uuid,
    demand: TaskDemand,
) -> mcp_server_lib::gate::Prereg {
    use mcp_server_lib::gate::Prereg;
    if demand == TaskDemand::SingleLookup {
        return Prereg::NotRequired;
    }
    let row: Result<Option<String>, _> = sqlx::query_scalar(
        "SELECT prereg_hash FROM agent_lifecycle.preregistration           WHERE run_id = $1 ORDER BY amendment DESC LIMIT 1",
    )
    .bind(run_id)
    .fetch_optional(pg)
    .await;
    match row {
        Ok(Some(hash)) => Prereg::Sealed(hash),
        Ok(None) => Prereg::Missing,
        Err(e) => {
            tracing::warn!(
                %run_id, error = %e,
                "could not read this run's pre-registration; trials are refused"
            );
            Prereg::Missing
        }
    }
}

/// The catalogue tools that reach paper trading (AGENT-013 §4; S-10.1).
///
/// Derived from `ToolSpec::ring`, which derives it from `Envelope::Promotion` —
/// so a tool that starts reaching paper is caught by the derivation rather than
/// by somebody remembering to add it to a list. The harness cannot compute this
/// itself: it is sans-IO and has no catalogue.
#[must_use]
pub fn paper_reaching_tools() -> Vec<&'static str> {
    mcp_server_lib::spec::names()
        .into_iter()
        .filter(|n| {
            mcp_server_lib::spec::find(n)
                .is_some_and(|s| s.ring() == mcp_server_lib::spec::RING_PAPER)
        })
        .collect()
}

/// The catalogue tools that spend a trial (AGENT-005 §20; S-9.7).
///
/// Read from the catalogue's own `spends_trial` declaration, which is the same
/// source the trial *counter* reads below. One source, so a tool cannot be
/// charged a trial by one path and sized as a cheap lookup by the other.
#[must_use]
pub fn trial_spending_tools() -> Vec<&'static str> {
    mcp_server_lib::spec::names()
        .into_iter()
        .filter(|n| {
            mcp_server_lib::spec::find(n)
                .is_some_and(|s| s.spends_trial != mcp_server_lib::spec::TrialKind::None)
        })
        .collect()
}

/// The loop's own word for how a run ended (§4, S-2.8).
///
/// Deliberately not the UI's `status`, which collapses `Refused` and `Escalated`
/// into "refused" because the timeline draws them the same. An eval that cannot
/// tell an escalation from a refusal is measuring the wrong thing: one is the run
/// declining, the other is the run asking for a bigger model.
fn outcome_word(o: &Outcome) -> &'static str {
    match o {
        Outcome::Finished { .. } => "finished",
        Outcome::Refused { .. } => "refused",
        Outcome::Escalated { .. } => "escalated",
        Outcome::Fenced(_) => "fenced",
        Outcome::Cancelled => "cancelled",
    }
}

/// One input, as the log records it.
///
/// The counterpart to [`describe_effect`], and recorded for the same reason: a
/// replay has to know not only what the loop decided but what it was told, because
/// the pair is what determines where a crash fell.
fn describe_input(i: &Input) -> (&'static str, Value) {
    match i {
        Input::Admission(a) => ("Admission", serde_json::json!({ "admission": format!("{a:?}") })),
        Input::ModelReply { raw } => (
            "ModelReply",
            // The reply in full. It is what the loop parsed, and a replay that
            // re-parses something else is not a replay.
            serde_json::json!({ "raw": raw }),
        ),
        Input::ModelTruncated { raw } => {
            ("ModelTruncated", serde_json::json!({ "raw": raw }))
        }
        Input::ModelError { detail, retryable } => (
            "ModelError",
            serde_json::json!({ "detail": detail, "retryable": retryable }),
        ),
        Input::ToolResult { output, source } => (
            "ToolResult",
            serde_json::json!({ "output": output, "source": source }),
        ),
        Input::ToolError { detail } => ("ToolError", serde_json::json!({ "detail": detail })),
        Input::Approval { granted, note } => (
            "Approval",
            serde_json::json!({ "granted": granted, "note": note }),
        ),
        Input::Cancel => ("Cancel", serde_json::json!({})),
        other => (
            // A variant added after this was written. Named by its debug form rather
            // than silently dropped: a log missing a row is a log that cannot be
            // replayed, and the failure should be visible in the row itself.
            "Other",
            serde_json::json!({ "debug": format!("{other:?}") }),
        ),
    }
}

/// [`describe_effect`], reachable from the trace projection's tests.
///
/// The projection consumes exactly what this produces, and nothing in the type
/// system connects the two — so the test that checks they agree has to call the
/// real producer rather than build a fixture that resembles it.
#[cfg(test)]
pub(crate) fn describe_effect_for_test(e: &Effect) -> (&'static str, Value) {
    describe_effect(e)
}
/// One effect, as the log records it (AGENT-010 §5, S-2.5).
///
/// The kind is the **variant name**, not a presentational category. `agent_messages`
/// keys on `assistant | tool_call | status | final`, which can draw a timeline and
/// cannot drive a replay: it cannot say which effect the loop was serving when a
/// tool ran, and it cannot distinguish an effect the loop decided on from one it
/// carried out.
///
/// The payloads are deliberately small. A replay needs to know *what was decided*,
/// and the prompt that produced the decision is already recorded by hash in
/// `harness::trace` — a log carrying full prompts would be a permanent record of
/// every prompt the platform ever sent, kept for a property that a hash provides.
fn describe_effect(e: &Effect) -> (&'static str, Value) {
    match e {
        Effect::CallModel(c) => (
            "CallModel",
            serde_json::json!({
                "step": c.step,
                "decode": c.decode.label(),
                "max_tokens": c.max_tokens,
                "num_ctx": c.num_ctx,
                "prompt_bytes": c.prompt.len(),
            }),
        ),
        Effect::ExecuteTool {
            step,
            name,
            namespace,
            arguments,
            risk,
            idempotent,
        } => (
            "ExecuteTool",
            serde_json::json!({
                "step": step,
                "name": name,
                "namespace": namespace,
                // The arguments in full: they are what a resume must NOT run twice,
                // and a hash of them cannot answer "was this the same call".
                "arguments": arguments,
                "risk": format!("{risk:?}"),
                "idempotent": idempotent,
            }),
        ),
        Effect::AskHuman { step, tool, ruling, .. } => (
            "AskHuman",
            serde_json::json!({ "step": step, "tool": tool, "ruling": format!("{ruling:?}") }),
        ),
        Effect::Await { step, job_id, tool } => (
            "Await",
            serde_json::json!({ "step": step, "job_id": job_id, "tool": tool }),
        ),
        // The row is logged by id and confidence, not in full. What a replay
        // needs is that the loop decided to remember something and which
        // verdict it reached; the claim text is already in the note beside it.
        Effect::Remember(e) => (
            "Remember",
            serde_json::json!({
                "entry_id": e.id,
                "kind": e.kind.as_str(),
                "confidence": e.confidence.as_str(),
                "window": format!("{}→{}", e.data_window.start, e.data_window.end),
            }),
        ),
        // A spill is logged by handle and size, not by content: the content is
        // now *in* the workspace and logging it here would be a second copy of
        // the bytes the spill exists to move out of memory.
        Effect::Spill { record, handle, body } => (
            "Spill",
            serde_json::json!({
                "record": record.to_string(),
                "handle": handle,
                "tokens": body.tokens(),
            }),
        ),
        // A core-tool note is the only note a replay needs, and it is recorded
        // structurally rather than as a debug string.
        //
        // `search_tools`, `record_finding`, `reflect` and `finish_task` are
        // never dispatched — the gate short-circuits them, on the grounds that
        // ruling on them would be the harness ruling on itself — so they never
        // become `ExecuteTool` rows. A trajectory without them cannot replay a
        // run whose whole content was findings, which is ADR-P0-18's gap and the
        // `honesty` suite's cases are made of exactly these calls.
        //
        // Structured, because the alternative is reading the name back out of a
        // `{n:?}`, and a field recovered by parsing a debug format is a field
        // one derive away from being gone.
        Effect::Note(Note::CoreTool { name, detail }) => (
            "Note",
            serde_json::json!({ "core_tool": name, "detail": detail }),
        ),
        Effect::Note(n) => (
            "Note",
            serde_json::json!({ "note": format!("{n:?}") }),
        ),
        Effect::Done(o) => (
            "Done",
            serde_json::json!({ "outcome": format!("{o:?}") }),
        ),
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

struct Recorder {
    pg: PgPool,
    run_id: Uuid,
    seq: i32,
    tokens_in: i64,
    tokens_out: i64,
    /// How big the prompts were, summed. **Not** `tokens_in` (§4, S-2.8).
    ///
    /// With caching on the two differ by the cache hit rate: a 40,000-token prompt of
    /// which 39,000 was a hit bills 1,000. `tokens_in` answers "what did this cost";
    /// this answers "how big was the prompt", and only this one may be compared
    /// against a context budget.
    prompt_tokens: i64,
    /// What the harness estimated for the same requests, by the arithmetic the
    /// assembler budgeted with. The pair is the ratio S-1.18 calibrates on; either
    /// number alone has nothing to check it.
    est_prompt_tokens: i64,
    cache_read_tokens: i64,
    /// Trials this run spent (§4, S-2.8).
    ///
    /// Counted from the catalogue's own `spends_trial` declaration rather than from
    /// anything the loop tracks, because the declaration is the single place a tool
    /// says whether calling it registers a trial (AGENT-006 §2.5) and a second
    /// counter would be a second thing to keep in agreement with it.
    ///
    /// Counted at **dispatch**, not at success: a trial is registered before the tool
    /// runs, so a call that failed still spent one. Counting successes would report a
    /// run as having searched less than it did, which is the direction that flatters.
    trials_spent: i32,
    /// The effect log's own sequence, separate from `agent_messages`'s.
    ///
    /// Separate because the two record different things at different rates: one row
    /// per rendered timeline entry against one row per effect and per input. Sharing
    /// a counter would make each log's ordering depend on how often the other was
    /// written, which is the one property an effect log cannot afford to lose.
    event_seq: i32,
    /// Where the turn's last tool slice is left for the next turn (§21; S-9.9).
    exposure_out: Arc<std::sync::Mutex<Vec<String>>>,
    /// What this run produced (AGENT-013 §10; S-10.13).
    ///
    /// Accumulated as the run goes rather than reconstructed at the end, because
    /// the end is exactly where a fenced or cancelled run has least state — and
    /// a yield only computable on the happy path would report every failure as
    /// having produced nothing, which is the one reading that is never right.
    yielded: harness::lifecycle::r#yield::Yield,
}

/// The harness that wrote a log. A replay by a different one is not a replay.
///
/// The crate version rather than a hand-maintained constant: a constant is a thing
/// to forget to bump, and what matters is only that it changes when the loop's
/// behaviour can.
pub const HARNESS_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The gate profile a run is judged under when nothing pins another.
///
/// `strict_v1`, which is also `gate_worker::profile_for`'s fallback — the same
/// string in two places would be two answers to "what is this run judged
/// against", and the one a skeleton wrote would be the one a registration got
/// stamped with.
pub const DEFAULT_GATE_PROFILE: &str = "strict_v1";

impl Recorder {
    /// One row of the effect log (AGENT-010 §5, S-2.5).
    ///
    /// **Called before the effect is executed, and before an input is fed back.**
    /// That ordering is the entire value of the log: a row written afterwards
    /// records what happened, which a transcript already does; a row written
    /// beforehand records what was *about to* happen, which is the only thing that
    /// can tell a crash-before from a crash-after — and therefore the only thing
    /// that can decide whether resuming would write twice.
    ///
    /// Failures are logged and swallowed rather than propagated. That direction is
    /// deliberate and is the opposite of the audit sink's: the audit trail gates
    /// dispatch because a look nobody counted is the thing this platform claims it
    /// cannot take, whereas a missing effect row costs a *resume* — the run in front
    /// of the user still works. Killing a live run to protect its resumability would
    /// be trading the certain thing for the contingent one.
    async fn event(&mut self, direction: &str, kind: &str, payload: Value) {
        self.event_seq += 1;
        if let Err(e) = sqlx::query(
            "INSERT INTO agent_effect_log              (run_id, seq, direction, kind, payload_json, harness_version)              VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(self.run_id)
        .bind(self.event_seq)
        .bind(direction)
        .bind(kind)
        .bind(&payload)
        .bind(HARNESS_VERSION)
        .execute(&self.pg)
        .await
        {
            tracing::warn!(run_id = %self.run_id, seq = self.event_seq, error = %e,
                "the effect log lost a row; this run is no longer resumable");
        }
    }

    /// Every effect of one turn, recorded before any of them runs.
    async fn effects(&mut self, fx: &[Effect]) {
        for e in fx {
            let (kind, payload) = describe_effect(e);
            self.event("effect", kind, payload).await;
        }
    }

    /// The input about to be fed back in.
    async fn input(&mut self, kind: &str, payload: Value) {
        self.event("input", kind, payload).await;
    }

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
            Note::Exposure { exposed, .. } => {
                // The next turn starts from this (§21). Written here rather than
                // returned, so every exit path records it.
                if let Ok(mut slot) = self.exposure_out.lock() {
                    slot.clone_from(exposed);
                }
                "exposure"
            }
            Note::Rejected { .. } => "validation",
            Note::Ruled { .. } => "policy",
            Note::Compacted(_) => "compaction",
            // Its own timeline kind, not "compaction": one says what was
            // chosen and the other what then had to be cut to fit, and the
            // scrubber needs to ask for either without getting both.
            Note::Selected { .. } => "selection",
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
    async fn usage(
        &mut self,
        step: u32,
        usage: &llm::Usage,
        estimated: u32,
        latency_ms: u128,
    ) {
        let (input, output) = (usage.input_tokens, usage.output_tokens);
        self.tokens_in += input as i64;
        self.tokens_out += output as i64;
        // The prompt as it actually was, and as the harness thought it would be
        // (§4, S-2.8). Accumulated per call because the ratio that calibrates the
        // estimator is over calls, and a run total would hide a systematic
        // under-estimate on long steps behind an over-estimate on short ones.
        self.prompt_tokens += i64::try_from(usage.prompt_tokens()).unwrap_or(i64::MAX);
        self.est_prompt_tokens += i64::from(estimated);
        self.cache_read_tokens +=
            i64::try_from(usage.cache_read_input_tokens).unwrap_or(i64::MAX);
        self.append(
            "usage",
            json!({ "step": step, "input_tokens": input, "output_tokens": output,
                    "prompt_tokens": usage.prompt_tokens(),
                    "est_prompt_tokens": estimated,
                    "cache_read_tokens": usage.cache_read_input_tokens,
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

    async fn admitted(&self, profile: &Profile, hardware: &str, adapter: &str) {
        // `profile_name` beside `profile_id`, and they are not the same thing
        // (§4, S-2.8). The id is what the loader keyed on; the name is what a person
        // reads in a scorecard, and an id renamed underneath silently changes what a
        // historical row meant.
        //
        // `transport` is the decode style, which is what actually differs between two
        // runs of the same model: `select_fill` is two constrained calls per step and
        // `native` is one tool call, and an eval that averages them is measuring the
        // mixture rather than either.
        let transport = if profile.output.constrained_decoding {
            "select_fill"
        } else {
            "native"
        };
        let _ = sqlx::query(
            "UPDATE agent_runs SET profile_id = $2, tier = $3, hardware = $4, status = 'running',
                 profile_name = $5, adapter = $6, transport = $7, harness_version = $8,
                 started_at = COALESCE(started_at, now()) WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(&profile.model_id)
        .bind(profile.tier.as_str())
        .bind(hardware)
        // There is no separate display name on `Profile`; the model id is what the
        // loader keys on and what a reader recognises. Bound explicitly rather than
        // left null so the column means something on every row, and a real display
        // name can replace it without changing what the column is for.
        .bind(&profile.model_id)
        .bind(adapter)
        .bind(transport)
        .bind(HARNESS_VERSION)
        .execute(&self.pg)
        .await;
    }

    /// Writes the typed terminal state.
    ///
    /// A fence is stored as its own status rather than folded into `failed`, because
    /// the two call for different actions: `failed` means look at the task, `fenced`
    /// means look at the **backend** — the harness is asserting it stopped getting
    /// the guarantee it was promised.
    /// Store a finding the loop verified (AGENT-007 §2.2; S-4.2).
    ///
    /// The loop leaves `provenance.run_id` blank because it is sans-IO and does
    /// not know which run it is. Stamping it here is not a convenience:
    /// `Entry::validate` refuses empty provenance, so a driver that forgot would
    /// fail at the store rather than write a row attributed to nothing.
    ///
    /// A failure is a warning. The finding is already in the working state and
    /// already in this turn's summary; losing the durable copy costs the *next*
    /// run a head start and costs this one nothing, and failing the run over it
    /// would trade a result for a cache.
    async fn remember(&mut self, user_id: Uuid, entry: &harness::memory::Entry) {
        let mut entry = entry.clone();
        entry.provenance.run_id = self.run_id.to_string();
        let store = crate::memory_store::MemoryStore::new(self.pg.clone());
        if let Err(e) = store.put(user_id, &entry).await {
            tracing::warn!(
                run_id = %self.run_id,
                entry = %entry.id,
                error = %e,
                "the finding did not persist; it is still in this run's state and summary"
            );
        }
    }

    /// Write the structured summary of this turn (AGENT-007 §2.1; S-4.1).
    ///
    /// Built from the loop's own state and the two ledgers — never from the
    /// model's closing prose, which is what `agent_runs.summary` holds and what
    /// the carry used to read. `TurnSummary::from_state` has no parameter
    /// through which prose could arrive.
    ///
    /// Failure is logged and not propagated. A turn whose summary does not
    /// store has still finished, and failing the run here would turn a carry
    /// problem into a lost result; `prior_context` falls back to the prose path
    /// and says so in a warning.
    async fn write_turn_summary(
        &mut self,
        conversation_id: Uuid,
        governor: &Loop,
        outcome: &Outcome,
    ) {
        let status = match outcome {
            Outcome::Finished { .. } => "finished",
            Outcome::Refused { .. } => "refused",
            Outcome::Escalated { .. } => "escalated",
            Outcome::Fenced(_) => "fenced",
            Outcome::Cancelled => "cancelled",
        };

        // The turn this run is. Read rather than passed, so no call site has to
        // carry it and none can pass a stale one.
        let turn: i32 = sqlx::query_scalar(
            "SELECT COALESCE(turn_index, 0) FROM agent_runs WHERE run_id = $1",
        )
        .bind(self.run_id)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()
        .unwrap_or(0);

        let summary = harness::memory::TurnSummary::from_state(
            turn,
            status,
            governor.working(),
            governor.summary_findings().to_vec(),
            // Negative results are derived by the platform from the funnel's
            // verdict (S-4.4), not by the loop, so the loop has none to give.
            // An empty list is the honest answer rather than a place to guess.
            Vec::new(),
            harness::memory::TurnBudget {
                tokens: self.prompt_tokens.max(0).unsigned_abs(),
                usd_tenth_cents: 0,
            },
        );

        let store = crate::turn_summary_store::TurnSummaryStore::new(self.pg.clone());
        if let Err(e) = store.put(conversation_id, &summary).await {
            tracing::warn!(
                run_id = %self.run_id,
                error = %e,
                "the turn summary did not store; the next turn falls back to the prose carry"
            );
        }
    }

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
        // Trials are counted at dispatch, in the effect loop; the yield reads
        // the same number rather than keeping its own.
        self.yielded.trials_spent = u32::try_from(self.trials_spent).unwrap_or(0);
        let barren = self.yielded.is_barren();
        if let Some(err) = &error {
            self.append("error", json!({ "error": err })).await;
        }
        let _ = sqlx::query(
            "UPDATE agent_runs SET status = $2, summary = $3, error = $4, outcome_json = $5,
                 harness_version = $6, prompt_tokens = $7, est_prompt_tokens = $8,
                 cache_read_tokens = $9, outcome = $10, trials_spent = $11,
                 findings_established = $12, findings_suggestive = $13,
                 findings_refuted = $14, negatives = $15, negatives_new_class = $16,
                 questions_opened = $17, questions_closed = $18,
                 contradictions_surfaced = $19, proposals = $20,
                 hypotheses_tested = $21, hypotheses_refuted = $22,
                 hypotheses_supported = $23, barren = $24,
                 finished_at = now() WHERE run_id = $1",
        )
        .bind(self.run_id)
        .bind(status)
        .bind(summary)
        .bind(error)
        .bind(&json)
        .bind(HARNESS_VERSION)
        .bind(self.prompt_tokens)
        .bind(self.est_prompt_tokens)
        .bind(self.cache_read_tokens)
        // The loop's own word for how it ended, not the UI's status. `outcome_json`
        // holds the whole thing; this is the part an index can group by, so "how
        // many runs fenced last week" does not require parsing every row.
        .bind(outcome_word(outcome))
        .bind(self.trials_spent)
        // What the run produced (§10). The trial count is set above from the
        // catalogue's own declaration, so it is copied here rather than
        // recounted — two counters for one quantity is two answers.
        .bind(i32::try_from(self.yielded.findings_established).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.findings_suggestive).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.findings_refuted).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.negatives).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.negatives_new_class).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.questions_opened).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.questions_closed).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.contradictions_surfaced).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.proposals).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.hypotheses_tested).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.hypotheses_refuted).unwrap_or(i32::MAX))
        .bind(i32::try_from(self.yielded.hypotheses_supported).unwrap_or(i32::MAX))
        .bind(barren)
        .execute(&self.pg)
        .await;
    }

    /// Whether this negative result's class is new for its subject.
    ///
    /// A question about the store, which is why it is here and not in the
    /// sans-IO `Yield`. A read that fails answers `false` — the conservative
    /// direction, because the bonus is for closing a door nobody had closed, and
    /// an outage is not evidence that nobody had.
    async fn negative_class_is_new(&self, entry: &harness::memory::Entry) -> bool {
        let Some(class) = entry.extra.get("failure_class").and_then(|v| v.as_str()) else {
            return false;
        };
        let seen: Result<Option<i64>, _> = sqlx::query_scalar(
            "SELECT 1 FROM agent_memory.entry
              WHERE kind = 'negative_result' AND subject = $1
                AND extra->>'failure_class' = $2 AND id <> $3 LIMIT 1",
        )
        .bind(&entry.subject)
        .bind(class)
        .bind(&entry.id)
        .fetch_optional(&self.pg)
        .await;
        matches!(seen, Ok(None))
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

/// What memory has for this run, and a record of having shown it
/// (AGENT-007 §4; Set S task S-4.5).
///
/// Assembled here rather than in the loop because retrieving it reads Postgres
/// and the loop is sans-IO. Returns `None` when the store holds nothing for this
/// user — a run with no memory is the ordinary case, and an empty `Reference`
/// section would spend budget saying so.
///
/// # Failure is a warning, never a refusal
///
/// A run that cannot reach its memory has lost an advantage, not its task.
/// Failing the run instead would make the memory store a new way for research to
/// stop, which is a worse trade than a run repeating a question it could have
/// looked up.
async fn load_brief(
    pg: &PgPool,
    user_id: Uuid,
    run_id: Uuid,
    goal: &str,
    profile: &Profile,
) -> Option<harness::memory::Brief> {
    let store = crate::memory_store::MemoryStore::new(pg.clone());

    // Every live entry of the kinds a brief draws on. The cap is the store's,
    // not the brief's: `assemble_brief` decides what fits, and handing it a
    // pre-truncated set would put the truncation before the ranking rather than
    // after it.
    let mut entries = Vec::new();
    for kind in [
        harness::memory::Kind::UserPreference,
        harness::memory::Kind::InstrumentProfile,
        harness::memory::Kind::Finding,
        harness::memory::Kind::NegativeResult,
        harness::memory::Kind::OpenQuestion,
    ] {
        match store.by_kind(user_id, kind, 200).await {
            Ok(mut found) => entries.append(&mut found),
            Err(e) => {
                tracing::warn!(error = %e, kind = kind.as_str(), "memory unreadable for this run");
                return None;
            }
        }
    }
    if entries.is_empty() {
        return None;
    }

    // The 20% is `Section::Reference`'s own share in `context.rs`, applied to
    // the budget the assembler actually works from — not the advertised window,
    // which is larger and would have the brief claim room the body needs.
    //
    // Frontier tiers get each entry's structured detail; everything else gets
    // one line. A small tier spending its Reference budget on detail is a small
    // tier with less room left for the task.
    let detail = if profile.tier.is_local() {
        harness::memory::BriefDetail::Lines
    } else {
        harness::memory::BriefDetail::Full
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let reference_budget = (profile.context.effective_budget_tokens as f64 * 0.20) as u32;

    let brief = harness::memory::assemble_brief(
        &run_id.to_string(),
        goal,
        &entries,
        reference_budget,
        detail,
        &chrono::Utc::now().date_naive().to_string(),
        &[],
        &[],
    );

    // Logged before the run reads it, so the denominator exists whatever the run
    // then does. A log written at the end would miss every run that crashed, and
    // those are not runs where memory worked.
    let retrievals = crate::retrieval_store::RetrievalStore::new(pg.clone());
    if let Err(e) = retrievals.log(&brief.retrievals).await {
        tracing::warn!(error = %e, "the retrieval log did not write; recall is unmeasured here");
    }
    Some(brief)
}

// ── The pump ────────────────────────────────────────────────────────────────

/// Runs one local-tier task to a terminal state.
#[allow(clippy::too_many_lines)]
pub async fn run(pg: PgPool, client: LlmClient, params: LocalRunParams, cancel: Arc<AtomicBool>) {
    // Every platform tool call this session makes is recorded before it is
    // authorised and after it is answered (SPEC §15, ADR-P2-21, ADR-P2-34) —
    // by `dispatch_tool`, through the platform's own audit endpoint, which is
    // the same trail the external MCP front door now writes. The principal is
    // derived from the session the API authenticated, so "agent X acting for
    // user Y" is a fact the agent has no field to write.
    let mut rec = Recorder {
        pg: pg.clone(),
        run_id: params.run_id,
        seq: 0,
        tokens_in: 0,
        tokens_out: 0,
        event_seq: 0,
        prompt_tokens: 0,
        est_prompt_tokens: 0,
        cache_read_tokens: 0,
        trials_spent: 0,
        exposure_out: params.exposure_out.clone(),
        yielded: harness::lifecycle::r#yield::Yield::default(),
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
    // Which adapter served this run. Two runs on different adapters are two
    // different experiments, and an eval that averages them is measuring the
    // mixture rather than either (§4, S-2.8).
    let adapter_name = harness::adapter::for_profile(&params.profile)
        .provider()
        .to_string();
    rec.admitted(&params.profile, &description, &adapter_name)
        .await;
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
                Effect::Remember(e) => rec.remember(params.user_id, &e).await,
                Effect::Done(o) => rec.finish(&o).await,
                _ => {}
                // A refusal has no workspace and no records, so it cannot spill.
            }
        }
        return;
    }

    // A third reason a task may not run here, beside the two above: the tier
    // does not offer this workflow (AGENT-013 §11; S-10.11, S-10.14).
    //
    // Refused rather than downgraded. A campaign quietly demoted to an
    // `investigate` would still finish, still write a verdict, and still be read
    // as the answer to the question that was asked — and the one place the
    // substitution would be visible is a `workflow` column nobody compares to
    // the request. The other two admission refusals are stated before anything
    // is spent, and so is this one.
    if !params.profile.may_run(params.workflow) {
        let offered: Vec<&str> = params
            .profile
            .workflows
            .iter()
            .map(|w| w.as_str())
            .collect();
        rec.finish(&Outcome::Refused {
            reason: format!(
                "the `{}` profile does not offer the `{}` workflow (AGENT-013 §11); it offers {}",
                params.profile.model_id,
                params.workflow.as_str(),
                offered.join(", ")
            ),
            fix: params.profile.escalate_to.as_ref().map_or_else(
                || "choose a workflow this tier offers, or configure a tier that offers this one".to_string(),
                |to| format!("run this on `{to}`, which is where this profile escalates"),
            ),
        })
        .await;
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
    // A Chat turn spends no trials (§2.2). The exposure filter already means the
    // model cannot name such a tool; this is the refusal at the door, which is
    // the mechanism that survives a later code path routing around the filter.
    //
    // And a look needs a commitment behind it (§7.2): the run's sealed
    // registration decides whether the trial gate admits anything at all. Read
    // here, once, because `McpContext` is one per run and so is the gate.
    let mcp_ctx = {
        let ctx = McpContext::new(ApiClient::new(
            params.api_base.clone(),
            params.service_token.clone(),
        ))
        .under_preregistration(sealed_registration(&pg, params.run_id, params.demand).await);
        // One question with one answer (§2.2, §3). A chat turn spends nothing;
        // so does a `survey`, for a different reason and with a different thing
        // to tell the model.
        ctx.spending(if params.demand == TaskDemand::SingleLookup {
            mcp_server_lib::gate::Spending::ChatTurn
        } else if params.workflow == harness::state::Workflow::Survey {
            mcp_server_lib::gate::Spending::Survey
        } else {
            mcp_server_lib::gate::Spending::Allowed
        })
    };

    let mut governor = Loop::new(
        params.profile.clone(),
        registry,
        Task {
            // One builder for both drivers (S-1.6). This path had its own copy of
            // the prior-turn framing and the hosted one had none at all, so the two
            // front doors disagreed about what the model was told: the local agent
            // remembered the conversation and the hosted agent did not.
            goal: prompt::opening_message(
                Some(params.prior.as_str()),
                &params.constraints,
                &params.goal,
            ),
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
    .with_gated_actions(ledger::audit::gated_actions())
    // The previous turn's tool slice, so this turn appends to it rather than
    // re-choosing an order (§21; S-9.9). Empty on the first turn of a
    // conversation, which is the one turn that should pay a cold prefix.
    .with_prior_exposure(params.prior_exposure.clone())
    // Which catalogue tools spend a trial, for §20's step value. The catalogue
    // owns the declaration and the loop is sans-IO, so it is told.
    .with_trial_spending_tools(trial_spending_tools())
    // Which tools reach paper, for the ring filter (§4; S-10.1). Derived from
    // the catalogue, so the filter cannot disagree with the gate about what
    // reaches capital.
    .with_paper_tools(paper_reaching_tools())
    // The stage skeleton (§3). Set before the first render, so the exposure
    // filter sees it on step one rather than from step two.
    .with_workflow(params.workflow);

    // What memory has to say about this goal, as `Reference`.
    if let Some(brief) = load_brief(
        &pg,
        params.user_id,
        params.run_id,
        &params.goal,
        &params.profile,
    )
    .await
    {
        governor = governor.with_brief(brief);
    }

    // One transcript per native run (S-2.2). `None` on `select_fill`, where every
    // call re-renders the body and nothing accumulates provider-side.
    let mut transcript = (!params.profile.output.constrained_decoding).then(|| {
        harness::transcript::Transcript::new(prompt::opening_message(
            Some(params.prior.as_str()),
            &params.constraints,
            &params.goal,
        ))
    });

    rec.input("Admission", serde_json::json!({ "admission": format!("{admission:?}") }))
        .await;
    let mut effects = governor.next(Input::Admission(admission));
    loop {
        // **Before anything is executed.** The log's whole value is that a row says
        // what was about to happen rather than what happened: a crash between the
        // decision and the act is then distinguishable from a crash after it, and
        // that distinction is what tells a resume whether running the effect again
        // would write twice (S-2.5, and S-2.6 reads it).
        rec.effects(&effects).await;

        let mut blocking: Option<Effect> = None;
        for e in effects {
            match e {
                Effect::Note(n) => rec.note(&n).await,
                Effect::Remember(e) => {
                    // The yield's counts come from what the run actually wrote,
                    // not from what it said it wrote (§10).
                    match e.kind {
                        harness::memory::Kind::NegativeResult => {
                            // Whether the class is new for the subject is a
                            // question about the store, so the store answers it.
                            let is_new = rec.negative_class_is_new(&e).await;
                            rec.yielded.record_negative(is_new);
                        }
                        harness::memory::Kind::OpenQuestion => {
                            rec.yielded.questions_opened += 1;
                        }
                        _ => rec.yielded.record_finding(e.confidence),
                    }
                    rec.remember(params.user_id, &e).await;
                }
                // The bytes leave memory and land in the agent's own folder,
                // where `expand` and `read_file` both reach them (§26.3). A
                // failed write is logged and the body stays where it is:
                // losing an observation to save memory would invert the whole
                // point of a store that never deletes.
                Effect::Spill { record, handle, body } => {
                    let text = serde_json::to_string(&*body).unwrap_or_default();
                    let args = serde_json::Map::from_iter([
                        ("path".to_string(), json!(handle)),
                        ("content".to_string(), json!(text)),
                    ]);
                    // The same `write_file` the agent's own tools use, so a
                    // spilled body lands where `read_file` can reach it and
                    // under the same path rules — a second write path would be
                    // a second set of rules about where the agent may write.
                    let (detail, failed) =
                        workspace_tools::execute(&ws, "write_file", &args).await;
                    if failed {
                        // The body goes back. Losing an observation to save
                        // memory would invert the point of a store that never
                        // deletes, and the record would then say "expand to
                        // read it" about a file that does not exist.
                        governor.restore_spilled(record, *body);
                        tracing::warn!(
                            run_id = %params.run_id, %handle, %detail,
                            "context spill failed; the record keeps its body"
                        );
                    }
                }
                Effect::Done(o) => {
                    // Before `finish`, because `finish` stamps the run complete
                    // and a reader that saw the row finish without a summary
                    // would take the prose path for a turn that has a real one.
                    rec.write_turn_summary(params.conversation_id, &governor, &o)
                        .await;
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
            rec.input("Cancel", serde_json::json!({})).await;
            effects = governor.next(Input::Cancel);
            continue;
        }

        let input = match blocking {
            // Drained with the notes above, never blocking: storing a finding is
            // something the platform does *because* of a decision, not something
            // the loop waits on. Reaching here would mean the drain missed it.
            Effect::Remember(_) => unreachable!("Remember is handled with the notes"),
            Effect::Spill { .. } => unreachable!("Spill is handled with the notes"),
            Effect::CallModel(call) => {
                let started = Instant::now();
                let req = build_request(&params.profile, &call, transcript.as_ref());

                // The same refusal the proxy makes, applied to the internal driver
                // too (S-2.7). With S-1.10 and S-1.11 in place this should never
                // fire — the assembler budgets the body against the window and
                // `ask` sends what it budgeted — so reaching it means the
                // arithmetic has drifted from what is actually sent.
                //
                // **It is therefore a regression alarm rather than a safety net**,
                // and it refuses rather than truncating for exactly that reason: a
                // silent truncation here would hide the drift and take the charter
                // with it, which is the failure the whole budget exists to prevent.
                let estimated = crate::llm_proxy::estimate_request_tokens(
                    &serde_json::to_value(&req).unwrap_or_else(|_| json!({})),
                );
                let cap = params.profile.input_budget_tokens();
                if estimated > cap {
                    tracing::error!(
                        run_id = %params.run_id, estimated, cap,
                        "an internal request exceeded the context budget; the assembler's                          arithmetic and what is sent have drifted apart"
                    );
                    rec.finish(&Outcome::Fenced(Degradation::BackendFailure {
                        detail: format!(
                            "context_budget_exceeded: about {estimated} input tokens against                              a budget of {cap}. The context assembler and the request builder                              disagree; this is a harness defect, not a model one."
                        ),
                    }))
                    .await;
                    return;
                }
                match client.chat(&req).await {
                    Ok(resp) => {
                        // The harness's own estimate of the request it just sent,
                        // by the same arithmetic the assembler budgeted with. Taken
                        // here rather than in the recorder because this is the only
                        // place that still holds the request (§4, S-2.8).
                        let estimated = harness::context::estimate_tokens(&call.system)
                            + harness::context::estimate_tokens(&call.prompt);
                        rec.usage(
                            call.step,
                            &resp.usage,
                            estimated,
                            started.elapsed().as_millis(),
                        )
                        .await;
                        // The conversation grows here, and is brought back inside
                        // the window at the same moment (S-2.13). Elision at a step
                        // boundary rather than per call: moving a cache breakpoint
                        // invalidates everything after it, so doing it once per step
                        // costs one re-read and doing it per call costs the cache.
                        if let Some(t) = transcript.as_mut() {
                            t.push_assistant(
                                resp.content.clone(),
                                resp.tool_calls
                                    .iter()
                                    .map(|c| harness::transcript::CallRef {
                                        id: c.id.clone(),
                                        name: c.name.clone(),
                                        arguments: c.arguments.to_string(),
                                    })
                                    .collect(),
                            );
                            let report = t.compact(&params.profile, 0);
                            if report.truncated > 0 {
                                rec.append(
                                    "compaction",
                                    json!({
                                        "transport": "native",
                                        "truncated": report.truncated,
                                        "tokens_before": report.tokens_before,
                                        "tokens_after": report.tokens_after,
                                    }),
                                )
                                .await;
                            }
                        }
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
                // The catalogue says whether this call registers a trial. A tool
                // with no spec is not a catalogue tool — `fs` and the harness-owned
                // ones — and spends nothing.
                if mcp_server_lib::spec::find(&name).is_some_and(|spec| {
                    spec.spends_trial != mcp_server_lib::spec::TrialKind::None
                }) {
                    rec.trials_spent += 1;
                }
                // What the model asked for, before the platform completes it.
                // The brief the driver attaches below is megabytes of the run's
                // own evidence and is recorded on the campaign row, not here.
                rec.append(
                    "tool_call",
                    json!({ "step": step, "name": name, "risk": risk.as_str(),
                            "arguments": arguments }),
                )
                .await;
                // `define_campaign` is the one call whose arguments the platform
                // completes (AGENT-013 §8.5, AGENT-005 §29; S-10.11).
                //
                // A campaign's HYPOTHESIZE is a worker sub-run reading a
                // *projection* of this run's context store, and the model cannot
                // supply that: it would be asking the thing whose reasoning is
                // being withheld to hand over the set with its reasoning left
                // out. The projection is derived here, from the store, by kind —
                // so what the worker sees is decided by
                // `projection::campaign_worker` and by nothing the model wrote.
                //
                // `run_id` travels the same way and for the same reason the
                // registration's does: a body-supplied run id would let one run
                // spend another's budget.
                // `preregister` on a tier that does not author freely is handed
                // the skeleton §11 says it should be (S-10.3). The platform
                // fills what the question already fixed and names the three
                // fields that are the run's own claim; a tier asked for eleven
                // fields from prose produces something that validates and
                // cannot refute anything.
                let arguments = if name == "preregister" {
                    match harness::lifecycle::skeleton::for_run(
                        &params.profile,
                        &params.constraints,
                        // No cost-model registry exists to name a version from,
                        // so the skeleton names the hole rather than inventing
                        // a version nobody published.
                        None,
                        DEFAULT_GATE_PROFILE,
                        // No profile declares a per-tier trial cap, so this is
                        // the second hole the skeleton names rather than fills.
                        None,
                    ) {
                        None => arguments,
                        Some(skeleton) => {
                            let mut filled = arguments.clone();
                            let still_yours: Vec<&str> = skeleton
                                .what_you_must_write()
                                .iter()
                                .map(|(f, _)| *f)
                                .collect();
                            // The model's own values win. A skeleton that
                            // overwrote them would make "confirm this" mean
                            // "your answer was discarded", and the run would be
                            // held to a protocol it did not write.
                            for (k, v) in skeleton.filled {
                                filled.entry(k).or_insert(v);
                            }
                            rec.append(
                                "status",
                                json!({
                                    "prereg_authoring": params.profile.prereg_authoring.as_str(),
                                    "skeleton_filled": filled.len() - arguments.len(),
                                    "still_yours": still_yours,
                                }),
                            )
                            .await;
                            filled
                        }
                    }
                } else if name == "define_campaign" {
                    let mut with_brief = arguments.clone();
                    with_brief.insert("run_id".into(), json!(params.run_id));
                    let query = harness::context::select::Query::from_state(
                        governor.working(),
                        step,
                    );
                    let brief = subrun::assemble(
                        governor.context_store(),
                        &query,
                        subrun::brief_budget(&params.profile),
                        governor.context_policy(),
                        subrun::BRIEF_MANIFEST_TOKENS,
                    );
                    with_brief.insert(
                        "brief".into(),
                        serde_json::to_value(subrun::PinnedBrief::of(&brief))
                            .unwrap_or(Value::Null),
                    );
                    rec.append(
                        "status",
                        json!({
                            "campaign_projection": brief.projection,
                            "shown": brief.shown.len(),
                            "withheld": brief.withheld.len(),
                        }),
                    )
                    .await;
                    with_brief
                } else {
                    arguments
                };

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
                    bridge::execute(&mcp_ctx, &name, &arguments).await
                };
                rec.append(
                    "tool_result",
                    json!({ "name": name, "is_error": out.is_error, "content": out.raw }),
                )
                .await;
                // Back into the conversation, paired with the call that asked for it
                // (S-2.2). Most providers reject a tool result whose `tool_call_id`
                // matches no call in the thread, so the id comes from the assistant
                // turn appended above rather than being invented here.
                if let Some(t) = transcript.as_mut() {
                    let call_id = t
                        .turns()
                        .iter()
                        .rev()
                        .find_map(|turn| match turn {
                            harness::transcript::Turn::Assistant { calls, .. } => calls
                                .iter()
                                .find(|c| c.name == name)
                                .map(|c| c.id.clone()),
                            harness::transcript::Turn::ToolResult { .. } => None,
                        })
                        .unwrap_or_else(|| format!("call_{step}"));
                    t.push_result(call_id, &name, &out.text, out.is_error, None);
                }
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

            // AGENT-010 §8. The loop suspends on a platform job and the driver
            // is meant to persist the suspension and exit — it must not poll,
            // which is the bug this effect exists to remove.
            //
            // **This driver cannot yet resume from a suspension.** The harness
            // produces the effect and `crates/harness` persists it; nothing on
            // the local-model path reads it back. So the run ends here, visibly
            // and with the job named, rather than being reported as finished or
            // being turned back into the polling loop it replaced. Recorded as
            // an open gap rather than papered over: see Set S phase 2's task
            // list. A `unreachable!` here would be false — a tool returning a
            // job handle reaches it.
            Effect::Await { step, job_id, tool } => {
                tracing::warn!(
                    run_id = %params.run_id, step, %job_id, %tool,
                    "suspended on a platform job; this driver cannot resume a suspension yet"
                );
                rec.finish(&Outcome::Fenced(Degradation::BackendFailure {
                    detail: format!(
                        "suspended at step {step} awaiting job {job_id} started by `{tool}`.                          This driver cannot resume a suspension yet, so the run stops here                          rather than polling. The job itself continues and its result is in                          the job service."
                    ),
                }))
                .await;
                return;
            }

            Effect::Note(_) | Effect::Done(_) => unreachable!("handled above"),
        };

        // And the answer, before it is fed back. An input recorded after the loop
        // has consumed it cannot tell a replay whether the loop had seen it.
        let (kind, payload) = describe_input(&input);
        rec.input(kind, payload).await;
        effects = governor.next(input);
    }
}

/// Builds one provider request from one [`harness::drive::ModelCall`].
///
/// The mapping is mechanical on purpose: every value here was decided by the loop,
/// and a driver that second-guessed one of them would be enforcing a policy in the
/// place the design put outside the tested boundary.
/// The request for one call, on whichever transport the profile decodes with
/// (AGENT-010 §3, S-2.2).
///
/// The two are genuinely different shapes and the adapter owns the difference:
///
/// * **`select_fill`** is single-turn. Every call carries the body the harness just
///   assembled, and nothing accumulates provider-side — which is right for a tier
///   with no prompt cache and a window measured in thousands.
/// * **`native`** is a conversation. The messages are the run's transcript, appended
///   to and elided by `harness::transcript`, because re-rendering a body each call
///   would throw away the cache the S-1.8 breakpoints exist to earn: a prefix is only
///   reusable if its bytes do not change.
///
/// `transcript` is `None` on the `select_fill` path, and the `call.prompt` fallback
/// covers the first native call, before the conversation has a turn in it.
fn build_request(
    profile: &Profile,
    call: &harness::drive::ModelCall,
    transcript: Option<&harness::transcript::Transcript>,
) -> ChatRequest {
    let (schema, tools) = match &call.decode {
        Decode::Native { tools } => (None, native_tools(tools)),
        other => (other.schema().cloned(), Vec::new()),
    };
    let messages = match transcript.filter(|t| matches!(call.decode, Decode::Native { .. })) {
        Some(t) => transcript_messages(t),
        None => vec![Message::User {
            content: call.prompt.clone(),
        }],
    };
    ChatRequest {
        model: profile.model_id.clone(),
        system: Some(call.system.clone()),
        messages,
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

/// The transcript, as this provider's wire format.
///
/// The mapping lives here rather than in `crates/harness` because the harness is
/// sans-IO: knowing about a wire format is knowing about IO, and the whole reason
/// the transcript is provider-neutral is so that adding a provider is an impl here
/// rather than a change there.
fn transcript_messages(t: &harness::transcript::Transcript) -> Vec<Message> {
    use harness::transcript::Turn;

    let mut out = Vec::with_capacity(t.len() + 1);
    out.push(Message::User {
        content: t.opening().to_string(),
    });
    for turn in t.turns() {
        match turn {
            Turn::Assistant { text, calls } => out.push(Message::Assistant {
                content: text.clone(),
                tool_calls: calls
                    .iter()
                    .map(|c| llm::ToolCall {
                        id: c.id.clone(),
                        name: c.name.clone(),
                        arguments: serde_json::from_str(&c.arguments)
                            .unwrap_or_else(|_| json!({})),
                    })
                    .collect(),
            }),
            Turn::ToolResult {
                call_id,
                name,
                body,
                is_error,
                ..
            } => out.push(Message::ToolResult {
                tool_call_id: call_id.clone(),
                name: name.clone(),
                content: body.clone(),
                is_error: *is_error,
            }),
        }
    }
    out
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
        let req = build_request(&p, &call, None);
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
            assert_eq!(build_request(&p, &call, None).num_ctx, Some(24_000));
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
                // The cache counters default to zero: this fixture is about
                // decoding a reply, not about what it cost.
                ..llm::Usage::default()
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
                // The cache counters default to zero: this fixture is about
                // decoding a reply, not about what it cost.
                ..llm::Usage::default()
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
