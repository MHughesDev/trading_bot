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

use super::{bridge, hardware_probe, prompt, skills_mount, subrun, workspace_tools};

/// How long a local backend should hold the weights resident between steps.
///
/// Measured on the dev box: without it a cold load costs ~100 s, and a loop that
/// takes 15 steps pays it 15 times. Not a performance tweak — a 25-minute tax on
/// every session.
const KEEP_ALIVE: &str = "30m";

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
    /// The working state a lower tier escalated with, when this run continues
    /// one (AGENT-008 §4; S-2.12). `None` for an ordinary turn.
    pub resumed_state: Option<harness::state::WorkingState>,
    /// Stop after this many trials, with one report turn (a session child's lease;
    /// S-11.6). `None` is no cap.
    pub max_trials: Option<u32>,
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
        Outcome::Suspended { .. } => "suspended",
        // Its own word, for the same reason `escalated` is not folded into
        // `refused`: one is the run declining and the other is the run asking
        // whether to spend. A rate that counted offers as refusals would read a
        // working door as a failure.
        Outcome::Offered(_) => "offered",
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
        Input::Admission(a) => (
            "Admission",
            serde_json::json!({ "admission": format!("{a:?}") }),
        ),
        Input::ModelReply { raw } => (
            "ModelReply",
            // The reply in full. It is what the loop parsed, and a replay that
            // re-parses something else is not a replay.
            serde_json::json!({ "raw": raw }),
        ),
        Input::ModelTruncated { raw } => ("ModelTruncated", serde_json::json!({ "raw": raw })),
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
        // Recorded field by field rather than as a debug string, because resume
        // reads them back (S-2.6, S-2.11, S-2.14).
        Input::Steer { text } => ("Steer", serde_json::json!({ "text": text })),
        Input::Pin { tool } => ("Pin", serde_json::json!({ "tool": tool })),
        Input::JobAccepted { job_id } => ("JobAccepted", serde_json::json!({ "job_id": job_id })),
        Input::JobDone {
            job_id,
            output,
            source,
            ok,
        } => (
            "JobDone",
            serde_json::json!({ "job_id": job_id, "output": output, "source": source, "ok": ok }),
        ),
        #[allow(unreachable_patterns)]
        other => (
            // A variant added after this was written. Named by its debug form rather
            // than silently dropped: a log missing a row is a log that cannot be
            // replayed, and the failure should be visible in the row itself.
            "Other",
            serde_json::json!({ "debug": format!("{other:?}") }),
        ),
    }
}

/// The job a long tool's answer says is still running, as the watcher names it
/// (`sweep:`, `backtest:`, `training:`, `study:`), or `None` when the call
/// finished and its answer is the result (S-2.11).
#[must_use]
pub fn long_job_handle(name: &str, arguments: &Map<String, Value>, raw: &Value) -> Option<String> {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_string);
    match name {
        "run_sweep" if raw.get("timed_out").and_then(Value::as_bool) == Some(true) => {
            s(raw, "sweep_id").map(|id| format!("sweep:{id}"))
        }
        "wait_for_backtest" if raw.get("timed_out").and_then(Value::as_bool) == Some(true) => {
            arguments
                .get("backtest_id")
                .and_then(Value::as_str)
                .map(|id| format!("backtest:{id}"))
        }
        "train_model" if raw.get("status").and_then(Value::as_str) == Some("still_running") => {
            Some(format!("training:{}:{}", s(raw, "model_id")?, s(raw, "run_id")?))
        }
        "run_study" if raw.get("detached").and_then(Value::as_bool) == Some(true) => {
            Some(format!("study:{}:{}", s(raw, "experiment_id")?, s(raw, "study_id")?))
        }
        _ => None,
    }
}

/// The job handle a `define_campaign` result suspends its run on, if it is one.
///
/// `campaign:<id>`, which the watcher resolves against
/// `GET /api/lifecycle/campaigns/{id}`. An error result, or one with no
/// campaign id, is an ordinary result: nothing was started, so there is nothing
/// to wait for.
#[must_use]
pub fn campaign_handle(name: &str, is_error: bool, raw: &Value) -> Option<String> {
    if name != "define_campaign" || is_error {
        return None;
    }
    raw.get("campaign_id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(|id| format!("campaign:{id}"))
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
        Effect::AskHuman {
            step, tool, ruling, ..
        } => (
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
        Effect::Spill {
            record,
            handle,
            body,
        } => (
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
        Effect::Note(Note::CoreTool { name, detail, args }) => (
            "Note",
            // The arguments too: a core tool is never dispatched, so this row is
            // the only place a replay can read what it was called with (S-4.8).
            serde_json::json!({ "core_tool": name, "detail": detail, "args": args }),
        ),
        Effect::Note(n) => ("Note", serde_json::json!({ "note": format!("{n:?}") })),
        Effect::Done(o) => ("Done", serde_json::json!({ "outcome": format!("{o:?}") })),
    }
}

// ── Persistence ─────────────────────────────────────────────────────────────

struct Recorder {
    pg: PgPool,
    run_id: Uuid,
    /// The tenant, for the ledger reads at run end. `mlops` is tenant-scoped and
    /// a verdict assembled without one would read another tenant's tickets.
    user_id: Uuid,
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
    /// Why the step boundary ended this run, when it was the boundary that did
    /// (ADR-D-27).
    ///
    /// The governor has one word for "stop" — `Input::Cancel` — and it should
    /// keep having one: a second exit would be a second way for a run to end
    /// that the loop's own accounting never saw. But `Outcome::Cancelled` then
    /// arrives at [`Self::finish`] carrying no error, so an expired pause and an
    /// operator's Stop would write byte-identical rows, and the one question an
    /// operator asks about a run they left paused — *did I lose it to the
    /// deadline?* — would have no answer anywhere in `agent_runs`.
    ///
    /// So the reason is carried beside the governor rather than through it, and
    /// only to decide the `error` text. It changes no status and no branch.
    stop_reason: Option<super::control::StopReason>,
    /// The offer this run ended on was not left for the user to accept: they have
    /// a standing "always start" preference for this scope (S-10.2), so a Research
    /// run is being started for them. The card says so rather than showing a
    /// Start button for something already started.
    offer_auto_started: bool,
    /// What this run's brief showed it (S-6.10). A finding whose evidence names
    /// one of these ids is recorded as a citation in `memory_retrievals`, which is
    /// what the desk's Memory tab reads as "cited".
    retrieved: Vec<harness::memory::Retrieval>,
    /// This run has read untrusted text; what it records is tainted (KM-10).
    consumed_untrusted: bool,
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
    fn new(
        pg: PgPool,
        run_id: Uuid,
        user_id: Uuid,
        exposure_out: Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            pg,
            run_id,
            user_id,
            seq: 0,
            tokens_in: 0,
            tokens_out: 0,
            event_seq: 0,
            prompt_tokens: 0,
            est_prompt_tokens: 0,
            cache_read_tokens: 0,
            trials_spent: 0,
            exposure_out,
            yielded: harness::lifecycle::r#yield::Yield::default(),
            stop_reason: None,
            offer_auto_started: false,
            retrieved: Vec::new(),
            consumed_untrusted: false,
        }
    }

    /// Record which retrieved entries this call's evidence names (S-6.10).
    async fn record_citations(&self, tool: &str, args: &Value) {
        crate::retrieval_store::record_citations(&self.pg, self.run_id, &self.retrieved, tool, args).await;
    }

    /// Pick the counters back up after a restart (S-2.6).
    ///
    /// Every number here is re-derived from what was written, never assumed to
    /// start at zero: a resumed run that restarted its trial count would report
    /// having searched less than it did, which is the direction that flatters.
    /// Both sequences continue from the highest row already written, so the
    /// resumed run's rows follow the interrupted ones rather than colliding.
    async fn restore(&mut self, rows: &[super::resume::LogRow]) {
        self.event_seq = rows.iter().map(|r| r.seq).max().unwrap_or(0);
        self.seq = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), 0) FROM agent_messages WHERE run_id = $1",
        )
        .bind(self.run_id)
        .fetch_one(&self.pg)
        .await
        .unwrap_or(0);
        if let Ok(Some((tin, tout))) = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
            "SELECT tokens_in, tokens_out FROM agent_runs WHERE run_id = $1",
        )
        .bind(self.run_id)
        .fetch_optional(&self.pg)
        .await
        {
            self.tokens_in = tin.unwrap_or(0);
            self.tokens_out = tout.unwrap_or(0);
        }
        // The prompt-size pair is per call, in the usage rows.
        let sums: Option<(Option<i64>, Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT SUM((content_json->>'prompt_tokens')::bigint),
                    SUM((content_json->>'est_prompt_tokens')::bigint),
                    SUM((content_json->>'cache_read_tokens')::bigint)
               FROM agent_messages WHERE run_id = $1 AND kind = 'usage'",
        )
        .bind(self.run_id)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten();
        if let Some((p, e, c)) = sums {
            self.prompt_tokens = p.unwrap_or(0);
            self.est_prompt_tokens = e.unwrap_or(0);
            self.cache_read_tokens = c.unwrap_or(0);
        }
        // Trials and findings from the log's own effect rows, by the same
        // declarations the live counters read.
        for r in rows.iter().filter(|r| r.direction == "effect") {
            match r.kind.as_str() {
                "ExecuteTool" => {
                    let name = r.payload.get("name").and_then(Value::as_str).unwrap_or("");
                    if mcp_server_lib::spec::find(name).is_some_and(|spec| {
                        spec.spends_trial != mcp_server_lib::spec::TrialKind::None
                    }) {
                        self.trials_spent += 1;
                    }
                }
                "Remember" => {
                    let kind = r.payload.get("kind").and_then(Value::as_str).unwrap_or("");
                    let confidence = r
                        .payload
                        .get("confidence")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if kind == harness::memory::Kind::NegativeResult.as_str() {
                        // Whether its class was new was decided when it was
                        // written; not re-decided against a store that now
                        // contains it.
                        self.yielded.record_negative(false);
                    } else if kind == harness::memory::Kind::OpenQuestion.as_str() {
                        self.yielded.questions_opened += 1;
                    } else {
                        for c in [
                            harness::memory::Confidence::Established,
                            harness::memory::Confidence::Suggestive,
                            harness::memory::Confidence::Refuted,
                        ] {
                            if c.as_str() == confidence {
                                self.yielded.record_finding(c);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

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
        if let Note::CoreTool { name, args, .. } = n {
            self.record_citations(name, args).await;
        }
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
                "layers": layer_tokens(call),
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
        usd_tenth_cents: Option<i64>,
        layers: Value,
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
        self.cache_read_tokens += i64::try_from(usage.cache_read_input_tokens).unwrap_or(i64::MAX);
        self.append("usage", usage_note(step, usage, estimated, latency_ms, usd_tenth_cents, layers))
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
        if self.consumed_untrusted && !entry.is_tainted() {
            entry = entry.mark_tainted();
        }
        let store = crate::memory_store::MemoryStore::new(self.pg.clone());
        if let Err(e) = store.put(user_id, &entry).await {
            tracing::warn!(
                run_id = %self.run_id,
                entry = %entry.id,
                error = %e,
                "the finding did not persist; it is still in this run's state and summary"
            );
            // Nothing was written, so nothing it cited was answered. Closing a
            // question on the strength of a finding that did not persist would
            // leave the question closed by a row nobody can read.
            return;
        }
        // A finding that cites an open question answers it (AGENT-013 §9;
        // S-10.12). The platform's move, from a fact about what was written
        // down — `close_cited` has had no caller, so a question opened in one
        // run stayed open forever and `questions_closed` has been zero on every
        // run ever recorded.
        match crate::questions::close_cited(&self.pg, user_id, &entry).await {
            Ok(closed) if !closed.is_empty() => {
                self.yielded.questions_closed += u32::try_from(closed.len()).unwrap_or(0);
                self.append(
                    "questions_closed",
                    json!({ "by": entry.id, "closed": closed }),
                )
                .await;
            }
            Ok(_) => {}
            Err(e) => {
                // Logged, never fatal. The finding is stored; a question that
                // stays open is retrieved again, which costs a step. Failing the
                // run over it would cost the finding.
                tracing::warn!(
                    run_id = %self.run_id,
                    entry = %entry.id,
                    error = %e,
                    "could not close the questions this finding cites"
                );
            }
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
            Outcome::Suspended { .. } => "suspended",
            Outcome::Offered(_) => "offered",
            Outcome::Cancelled => "cancelled",
        };

        // The turn this run is. Read rather than passed, so no call site has to
        // carry it and none can pass a stale one.
        let turn: i32 =
            sqlx::query_scalar("SELECT COALESCE(turn_index, 0) FROM agent_runs WHERE run_id = $1")
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
        // The platform decides every hypothesis this run sealed, from the
        // ledger's own rows (AGENT-013 §6.2; S-10.4).
        //
        // Here rather than at a happy-path exit, because `finish` is the one
        // place every ending passes through — and a fenced run's hypotheses are
        // no less decidable than a finished one's. A run that died holding a
        // refutation should still record it.
        //
        // Before the row is written, so the counts below are the verdicts and
        // not the model's account of them. The three `hypotheses_*` fields on
        // `Yield` were declared with S-10.13 and set by nothing: they have been
        // zero on every run ever recorded, which made "how many hypotheses has
        // this platform actually settled" unanswerable from the one table built
        // to answer it.
        self.decide_hypotheses().await;

        // And compile the dossier, from the same rows (S-8.7). Here for the
        // same reason as above — `finish` is the one place every ending passes
        // through — but only for a run that produced a result: a fenced run has
        // no report to check the claims of, and compiling one would be inventing
        // a document about a run that did not finish.
        if let Outcome::Finished {
            result, evidence, ..
        } = outcome
        {
            self.compile_dossier(result, evidence).await;
        }

        // The card that had no producer (S-10.2). `ResearchOffer` is the offer
        // and this is the event the desk turns into it: `Cards.tsx` has rendered
        // an `offer` card with a *Start* action since S-10.15, and nothing on
        // this platform had ever emitted one.
        if let Outcome::Offered(offer) = outcome {
            self.append(
                "offer",
                json!({
                    "question": offer.question,
                    "workflow": offer.workflow.as_str(),
                    "kind": offer.kind,
                    // Absent rather than zero. The desk prints "trial count not
                    // estimated", because a zero is a claim that the run is free.
                    "est_trials": offer.est_trials,
                    "est_usd": offer.est_usd,
                    "auto_started": self.offer_auto_started,
                }),
            )
            .await;
        }

        let (status, summary, error) = match outcome {
            Outcome::Finished {
                result,
                evidence,
                budget_exhausted,
            } => (
                "completed",
                Some(if *budget_exhausted {
                    format!(
                        "{result}\n\nEvidence: {evidence}\n\n(Reported on the last turn a spent \
                         budget allowed.)"
                    )
                } else {
                    format!("{result}\n\nEvidence: {evidence}")
                }),
                None,
            ),
            Outcome::Refused { reason, fix } => ("refused", None, Some(format!("{reason}. {fix}"))),
            Outcome::Escalated { reason, to, .. } => {
                ("refused", None, Some(format!("{reason}; escalate to {to}")))
            }
            Outcome::Fenced(d) => ("fenced", None, Some(fence_message(d))),
            // A suspension is not an error, so it does not write one. The run
            // is waiting on a job that is still running, and the summary says
            // which — a reader who found "error: null" and no explanation would
            // have to guess whether the run had failed.
            Outcome::Suspended { step, job_id, tool } => (
                "suspended",
                Some(format!(
                    "suspended at step {step} waiting on job {job_id}, started by `{tool}`.                      The job continues; its result is in the job service. The run holds no model                      connection and no tokens, and resumes from its effect log when the job ends."
                )),
                None,
            ),
            // The error text is the *only* place an expired pause differs from a
            // Stop. Status stays `cancelled` for both, deliberately: inventing a
            // sixth terminal word would put a value in this column that every
            // existing reader — the sidebar's running dot, `ACTIVE_RUN_STATES`,
            // the restart sweep — would fail to recognise, which is the
            // `ended == "succeeded"` defect rebuilt.
            Outcome::Cancelled => (
                "cancelled",
                None,
                self.stop_reason
                    .and_then(super::control::StopReason::error_text)
                    .map(str::to_owned),
            ),
            // Not an error and not a refusal. The turn ended because the
            // question needs a Research run, and the summary says what would be
            // run rather than what was not.
            Outcome::Offered(offer) => (
                "offered",
                Some(format!(
                    "this needs a research run ({}): {}",
                    offer.workflow.as_str(),
                    offer.question
                )),
                None,
            ),
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
                 hypotheses_supported = $23, barren = $24, n_eff_after = $25,
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
        .bind(self.n_eff_after().await)
        .execute(&self.pg)
        .await;
    }

    /// The tenant's effective trial count as this run ends (§4; S-2.8).
    ///
    /// The other half of the honesty pair: four trials that moved N_eff by four
    /// searched, four that moved it by one searched the same idea four times.
    /// Read from the ledger's own computation — the one the gate worker reads —
    /// and `None` when it cannot be read, because an invented N_eff is worse
    /// than a null one: a null cannot be averaged by accident.
    async fn n_eff_after(&self) -> Option<f64> {
        match ledger::pg::PgTrialLedger::new(self.pg.clone())
            .n_eff_async(&self.user_id.to_string())
            .await
        {
            Ok(n) => Some(n.value()),
            Err(e) => {
                tracing::warn!(run_id = %self.run_id, error = %e, "N_eff unreadable; n_eff_after stays null");
                None
            }
        }
    }

    /// Compile and store this run's dossier (AGENT-006 §5.8; S-8.7).
    ///
    /// The ticket is the run id. A dossier is about one run's report, and a
    /// second identifier would be a second thing to keep in step for no gain —
    /// the proposals queue's `dossier_ref` is this value.
    ///
    /// A failure is recorded in the run's events and does not fail the run. That
    /// is not indifference: a dossier refuses to compile when a claim is not
    /// supported by its evidence, which is a finding about the report and worth
    /// keeping, and the run ended either way.
    async fn compile_dossier(&mut self, result: &str, evidence: &str) {
        let tenant = self.user_id.to_string();
        let run = self.run_id.to_string();
        // The profile the run was judged under, read back rather than carried:
        // a run can be re-tiered mid-flight, and the provenance block has to say
        // which profile the row actually ended on.
        let profile: String = sqlx::query_scalar(
            "SELECT COALESCE(profile_id, 'unknown') FROM agent_runs WHERE run_id = $1",
        )
        .bind(self.run_id)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown".to_string());
        let produced = crate::dossier::Produced {
            run_id: self.run_id,
            tenant_id: &tenant,
            ticket: &run,
            result,
            // What `finish_task` cited, rendered as one piece of evidence under
            // its own name. The ids inside it are resolved from memory and the
            // ledger by the compiler; this is the model's own sentence, kept so
            // a number it quoted from a tool result it did not record is still
            // alignable.
            evidence: vec![harness::report::claims::Evidence {
                id: format!("{run}:cited"),
                text: evidence.to_string(),
            }],
            spend: self.yielded.spend,
            // `self.trials_spent`, **not** `self.yielded.trials_spent`. The
            // yield's copy is assigned thirty lines below this call, so reading
            // it here recorded `0` on every dossier ever compiled — for the one
            // number the platform exists to count. Reading the counter removes
            // the ordering rather than reordering the statements, because an
            // ordering that has to be maintained is one that drifts back.
            trials: u32::try_from(self.trials_spent).unwrap_or(0),
            harness_version: HARNESS_VERSION,
            profile: &profile,
            window: "the run's registered window",
            seeds: "as recorded on each trial",
        };
        // The yield rides on this event, because this is the card a reader
        // opens at the end of a run and the counts were reaching nothing: they
        // are columns on `agent_runs`, and the desk's Report card had a
        // headline and a button. Every count is final by now — the
        // `hypotheses_*` three were set by `decide_hypotheses` just above, and
        // the rest accumulate as the run goes.
        let (key, value) =
            match crate::dossier::compile_and_store(&self.pg, self.user_id, produced).await {
                Ok(_) => ("ticket", json!(run)),
                // Named, not silent. A dossier refuses to compile when a claim is
                // not supported by its evidence or when no critic ran, and both are
                // findings about the run rather than a missing button.
                Err(e) => ("not_compiled", json!(e.to_string())),
            };
        self.append("dossier", self.yield_payload(key, value)).await;
    }

    /// What this run produced, flat, for the Report card (AGENT-013 §10).
    ///
    /// Flat rather than nested under `yield`: the card reads counts by name and
    /// a nested object would make the reader of the card and the writer of the
    /// row disagree about where a number lives.
    ///
    /// `trials_spent` comes from the counter for the same reason the dossier's
    /// does. Absent counts are not emitted as zero anywhere else, but these are
    /// always known at this point, so a zero here means zero.
    fn yield_payload(&self, key: &str, value: Value) -> Value {
        let y = &self.yielded;
        let mut out = json!({
            "findings_established": y.findings_established,
            "findings_suggestive": y.findings_suggestive,
            "findings_refuted": y.findings_refuted,
            "negatives": y.negatives,
            "negatives_new_class": y.negatives_new_class,
            "questions_opened": y.questions_opened,
            "questions_closed": y.questions_closed,
            "contradictions_surfaced": y.contradictions_surfaced,
            "proposals": y.proposals,
            "hypotheses_tested": y.hypotheses_tested,
            "hypotheses_refuted": y.hypotheses_refuted,
            "hypotheses_supported": y.hypotheses_supported,
            "trials_spent": u32::try_from(self.trials_spent).unwrap_or(0),
        });
        if let Some(obj) = out.as_object_mut() {
            obj.insert(key.to_string(), value);
        }
        out
    }

    /// Decide this run's open hypotheses and count the verdicts.
    ///
    /// Failures are logged to the run's own event stream rather than raised: a
    /// verdict that could not be written must not stop the run record from being
    /// written, because the run record is how anybody finds out the run ended at
    /// all.
    async fn decide_hypotheses(&mut self) {
        let tenant = self.user_id.to_string();
        match crate::lifecycle_evaluate::decide_run(&self.pg, &tenant, self.run_id).await {
            Ok((decided, undecidable)) => {
                if decided.is_empty() && undecidable.is_empty() {
                    return;
                }
                use harness::lifecycle::evaluate::Verdict;
                self.yielded.hypotheses_tested = u32::try_from(decided.len()).unwrap_or(u32::MAX);
                self.yielded.hypotheses_refuted = u32::try_from(
                    decided
                        .iter()
                        .filter(|d| d.verdict == Verdict::Refuted)
                        .count(),
                )
                .unwrap_or(u32::MAX);
                self.yielded.hypotheses_supported = u32::try_from(
                    decided
                        .iter()
                        .filter(|d| d.verdict == Verdict::Supported)
                        .count(),
                )
                .unwrap_or(u32::MAX);
                let payload = crate::lifecycle_evaluate::as_json(&decided, &undecidable);
                self.append("verdicts", payload).await;
            }
            Err(e) => {
                self.append("status", json!({ "verdicts_not_written": e.to_string() }))
                    .await;
            }
        }
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
        Degradation::BudgetExhausted { axis } => format!(
            "The run reached its {axis} budget and did not report on the last turn that bought \
             it. Raise `budgets` in the profile, or narrow the task."
        ),
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

/// The effect log's schema version (AGENT-010 §5; S-2.6).
///
/// Changes far less often than `harness_version`, and is what a **suspended** run
/// is resumed under: a deploy inside a forty-minute sweep is likely rather than
/// exceptional, and discarding the sweep because the binary changed would make
/// the platform's most expensive operation its least reliable. Bump it when the
/// meaning of a row kind changes, not when the loop does.
pub const EFFECT_LOG_SCHEMA: u32 = 1;

/// Everything replay rebuilds a run's loop from (S-2.6).
///
/// Written once, at admission, as a `state` row of kind `RunSpec`, **before** the
/// first input. The loop is deterministic given its construction and its inputs,
/// so this plus the input rows is the run — and every value here is one that would
/// otherwise be re-read from a world that has moved on since (memory has grown, the
/// price table may have been edited, the conversation's prefix has advanced).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunSpec {
    pub goal: String,
    pub charter: String,
    pub prior: String,
    pub constraints: Value,
    pub profile: Profile,
    pub namespaces: Vec<String>,
    pub workflow: harness::state::Workflow,
    pub demand: TaskDemand,
    pub prior_exposure: Vec<String>,
    /// What memory showed this run, as it was at the start. Re-reading it on
    /// resume would show the run its own findings as prior knowledge.
    #[serde(default)]
    pub brief: Option<harness::memory::Brief>,
    /// The dollar cap, only when a fresh price row existed (ADR-D-03).
    #[serde(default)]
    pub usd_cap_tenth_cents: Option<i64>,
    /// The date the price table was read at, so resume prices calls the same way.
    #[serde(default)]
    pub priced_on: Option<String>,
    /// The cost estimator's inputs (S-2.16), when both halves were measured.
    #[serde(default)]
    pub cost_basis: Option<CostBasis>,
    /// Working state carried in from a lower tier that escalated (S-2.12).
    #[serde(default)]
    pub resumed_state: Option<harness::state::WorkingState>,
    /// The trial cap the run was started under, so a resume holds the same one.
    #[serde(default)]
    pub max_trials: Option<u32>,
    /// The calibrated bytes-per-token this run budgeted with, so a replay sizes
    /// every prompt the way the original did (S-4.12). `None` is the prior.
    #[serde(default)]
    pub bytes_per_token: Option<f64>,
}

/// What the cost axis prices a call from (AGENT-008 §4; S-2.16): this user's
/// measured backtest throughput, and the fresh price of a second of the platform's
/// compute. Both measured or neither used — an estimate from one real number and
/// one assumed one would be a number nobody measured.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct CostBasis {
    pub rows_per_second: f64,
    pub micro_usd_per_second: i64,
}

/// Bars in a call's window, when its arguments name one. `None` rather than a
/// guess: a call with no window has no size this can know.
fn rows_in_window(arguments: &Map<String, Value>) -> Option<u64> {
    let text = |k: &str| arguments.get(k).and_then(Value::as_str);
    let start = chrono::DateTime::parse_from_rfc3339(text("start")?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(text("end")?).ok()?;
    let tf = text("timeframe").unwrap_or("1h");
    let (size, unit) = tf.split_at(tf.len().checked_sub(1)?);
    let size: i64 = size.parse().ok()?;
    let unit = match unit {
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    let span = (end - start).num_seconds();
    (span > 0 && size > 0)
        .then(|| u64::try_from(span / (size * unit)).ok())
        .flatten()
}

impl CostBasis {
    /// The estimator the loop's cost axis reads. Prices only calls that spend a
    /// trial — the ones that run backtests on platform compute — and only when the
    /// arguments name a window.
    fn estimator(self) -> harness::drive::CostEstimator {
        Arc::new(move |tool: &str, arguments: &Map<String, Value>| {
            let spends = mcp_server_lib::spec::find(tool)
                .is_some_and(|s| s.spends_trial != mcp_server_lib::spec::TrialKind::None);
            if !spends || self.rows_per_second <= 0.0 {
                return None;
            }
            let rows = rows_in_window(arguments)?;
            #[allow(clippy::cast_precision_loss)]
            let seconds = rows as f64 / self.rows_per_second;
            #[allow(clippy::cast_precision_loss)]
            let usd = seconds * self.micro_usd_per_second as f64 / 1_000_000.0;
            Some(usd)
        })
    }
}

/// The shipped price table, read once (ADR-D-03). `None` when it is missing or
/// does not parse — logged, and every model is then unpriced, which leaves tokens
/// binding rather than inventing a dollar figure.
fn price_table() -> Option<&'static harness::adapter::PriceTable> {
    static TABLE: std::sync::OnceLock<Option<harness::adapter::PriceTable>> =
        std::sync::OnceLock::new();
    TABLE
        .get_or_init(|| {
            let path = std::env::var("TBOT_PRICES_PATH")
                .unwrap_or_else(|_| "config/prices.yaml".to_string());
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| harness::adapter::PriceTable::from_yaml(&t).map_err(|e| e.to_string()))
            {
                Ok(t) => Some(t),
                Err(e) => {
                    tracing::warn!(%path, error = %e, "no price table; dollar caps are not enforced and tokens stay binding");
                    None
                }
            }
        })
        .as_ref()
}

fn today() -> String {
    chrono::Utc::now().date_naive().to_string()
}

/// The charter with today's date stated, so a model never dates "now" from its training
/// data. A live test on 2026-10-05 asked gpt-4o-mini for the current BTC price and it queried
/// bars for 2023-10-06, its own training era. Day granularity keeps the charter, and with it
/// the cached prefix, stable across every turn of a day; the run spec records it, so a
/// replay rebuilds the same text.
fn with_today(charter: &str, today: &str) -> String {
    format!(
        "{charter}\n\nThe current time is {today} UTC (when this run started). When asked about \"now\", \
         \"current\" or \"latest\" anything, use this time: query the most recent data the platform has, \
         and say how old it is. \
         Never assume a date from your training data."
    )
}

/// The token rates for a run's model, from a fresh row only.
fn rates_for(spec: &RunSpec) -> Option<harness::adapter::TokenRates> {
    let on = spec.priced_on.as_deref()?;
    price_table()?.token_rates(&spec.profile.model_id, on)
}

/// A stable short hash of a working state, for the effect log's state rows.
pub(crate) fn state_hash(state: &harness::state::WorkingState) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(state).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// The loop for a spec. One builder for a fresh run and a resumed one, because a
/// replay through a loop built differently is not a replay.
fn build_governor(spec: &RunSpec, registry: harness::registry::ToolRegistry) -> Loop {
    let mut governor = Loop::new(
        spec.profile.clone(),
        registry,
        Task {
            // One builder for both front doors (S-1.6).
            goal: prompt::opening_message(Some(spec.prior.as_str()), &spec.constraints, &spec.goal),
            charter: format!(
                "{}\n\n\u{2500}\u{2500} YOUR WORKSPACE \u{2500}\u{2500}\n\n{}",
                spec.charter,
                workspace_tools::areas_hint()
            ),
            demand: spec.demand,
            namespaces: spec.namespaces.clone(),
        },
    )
    // The four §15 envelopes, each paired with the envelope the platform
    // classifies it under (ADR-P2-20).
    .with_gated_actions(
        ledger::audit::gated_actions()
            .into_iter()
            .map(|a| (a, ledger::audit::Envelope::for_action(a).as_str())),
    )
    .with_prior_exposure(spec.prior_exposure.clone())
    .with_trial_spending_tools(trial_spending_tools())
    .with_paper_tools(paper_reaching_tools())
    .with_workflow(spec.workflow)
    .with_usd_cap(spec.usd_cap_tenth_cents)
    .with_trial_cap(spec.max_trials)
    .with_bytes_per_token(spec.bytes_per_token);
    if let Some(brief) = spec.brief.clone() {
        governor = governor.with_brief(brief);
    }
    if let Some(basis) = spec.cost_basis {
        governor = governor.with_cost_estimator(basis.estimator());
    }
    if let Some(state) = spec.resumed_state.clone() {
        governor = governor.resuming_from(state);
    }
    governor
}

/// How a run's time in this process ended.
#[derive(Debug, Clone)]
pub enum Ended {
    /// The loop reached an outcome and the run row says so.
    Finished,
    /// Suspended on a platform job (§8). The run is `waiting_backtest`, holds no
    /// model connection and no task, and the manager resumes it when the job ends.
    Suspended { job_id: String, tool: String },
    /// Left at a step boundary, still `running`, for the next process to resume
    /// (a draining deploy, S-2.15).
    Detached,
    /// A Chat turn was offered a Research run and the user has told the platform
    /// to start those without asking (S-10.2). The manager starts it.
    StartResearch {
        spec: Box<RunSpec>,
        workflow: harness::state::Workflow,
    },
    /// The run escalated with its working state; the manager may start the next
    /// tier from it (S-2.12).
    Escalated {
        to: String,
        state: Option<Box<harness::state::WorkingState>>,
        spec: Box<RunSpec>,
    },
}

/// One run's IO half, from admission (or a resume) to wherever it stops.
struct Pump {
    pg: PgPool,
    client: LlmClient,
    run_id: Uuid,
    user_id: Uuid,
    conversation_id: Uuid,
    spec: RunSpec,
    rec: Recorder,
    governor: Loop,
    /// The provider thread, on the native transport (S-2.2). Started from the
    /// loop's first rendered body, so a native model reads exactly what a
    /// `select_fill` one would on its first call.
    transcript: Option<harness::transcript::Transcript>,
    native: bool,
    mcp_ctx: McpContext,
    ws: harness::workspace::Workspace,
    cancel: Arc<super::control::RunControl>,
    rates: Option<harness::adapter::TokenRates>,
    /// The most recent rejection, waiting to be handed to a native model.
    last_rejection: Option<String>,
    /// The directive a native model was last told, so a new one is told once.
    delivered_directive: Option<String>,
    /// Consecutive retryable model errors, for the backoff.
    error_streak: u32,
    /// The last `skill_notice` this run was told about, and the skills it mounted
    /// (R-a RA-2.6: a revocation is a notice appended to the thread, never an edit to
    /// the system prompt).
    skill_notice_cursor: i64,
    skills_resident: Vec<String>,
    /// Whether this run has read untrusted text (a `web` tool's result). Findings it
    /// writes after that carry the `tainted` mark, which keeps them out of the default
    /// retrieval and out of any skill promoted to global (KM-10).
    consumed_untrusted: bool,
}

/// Seconds to wait before re-asking a backend that said "not now".
///
/// Exponential from two seconds, capped at two minutes, and the provider's own
/// `retry-after` wins when it gives one. The loop re-issues immediately on a
/// retryable error; without this a rate limit would spend the retry budget in
/// three requests a few milliseconds apart.
fn backoff(e: &LlmError, streak: u32) -> Duration {
    if let LlmError::RateLimited {
        retry_after_secs: Some(s),
    } = e
    {
        return Duration::from_secs((*s).min(120));
    }
    let secs = 2u64.saturating_mul(4u64.saturating_pow(streak.min(4)));
    Duration::from_secs(secs.min(120))
}

impl Pump {
    /// Whether the user has a standing preference to start Research for this
    /// scope without being offered it (S-10.2).
    ///
    /// The scope is the run's instrument when its constraints name one; a
    /// preference scoped to `*` matches every scope either way. A failed read is
    /// "no": the offer is the safe side, because auto-starting spends trials the
    /// user did not just ask for, and an outage is not a preference.
    async fn prefers_research_without_asking(&self, offer: &harness::drive::ResearchOffer) -> bool {
        let scope = self
            .spec
            .constraints
            .get("instrument")
            .and_then(Value::as_str)
            .unwrap_or("*");
        match crate::memory_store::MemoryStore::new(self.pg.clone())
            .prefers_always_start(self.user_id, scope)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(run_id = %self.run_id, error = %e, question = %offer.question,
                    "could not read the always-start preference; the offer stands");
                false
            }
        }
    }

    /// A transcript mutation, logged before it is applied (S-2.6), so a resumed
    /// native run rebuilds exactly the thread the provider last saw.
    async fn push_turn(&mut self, turn: harness::transcript::Turn) {
        if self.transcript.is_none() {
            return;
        }
        self.rec
            .event(
                "state",
                "Turn",
                serde_json::to_value(&turn).unwrap_or(Value::Null),
            )
            .await;
        if let Some(t) = self.transcript.as_mut() {
            t.push_turn(turn);
        }
    }

    /// Before a native call: answer every call the thread left open, and hand
    /// over what the loop decided since the last call (S-2.9).
    ///
    /// Providers reject a request whose assistant turn asked for a call nobody
    /// answered, and a native model is otherwise never told that its reply was
    /// rejected, that the budget is spent, or that the user steered — the loop
    /// renders all of that into a body the native transport does not send.
    async fn prepare_native(&mut self, call: &harness::drive::ModelCall) {
        if !self.native {
            return;
        }
        if self.transcript.is_none() {
            // The first call: the loop's own first view of the task, which is
            // what a `select_fill` call would have sent.
            self.rec
                .event("state", "Opening", json!({ "text": call.prompt }))
                .await;
            self.transcript = Some(harness::transcript::Transcript::new(call.prompt.clone()));
            self.delivered_directive = self.governor.directive().map(str::to_string);
            return;
        }
        let open = self
            .transcript
            .as_ref()
            .map(harness::transcript::Transcript::unanswered_calls)
            .unwrap_or_default();
        let rejection = self.last_rejection.take();
        for (i, c) in open.iter().enumerate() {
            let body = match (&rejection, i) {
                (Some(r), 0) => format!("REJECTED by the harness, not run: {r}"),
                _ => "not run: the harness takes one tool call per turn. Call it again if you \
                      still need it."
                    .to_string(),
            };
            self.push_turn(harness::transcript::Turn::ToolResult {
                call_id: c.id.clone(),
                name: c.name.clone(),
                body,
                is_error: true,
                handle: None,
            })
            .await;
        }
        if open.is_empty() {
            if let Some(r) = rejection {
                self.push_turn(harness::transcript::Turn::User {
                    text: format!(
                        "[harness] Your last reply was rejected: {r}. Answer with a tool call."
                    ),
                })
                .await;
            }
        }
        let now = self.governor.directive().map(str::to_string);
        if now != self.delivered_directive {
            if let Some(text) = &now {
                self.push_turn(harness::transcript::Turn::User {
                    text: format!("[harness] {text}"),
                })
                .await;
            }
            self.delivered_directive = now;
        }
        if !self.skills_resident.is_empty() {
            let (cursor, notices) =
                skills_mount::pending_notices(&self.pg, self.skill_notice_cursor, &self.skills_resident)
                    .await;
            self.skill_notice_cursor = cursor;
            for text in notices {
                self.push_turn(harness::transcript::Turn::User {
                    text: format!("[harness] {text}"),
                })
                .await;
            }
        }
    }

    /// Where an elided body can be read back, written before it is needed
    /// (AGENT-005 P6; S-2.10). A handle that names a file nobody wrote is the
    /// `[elided — was N bytes]` hole with a path on it.
    async fn result_handle(&self, name: &str, text: &str) -> Option<String> {
        if !self.native || text.len() <= 200 {
            return None;
        }
        let n = self.transcript.as_ref().map_or(0, harness::transcript::Transcript::len);
        let path = format!("work/transcript/{}-{n}-{name}.txt", self.run_id);
        let args = Map::from_iter([
            ("path".to_string(), json!(path)),
            ("content".to_string(), json!(text)),
        ]);
        let (_, failed) = workspace_tools::execute(&self.ws, "write_file", &args).await;
        (!failed).then_some(path)
    }

    /// The result a write already returned this run, if the durable record has
    /// one (S-1.19).
    async fn recorded_result(&self, key: &str) -> Option<Value> {
        sqlx::query_scalar("SELECT result FROM agent_idempotency WHERE run_id = $1 AND key = $2")
            .bind(self.run_id)
            .bind(key)
            .fetch_optional(&self.pg)
            .await
            .ok()
            .flatten()
    }

    async fn record_result(&self, key: &str, tool: &str, result: &Value) {
        if let Err(e) = sqlx::query(
            "INSERT INTO agent_idempotency (run_id, key, tool, result) VALUES ($1, $2, $3, $4)
             ON CONFLICT (run_id, key) DO NOTHING",
        )
        .bind(self.run_id)
        .bind(key)
        .bind(tool)
        .bind(result)
        .execute(&self.pg)
        .await
        {
            tracing::warn!(run_id = %self.run_id, error = %e,
                "the write's result was not recorded; a post-crash resume will not be able to answer it");
        }
    }

    /// Feed one input: logged first, then into the loop.
    ///
    /// A `WorkingState` row goes in front of every input (AGENT-010 §5; S-2.5):
    /// the hash of what the run believed at that boundary. Replay checks it, so a
    /// rebuilt loop that took the same actions for a different reason is caught
    /// rather than resumed.
    /// Add this run to the attribution corpus (S-9.10; AGENT-005 section 19).
    ///
    /// The one call site of `RunEntry::from_attribution`: until it existed the
    /// corpus the weight fit reads was empty by construction. A run under the
    /// buffer policy rendered no retrieval records and contributes nothing, which
    /// is stated by writing nothing rather than a row of zeros. A write failure
    /// is logged and swallowed: a lost corpus row costs a data point, and failing
    /// a finished run to protect it would trade the certain thing for the
    /// contingent one.
    async fn record_context_corpus(&self) {
        let tape = self.governor.context_tape();
        if tape.renders.is_empty() {
            return;
        }
        let attribution = self.governor.context_attribution();
        let entry = crate::context_corpus::RunEntry::from_attribution(
            self.run_id,
            &self.spec.profile.model_id,
            self.spec.profile.tier_label(),
            &self.governor.context_policy().version,
            self.governor.context_store(),
            &attribution,
            |id| tape.signals.get(&id).cloned(),
        );
        if let Err(e) = crate::context_corpus::record_run(&self.pg, &entry).await {
            tracing::warn!(run_id = %self.run_id, error = %e, "the attribution corpus did not record this run");
        }
    }

    async fn feed(&mut self, input: Input) -> Vec<Effect> {
        self.rec
            .event(
                "state",
                "WorkingState",
                json!({ "step": self.governor.step(), "hash": state_hash(self.governor.working()) }),
            )
            .await;
        let (kind, payload) = describe_input(&input);
        self.rec.input(kind, payload).await;
        self.governor.next(input)
    }

    /// The pump. `logged` says the first batch is already in the log and its
    /// notes already rendered — true on a resume, where the batch is the one the
    /// replay just rebuilt.
    #[allow(clippy::too_many_lines)]
    async fn run(mut self, mut effects: Vec<Effect>, mut logged: bool) -> Ended {
        loop {
            // **Before anything is executed** (S-2.5): a row saying what was about
            // to happen is what tells a resume whether running it again would
            // write twice.
            if !logged {
                self.rec.effects(&effects).await;
            }

            let mut blocking: Option<Effect> = None;
            for e in effects {
                match e {
                    Effect::Note(n) => {
                        if let Note::Rejected { code, message, .. } = &n {
                            self.last_rejection = Some(format!("{code}: {message}"));
                        }
                        if !logged {
                            self.rec.note(&n).await;
                        }
                    }
                    Effect::Remember(e) if !logged => {
                        match e.kind {
                            harness::memory::Kind::NegativeResult => {
                                let is_new = self.rec.negative_class_is_new(&e).await;
                                self.rec.yielded.record_negative(is_new);
                            }
                            harness::memory::Kind::OpenQuestion => {
                                self.rec.yielded.questions_opened += 1;
                            }
                            _ => self.rec.yielded.record_finding(e.confidence),
                        }
                        self.rec.remember(self.user_id, &e).await;
                    }
                    Effect::Remember(_) => {}
                    // The bytes leave memory and land in the agent's own folder
                    // (§26.3). On a resume the file is already there.
                    Effect::Spill {
                        record,
                        handle,
                        body,
                    } => {
                        if logged {
                            continue;
                        }
                        let text = serde_json::to_string(&*body).unwrap_or_default();
                        let args = Map::from_iter([
                            ("path".to_string(), json!(handle)),
                            ("content".to_string(), json!(text)),
                        ]);
                        let (detail, failed) =
                            workspace_tools::execute(&self.ws, "write_file", &args).await;
                        if failed {
                            self.governor.restore_spilled(record, *body);
                            tracing::warn!(
                                run_id = %self.run_id, %handle, %detail,
                                "context spill failed; the record keeps its body"
                            );
                        }
                    }
                    Effect::Done(o) => {
                        // Where the offer is emitted (S-10.2): a user who has said
                        // "always start research" for this scope is not asked.
                        let auto_start = match &o {
                            Outcome::Offered(offer) => {
                                self.prefers_research_without_asking(offer).await
                            }
                            _ => false,
                        };
                        self.rec.offer_auto_started = auto_start;
                        self.rec
                            .write_turn_summary(self.conversation_id, &self.governor, &o)
                            .await;
                        self.rec.finish(&o).await;
                        self.record_context_corpus().await;
                        if let Outcome::Offered(offer) = &o {
                            if auto_start {
                                return Ended::StartResearch {
                                    spec: Box::new(self.spec.clone()),
                                    workflow: offer.workflow,
                                };
                            }
                        }
                        if let Outcome::Escalated { to, state, .. } = o {
                            return Ended::Escalated {
                                to,
                                state,
                                spec: Box::new(self.spec.clone()),
                            };
                        }
                        return Ended::Finished;
                    }
                    other => blocking = Some(other),
                }
            }
            logged = false;
            let Some(blocking) = blocking else {
                tracing::error!(run_id = %self.run_id, "the loop yielded no effect and no outcome");
                self.rec
                    .finish(&Outcome::Fenced(Degradation::BackendFailure {
                        detail: "the loop yielded no effect and no outcome".into(),
                    }))
                    .await;
                return Ended::Finished;
            };

            // ── The step boundary (ADR-D-27) ─────────────────────────────────
            if let Some(until) = self.cancel.paused_until() {
                self.rec
                    .append(
                        "status",
                        json!({
                            "phase": "paused",
                            "until": until.to_rfc3339(),
                            "note": "held at a step boundary; on expiry this run stops, it does not resume",
                        }),
                    )
                    .await;
            }
            match self.cancel.hold_at_boundary().await {
                super::control::Hold::Proceed => {}
                super::control::Hold::Resumed => {
                    self.rec.append("status", json!({ "phase": "resumed" })).await;
                }
                super::control::Hold::Stop(reason) => {
                    self.rec
                        .append(
                            "status",
                            json!({ "phase": "stopping", "reason": reason.as_str() }),
                        )
                        .await;
                    self.rec
                        .input("Cancel", json!({ "reason": reason.as_str() }))
                        .await;
                    self.rec.stop_reason = Some(reason);
                    effects = self.governor.next(Input::Cancel);
                    continue;
                }
            }
            // A deploy: leave the run where it is, still `running`, with its next
            // effect already in the log. The next process resumes it (S-2.15).
            if self.cancel.is_detached() {
                self.rec
                    .append(
                        "status",
                        json!({ "phase": "detached",
                                "note": "the platform is restarting; this run resumes from its effect log on the next boot" }),
                    )
                    .await;
                return Ended::Detached;
            }
            // A message the user sent mid-run, delivered where it can change the
            // next decision: before a model call (S-2.14). Mid-tool it would have
            // nothing to act on until the tool returned anyway.
            if matches!(blocking, Effect::CallModel(_)) {
                // A tool the human pinned from the desk (S-6.11): fed as an input
                // so it is in the effect log, and the router reads it at the next
                // routing pass. The row makes the pin visible in the run's log.
                // Not at a fill or plan call: the loop re-routes on a pin, which would throw
                // away a half-chosen tool or a plan in flight for nothing.
                let at_fill = matches!(
                    &blocking,
                    Effect::CallModel(c) if matches!(c.decode, Decode::FillArguments { .. } | Decode::Plan { .. })
                );
                if !at_fill {
                    if let Some(tool) = self.cancel.take_pin() {
                        self.rec.append("pin", json!({ "tool": tool })).await;
                        effects = self.feed(Input::Pin { tool }).await;
                        continue;
                    }
                }
                if let Some(text) = self.cancel.take_steer() {
                    self.rec.append("steer", json!({ "text": text })).await;
                    if self.transcript.is_some() {
                        self.push_turn(harness::transcript::Turn::User {
                            text: format!("[the user, while you were working] {text}"),
                        })
                        .await;
                    }
                    effects = self.feed(Input::Steer { text }).await;
                    continue;
                }
            }

            let input = match blocking {
                Effect::Remember(_) | Effect::Spill { .. } => {
                    unreachable!("handled with the notes")
                }
                Effect::CallModel(call) => self.call_model(&call).await,
                Effect::ExecuteTool {
                    step,
                    name,
                    namespace,
                    arguments,
                    risk,
                    ..
                } => self.execute_tool(step, name, namespace, arguments, risk).await,
                Effect::AskHuman {
                    step,
                    tool,
                    arguments,
                    ruling,
                } => {
                    match await_approval(
                        &self.pg,
                        &self.mcp_ctx,
                        self.run_id,
                        step,
                        &tool,
                        &arguments,
                        &ruling,
                        &self.cancel,
                    )
                    .await
                    {
                        Some((granted, note)) => Input::Approval { granted, note },
                        None => Input::Cancel,
                    }
                }
                // AGENT-010 §8: suspend on the job and exit. The resume contract is
                // written first — the job, the tool, and the schema it was written
                // under — so a deploy during the wait resumes rather than restarts.
                Effect::Await { step, job_id, tool } => {
                    self.rec
                        .event(
                            "state",
                            "Suspended",
                            json!({ "step": step, "job_id": job_id, "tool": tool,
                                    "schema": EFFECT_LOG_SCHEMA }),
                        )
                        .await;
                    let _ = sqlx::query(
                        "UPDATE agent_runs SET status = 'waiting_backtest' WHERE run_id = $1",
                    )
                    .bind(self.run_id)
                    .execute(&self.pg)
                    .await;
                    self.rec
                        .append(
                            "status",
                            json!({ "phase": "waiting", "job_id": job_id, "tool": tool,
                                    "note": "suspended on a platform job: no model connection and no tokens until it finishes" }),
                        )
                        .await;
                    return Ended::Suspended { job_id, tool };
                }
                Effect::Note(_) | Effect::Done(_) => unreachable!("handled above"),
            };

            effects = self.feed(input).await;
        }
    }

    async fn call_model(&mut self, call: &harness::drive::ModelCall) -> Input {
        self.prepare_native(call).await;
        let started = Instant::now();
        let req = build_request(&self.spec.profile, call, self.transcript.as_ref());

        // The proxy's refusal, applied here too (S-2.7). A regression alarm, not a
        // safety net: it refuses rather than truncating, so drift is visible.
        let estimated = crate::llm_proxy::estimate_request_tokens(
            &serde_json::to_value(&req).unwrap_or_else(|_| json!({})),
        );
        let cap = self.spec.profile.input_budget_tokens();
        if estimated > cap {
            tracing::error!(
                run_id = %self.run_id, estimated, cap,
                "an internal request exceeded the context budget; the assembler's arithmetic and what is sent have drifted apart"
            );
            return Input::ModelError {
                retryable: false,
                detail: format!(
                    "context_budget_exceeded: about {estimated} input tokens against a budget of \
                     {cap}. The context assembler and the request builder disagree; this is a \
                     harness defect, not a model one."
                ),
            };
        }

        // The no-progress watchdog (AGENT-010 §6; S-2.12). Idle, not elapsed: a
        // run suspended on a sweep is not here at all, so only a call that is
        // genuinely hung reaches the limit.
        let idle = Duration::from_secs(
            self.spec.profile.orchestration.idle_timeout_s.max(1),
        );
        let reply = match tokio::time::timeout(idle, self.client.chat(&req)).await {
            Ok(r) => r,
            Err(_) => {
                return Input::ModelError {
                    retryable: false,
                    detail: format!(
                        "stalled: no reply within {}s (orchestration.idle_timeout_s)",
                        idle.as_secs()
                    ),
                };
            }
        };
        match reply {
            Ok(resp) => {
                self.error_streak = 0;
                let estimated = harness::context::estimate_tokens(&call.system)
                    + harness::context::estimate_tokens(&call.prompt);
                // What it cost, told to the loop and logged before the reply is
                // fed, so a replay reaches the same wrap-up at the same step.
                let tokens = resp
                    .usage
                    .prompt_tokens()
                    .saturating_add(resp.usage.output_tokens);
                let usd = self.rates.map(|r| {
                    r.cost_tenth_cents(harness::adapter::CallTokens {
                        input: resp.usage.input_tokens,
                        output: resp.usage.output_tokens,
                        cache_read: resp.usage.cache_read_input_tokens,
                        cache_write: resp.usage.cache_creation_input_tokens,
                    })
                });
                self.rec
                    .usage(
                        call.step,
                        &resp.usage,
                        estimated,
                        started.elapsed().as_millis(),
                        usd,
                        layer_tokens(call),
                    )
                    .await;
                self.rec
                    .event(
                        "state",
                        "Spend",
                        json!({ "tokens": tokens, "usd_tenth_cents": usd }),
                    )
                    .await;
                self.governor.record_spend(tokens, usd);

                if self.transcript.is_some() {
                    self.push_turn(harness::transcript::Turn::Assistant {
                        text: resp.content.clone(),
                        calls: resp
                            .tool_calls
                            .iter()
                            .map(|c| harness::transcript::CallRef {
                                id: c.id.clone(),
                                name: c.name.clone(),
                                arguments: c.arguments.to_string(),
                            })
                            .collect(),
                    })
                    .await;
                    self.rec.event("state", "Compact", json!({})).await;
                    let report = self
                        .transcript
                        .as_mut()
                        .map(|t| match self.spec.bytes_per_token {
                            Some(b) => harness::context::with_bytes_per_token(b, || {
                                t.compact(&self.spec.profile, 0)
                            }),
                            None => t.compact(&self.spec.profile, 0),
                        })
                        .unwrap_or_default();
                    if report.truncated > 0 {
                        self.rec
                            .append(
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
                self.rec
                    .decision(call, &raw, &resp, started.elapsed().as_millis())
                    .await;
                if resp.stop_reason == llm::StopReason::MaxTokens {
                    Input::ModelTruncated { raw }
                } else {
                    Input::ModelReply { raw }
                }
            }
            Err(e) => {
                let retryable = is_retryable(&e);
                if retryable {
                    let wait = backoff(&e, self.error_streak);
                    self.error_streak = self.error_streak.saturating_add(1);
                    tracing::warn!(run_id = %self.run_id, error = %e, wait_s = wait.as_secs(),
                        "model call failed; backing off before the loop re-asks");
                    tokio::time::sleep(wait).await;
                }
                Input::ModelError {
                    retryable,
                    detail: e.to_string(),
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn execute_tool(
        &mut self,
        step: u32,
        name: String,
        namespace: String,
        mut arguments: Map<String, Value>,
        risk: harness::registry::Risk,
    ) -> Input {
        if mcp_server_lib::spec::find(&name)
            .is_some_and(|spec| spec.spends_trial != mcp_server_lib::spec::TrialKind::None)
        {
            self.rec.trials_spent += 1;
        }
        self.rec
            .append(
                "tool_call",
                json!({ "step": step, "name": name, "risk": risk.as_str(),
                        "arguments": arguments }),
            )
            .await;

        // Long platform work (AGENT-010 §8; S-2.11). The call is dispatched through
        // the same catalogue path as every other — audited, gated, its trial
        // counted — but told not to wait: a sweep or a backtest still running
        // comes back as a handle, and the run suspends on it instead of holding a
        // model connection for forty minutes.
        let long = match name.as_str() {
            "run_sweep" => {
                arguments.insert("timeout_seconds".into(), json!(0));
                true
            }
            "wait_for_backtest" => {
                // The tool clamps to ten seconds; that is its floor, not a wait.
                arguments.insert("timeout_seconds".into(), json!(10));
                true
            }
            // Training polls inside the tool until its own deadline; thirty
            // seconds is the tool's floor. A run still going comes back as
            // `still_running` with its ids (S-2.11).
            "train_model" => {
                arguments.insert("timeout_seconds".into(), json!(30));
                true
            }
            // A Study is a blocking route; `detach` makes it return 202 at once
            // and run in the background, so the run suspends instead of
            // holding a connection for minutes (S-2.11).
            "run_study" => {
                arguments.insert("detach".into(), json!(true));
                true
            }
            _ => false,
        };

        let arguments = self.complete_arguments(step, &name, arguments).await;

        // A write the run already made is answered from its record (S-1.19).
        let key = (risk != harness::registry::Risk::Read).then(|| {
            mcp_server_lib::idempotency::key(self.run_id, step, &name, &Value::Object(arguments.clone()))
        });
        let recorded = match &key {
            Some(k) => self.recorded_result(k).await,
            None => None,
        };

        let out = if let Some(prev) = recorded {
            self.rec
                .append("status", json!({ "replayed_write": name, "note": "answered from this run's own record; the write was not repeated" }))
                .await;
            bridge::Executed {
                text: match &prev {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                },
                is_error: false,
                raw: prev,
            }
        } else if workspace_tools::is_workspace_tool(&namespace) {
            let started = Instant::now();
            let (text, is_error) = workspace_tools::execute(&self.ws, &name, &arguments).await;
            // R-a RA-2.4: reading a mounted SKILL.md is the activation (SK-13).
            if !is_error && name == "read_file" {
                if let Some(skill) = arguments
                    .get("path")
                    .and_then(Value::as_str)
                    .and_then(workspace_tools::activated_skill)
                {
                    let _ = crate::skills_store::SkillStore::new(self.pg.clone())
                        .record_activation(self.run_id, &skill)
                        .await;
                }
            }
            let raw = if is_error {
                json!({ "error": text })
            } else {
                json!({ "result": text })
            };
            self.mcp_ctx
                .record_step(
                    &name,
                    &Value::Object(arguments.clone()),
                    &raw,
                    started.elapsed().as_millis(),
                    None,
                )
                .await;
            bridge::Executed {
                raw: json!({ "result": text }),
                text,
                is_error,
            }
        } else {
            bridge::execute(&self.mcp_ctx, &name, &arguments).await
        };
        if !out.is_error {
            if let Some(k) = &key {
                self.record_result(k, &name, &out.raw).await;
            }
        }
        self.rec
            .append(
                "tool_result",
                json!({ "name": name, "is_error": out.is_error, "content": out.raw }),
            )
            .await;

        // Still running: a handle, not a result. The tool's answer arrives as
        // `JobDone` when the job ends, and joins the thread then.
        if long && !out.is_error {
            if let Some(job_id) = long_job_handle(&name, &arguments, &out.raw) {
                return Input::JobAccepted { job_id };
            }
        }

        // A campaign is long work too (AGENT-013 §8.5; S-10.11): the parent
        // suspends on it and the job watcher resumes the run when the campaign
        // reaches a terminal phase. Until resumption existed this was refused,
        // because suspending a run that could not resume would have killed a
        // healthy one and labelled the death a backend failure.
        if let Some(job_id) = campaign_handle(&name, out.is_error, &out.raw) {
            return Input::JobAccepted { job_id };
        }

        if self.transcript.is_some() {
            let call_id = self.open_call_id(&name, step);
            let handle = self.result_handle(&name, &out.text).await;
            self.push_turn(harness::transcript::Turn::ToolResult {
                call_id,
                name: name.clone(),
                body: out.text.clone(),
                is_error: out.is_error,
                handle,
            })
            .await;
        }
        if out.is_error {
            Input::ToolError { detail: out.text }
        } else {
            if harness::memory::consumes_untrusted(&namespace) {
                self.consumed_untrusted = true;
                self.rec.consumed_untrusted = true;
            }
            Input::ToolResult {
                output: out.text,
                source: bridge::source_for(&namespace),
            }
        }
    }

    /// The id the provider gave the call this result answers. Most providers
    /// reject a result whose id matches no call in the thread.
    fn open_call_id(&self, name: &str, step: u32) -> String {
        self.transcript
            .as_ref()
            .and_then(|t| {
                t.unanswered_calls()
                    .into_iter()
                    .find(|c| c.name == name)
                    .map(|c| c.id)
            })
            .unwrap_or_else(|| format!("call_{step}"))
    }

    /// The arguments the platform completes (S-10.3, S-10.11): a `preregister`
    /// skeleton on a tier that does not author freely, and the campaign brief.
    async fn complete_arguments(
        &mut self,
        step: u32,
        name: &str,
        arguments: Map<String, Value>,
    ) -> Map<String, Value> {
        if name == "preregister" {
            match harness::lifecycle::skeleton::for_run(
                &self.spec.profile,
                &self.spec.constraints,
                None,
                DEFAULT_GATE_PROFILE,
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
                    // The model's own values win.
                    for (k, v) in skeleton.filled {
                        filled.entry(k).or_insert(v);
                    }
                    self.rec
                        .append(
                            "status",
                            json!({
                                "prereg_authoring": self.spec.profile.prereg_authoring.as_str(),
                                "skeleton_filled": filled.len() - arguments.len(),
                                "still_yours": still_yours,
                            }),
                        )
                        .await;
                    filled
                }
            }
        } else if name == "define_campaign" {
            let mut with_brief = arguments;
            with_brief.insert("run_id".into(), json!(self.run_id));
            let query =
                harness::context::select::Query::from_state(self.governor.working(), step);
            let brief = subrun::assemble(
                self.governor.context_store(),
                &query,
                subrun::brief_budget(&self.spec.profile),
                self.governor.context_policy(),
                subrun::BRIEF_MANIFEST_TOKENS,
            );
            with_brief.insert(
                "brief".into(),
                serde_json::to_value(subrun::PinnedBrief::of(&brief)).unwrap_or(Value::Null),
            );
            self.rec
                .append(
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
        }
    }
}

/// Runs one turn to a terminal state, or to a suspension.
#[allow(clippy::too_many_lines)]
pub async fn run(
    pg: PgPool,
    client: LlmClient,
    params: LocalRunParams,
    cancel: Arc<super::control::RunControl>,
) -> Ended {
    let mut rec = Recorder::new(
        pg.clone(),
        params.run_id,
        params.user_id,
        params.exposure_out.clone(),
    );

    // ── Registry ────────────────────────────────────────────────────────────
    let registry = match bridge::registry_with_workspace() {
        Ok(r) => r,
        Err(e) => {
            rec.finish(&Outcome::Refused {
                reason: e.to_string(),
                fix: "classify the tool in crates/mcp-server/src/taxonomy.rs".into(),
            })
            .await;
            return Ended::Finished;
        }
    };

    // ── Hardware, then admission ────────────────────────────────────────────
    let hw = hardware_probe::detect().await;
    let description = hardware_probe::describe(&hw);
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

    if let Some(req) = &params.profile.requires {
        if let Err(unfit) = hardware::fits(&hw, req) {
            // One exemption: the model is already resident, so it fits by
            // demonstration. Narrow on purpose — see `hardware::fits`.
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
                return Ended::Finished;
            }
        }
    }
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
            "tool_calling_demoted_because": demotion,
        }),
    )
    .await;
    let admission = hardware::admit(
        capability,
        params.demand,
        params.profile.escalate_to.as_deref(),
    );

    // A task this tier cannot take is answered before the canary.
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
            }
        }
        return Ended::Finished;
    }

    // The tier does not offer this workflow (AGENT-013 §11): refused, not
    // downgraded.
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
                || {
                    "choose a workflow this tier offers, or configure a tier that offers this one"
                        .to_string()
                },
                |to| format!("run this on `{to}`, which is where this profile escalates"),
            ),
        })
        .await;
        return Ended::Finished;
    }

    // ── The canary, and the adapter checked against the profile ─────────────
    //
    // Every tier now (S-2.10). The frontier path used to start without any
    // check at all, so a profile asking for something the adapter cannot do
    // would surface as a model that "will not follow instructions".
    if params.profile.output.constrained_decoding {
        let verdicts = match run_canary(&client, &params.profile).await {
            Ok(v) => v,
            Err(e) => {
                rec.finish(&Outcome::Fenced(Degradation::BackendFailure { detail: e }))
                    .await;
                return Ended::Finished;
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
            return Ended::Finished;
        }
    } else if let Err(e) = adapter::verify(&*adapter::for_profile(&params.profile), &params.profile)
    {
        rec.finish(&Outcome::Refused {
            reason: e.to_string(),
            fix: "correct the profile for this backend; the refusal names the field".into(),
        })
        .await;
        return Ended::Finished;
    }

    // ── The agent's own folder ──────────────────────────────────────────────
    let ws = workspace_tools::workspace_for(&params.workspace_root, params.conversation_id);
    if let Err(e) = workspace_tools::ensure(&ws).await {
        tracing::warn!(error = %e, "could not create the agent workspace");
    }
    // Set R-a: the session's skills, mounted read-only under `.claude/skills`.
    let session_skills = skills_mount::session_start(
        &pg,
        &ws,
        params.user_id,
        params.run_id,
        &params.profile.model_id,
    )
    .await;
    let skill_notice_cursor = crate::skills_store::SkillStore::new(pg.clone())
        .latest_notice()
        .await
        .unwrap_or(0);

    let mcp_ctx = run_context(&pg, &params.api_base, &params.service_token, params.run_id, params.demand, params.workflow).await;

    // ── The run's spec: what replay rebuilds it from ────────────────────────
    let brief = load_brief(
        &pg,
        params.user_id,
        params.run_id,
        &params.goal,
        &params.profile,
    )
    .await;
    let priced_on = today();
    let priced = price_table()
        .and_then(|t| t.token_rates(&params.profile.model_id, &priced_on))
        .is_some();
    let cost_basis = cost_basis(&mcp_ctx, &priced_on).await;
    let spec = RunSpec {
        goal: params.goal.clone(),
        charter: with_today(
            &skills_mount::with_skills(&params.charter, &session_skills),
            &chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string(),
        ),
        prior: params.prior.clone(),
        constraints: params.constraints.clone(),
        profile: params.profile.clone(),
        namespaces: params.namespaces.clone(),
        workflow: params.workflow,
        demand: params.demand,
        prior_exposure: params.prior_exposure.clone(),
        brief,
        // Enforced only against a fresh price (ADR-D-03); otherwise tokens bind.
        usd_cap_tenth_cents: if priced {
            harness::adapter::usd_cap_tenth_cents(&params.profile)
        } else {
            None
        },
        priced_on: Some(priced_on),
        cost_basis,
        resumed_state: params.resumed_state.clone(),
        max_trials: params.max_trials,
        // The eval store's calibrated row; the prior when the model has none or
        // the read fails (S-4.12). Recorded in the spec, so resume uses the same.
        bytes_per_token: crate::eval_store::bytes_per_token(&pg, &params.profile.model_id)
            .await
            .ok()
            .filter(|b| (b - harness::context::PRIOR_BYTES_PER_TOKEN).abs() > f64::EPSILON),
    };
    rec.event(
        "state",
        "RunSpec",
        serde_json::to_value(&spec).unwrap_or(Value::Null),
    )
    .await;
    let governor = build_governor(&spec, registry);
    rec.retrieved = spec
        .brief
        .as_ref()
        .map(|b| b.retrievals.clone())
        .unwrap_or_default();
    let rates = rates_for(&spec);
    let native = !spec.profile.output.constrained_decoding;

    let mut pump = Pump {
        pg,
        client,
        run_id: params.run_id,
        user_id: params.user_id,
        conversation_id: params.conversation_id,
        spec,
        rec,
        governor,
        transcript: None,
        native,
        mcp_ctx,
        ws,
        cancel,
        rates,
        last_rejection: None,
        delivered_directive: None,
        error_streak: 0,
        skill_notice_cursor,
        skills_resident: session_skills.resident.clone(),
        consumed_untrusted: false,
    };
    let effects = pump.feed(Input::Admission(admission)).await;
    pump.run(effects, false).await
}

/// The run's `McpContext`: its pre-registration, its run id, its workflow and
/// what it may spend (S-10.1, S-10.3, S-10.14, ADR-S10-10).
async fn run_context(
    pg: &PgPool,
    api_base: &str,
    service_token: &str,
    run_id: Uuid,
    demand: TaskDemand,
    workflow: harness::state::Workflow,
) -> McpContext {
    let ctx = McpContext::new(ApiClient::new(
        api_base.to_string(),
        service_token.to_string(),
    ))
    .under_preregistration(sealed_registration(pg, run_id, demand).await)
    .for_run(run_id.to_string())
    .following(workflow.as_str());
    ctx.spending(if demand == TaskDemand::SingleLookup {
        mcp_server_lib::gate::Spending::ChatTurn
    } else if workflow == harness::state::Workflow::Survey {
        mcp_server_lib::gate::Spending::Survey
    } else {
        mcp_server_lib::gate::Spending::Allowed
    })
}

/// The cost axis's two measured inputs, or `None` (S-2.16).
///
/// The throughput comes from the platform's own estimator (S-3.19), asked once at
/// admission; the price of a second from a fresh hardware row. The hardware class
/// is named by `TBOT_HARDWARE_CLASS`, because the platform cannot tell what its
/// compute costs by looking at itself.
async fn cost_basis(ctx: &McpContext, today: &str) -> Option<CostBasis> {
    let class = std::env::var("TBOT_HARDWARE_CLASS").ok()?;
    let micro_usd_per_second = price_table()?.hardware_micro_usd_per_second(&class, today)?;
    let answer = ctx
        .api
        .post(
            "/api/agent/cost/estimate",
            json!({ "tool": "backtest_strategy", "arguments": {} }),
        )
        .await
        .ok()?;
    let rows_per_second = answer
        .pointer("/detail/throughput_rows_per_second")
        .and_then(Value::as_f64)
        .filter(|t| *t > 0.0)?;
    Some(CostBasis {
        rows_per_second,
        micro_usd_per_second,
    })
}

// ── Resume (AGENT-010 §5; S-2.6, S-2.11) ────────────────────────────────────

/// A finished job, for a run suspended on it.
#[derive(Debug, Clone)]
pub struct JobDone {
    pub job_id: String,
    pub output: String,
    pub ok: bool,
}

/// What a resume needs that the log does not hold.
pub struct ResumeParams {
    pub run_id: Uuid,
    pub user_id: Uuid,
    pub conversation_id: Uuid,
    pub workspace_root: std::path::PathBuf,
    pub api_base: String,
    pub service_token: String,
    pub exposure_out: Arc<std::sync::Mutex<Vec<String>>>,
    /// The job result to deliver, when the run is suspended on one.
    pub job_done: Option<JobDone>,
}

/// What a resume did.
#[derive(Debug, Clone)]
pub enum Resumed {
    /// The pump ran again and stopped here.
    Ran(Ended),
    /// It could not be resumed; the run is marked `interrupted` with its working
    /// state kept and, for a suspended run, the job's result attached.
    Interrupted(String),
    /// Still waiting on its job; nothing was done.
    StillWaiting,
}

/// Rebuild an `Input` from its log row. `None` for a row this harness cannot
/// read back, which interrupts the resume rather than guessing.
fn input_from_row(kind: &str, p: &Value) -> Option<Input> {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let b = |k: &str| p.get(k).and_then(Value::as_bool);
    Some(match kind {
        "Admission" => {
            if p.get("admission").and_then(Value::as_str) == Some("Admit") {
                Input::Admission(hardware::Admission::Admit)
            } else {
                return None;
            }
        }
        "ModelReply" => Input::ModelReply { raw: s("raw")? },
        "ModelTruncated" => Input::ModelTruncated { raw: s("raw")? },
        "ModelError" => Input::ModelError {
            detail: s("detail")?,
            retryable: b("retryable")?,
        },
        "ToolResult" => Input::ToolResult {
            output: s("output")?,
            source: s("source")?,
        },
        "ToolError" => Input::ToolError { detail: s("detail")? },
        "Approval" => Input::Approval {
            granted: b("granted")?,
            note: s("note").unwrap_or_default(),
        },
        "Steer" => Input::Steer { text: s("text")? },
        "Pin" => Input::Pin { tool: s("tool")? },
        "JobAccepted" => Input::JobAccepted { job_id: s("job_id")? },
        "JobDone" => Input::JobDone {
            job_id: s("job_id")?,
            output: s("output")?,
            source: s("source")?,
            ok: b("ok")?,
        },
        "Cancel" => Input::Cancel,
        _ => return None,
    })
}

/// Replay a run's log through a fresh loop, checking every batch it produces
/// against the batch the log recorded (S-2.6).
///
/// The check is what makes this a replay rather than a re-run: the loop is
/// deterministic given its construction and inputs, so a batch that differs means
/// one of the two changed — and continuing would act on a state the run never
/// had. A divergence returns the row it happened at and nothing is executed.
///
/// Returns the loop, the native transcript, and the last batch (the pending one).
pub(crate) fn replay(
    spec: &RunSpec,
    rows: &[super::resume::LogRow],
) -> Result<Replayed, String> {
    let registry = bridge::registry_with_workspace().map_err(|e| e.to_string())?;
    let mut governor = build_governor(spec, registry);
    let mut transcript: Option<harness::transcript::Transcript> = None;
    let mut last: Vec<Effect> = Vec::new();
    let mut last_logged = true;
    let mut i = 0;
    while i < rows.len() {
        let r = &rows[i];
        i += 1;
        match r.direction.as_str() {
            "state" => match r.kind.as_str() {
                "Spend" => {
                    let tokens = r.payload.get("tokens").and_then(Value::as_u64).unwrap_or(0);
                    let usd = r.payload.get("usd_tenth_cents").and_then(Value::as_i64);
                    governor.record_spend(tokens, usd);
                }
                "Opening" => {
                    let text = r
                        .payload
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    transcript = Some(harness::transcript::Transcript::new(text));
                }
                "Turn" => {
                    let turn: harness::transcript::Turn =
                        serde_json::from_value(r.payload.clone())
                            .map_err(|e| format!("transcript turn at seq {} unreadable: {e}", r.seq))?;
                    if let Some(t) = transcript.as_mut() {
                        t.push_turn(turn);
                    }
                }
                "Compact" => {
                    if let Some(t) = transcript.as_mut() {
                        t.compact(&spec.profile, 0);
                    }
                }
                "WorkingState" => {
                    let recorded = r.payload.get("hash").and_then(Value::as_str).unwrap_or("");
                    let rebuilt = state_hash(governor.working());
                    if recorded != rebuilt {
                        return Err(format!(
                            "replay diverged at seq {}: the run believed state {recorded} and this harness rebuilt {rebuilt}",
                            r.seq
                        ));
                    }
                }
                _ => {}
            },
            "input" => {
                let input = input_from_row(&r.kind, &r.payload).ok_or_else(|| {
                    format!("input {} at seq {} cannot be read back", r.kind, r.seq)
                })?;
                let produced = governor.next(input);
                // The effects the log recorded for this input are the rows that
                // follow it, up to the next input or state row.
                let recorded: Vec<&str> = rows[i..]
                    .iter()
                    .take_while(|x| x.direction == "effect")
                    .map(|x| x.kind.as_str())
                    .collect();
                let rebuilt: Vec<&str> = produced.iter().map(|e| describe_effect(e).0).collect();
                // The one legitimate gap: a crash after the input was logged and
                // before its batch was. Nothing after it ran, so the rebuilt
                // batch is simply the next thing to do — and it is not logged yet.
                let at_end = rows[i..]
                    .iter()
                    .all(|x| x.direction == "state");
                if recorded.is_empty() && at_end {
                    last = produced;
                    last_logged = false;
                    continue;
                }
                if recorded != rebuilt {
                    return Err(format!(
                        "replay diverged after seq {}: the log recorded {recorded:?} and this \
                         harness produced {rebuilt:?}",
                        r.seq
                    ));
                }
                i += recorded.len();
                last = produced;
                last_logged = true;
            }
            _ => {}
        }
    }
    Ok(Replayed {
        governor,
        transcript,
        batch: last,
        logged: last_logged,
    })
}

/// A run rebuilt from its log.
pub(crate) struct Replayed {
    pub governor: Loop,
    pub transcript: Option<harness::transcript::Transcript>,
    /// The last batch the loop produced: the run's next move.
    pub batch: Vec<Effect>,
    /// Whether that batch is already in the log (it is, unless the crash fell
    /// between logging an input and logging what it produced).
    pub logged: bool,
}

/// Resume one run from its effect log (AGENT-010 §5; S-2.6, S-2.11).
///
/// The decisions, in order, and each is the conservative one:
///
/// * a log this harness cannot replay — another version, a divergent batch, a
///   missing spec — **interrupts**, keeping the working state, and attaches a
///   suspended run's job result so the compute is not lost;
/// * an outstanding **write** is never re-run. If the durable record holds its
///   answer the model gets the answer; otherwise it gets "interrupted; not
///   retried because it writes";
/// * an outstanding read, model call or approval simply continues — the
///   approval re-attaches to the row the human may already have answered;
/// * a run suspended on a job stays suspended until the job is done.
#[allow(clippy::too_many_lines)]
pub async fn resume(
    pg: PgPool,
    client: LlmClient,
    params: ResumeParams,
    cancel: Arc<super::control::RunControl>,
) -> Resumed {
    let rows = match load_log(&pg, params.run_id).await {
        Ok(r) => r,
        Err(e) => return interrupt(&pg, params.run_id, &format!("the effect log could not be read: {e}"), params.job_done.as_ref()).await,
    };
    let decision = super::resume::decide_for_resume(&rows, HARNESS_VERSION, EFFECT_LOG_SCHEMA);
    let (inputs, pending, version_changed) = match decision {
        super::resume::Resume::Replay { inputs, pending } => {
            let changed = rows.iter().any(|r| r.harness_version != HARNESS_VERSION);
            (inputs, pending, changed)
        }
        super::resume::Resume::Interrupted { reason } => {
            return interrupt(&pg, params.run_id, &reason, params.job_done.as_ref()).await;
        }
        super::resume::Resume::Nothing { .. } => return Resumed::Ran(Ended::Finished),
    };
    let _ = inputs;

    let Some(spec) = rows
        .iter()
        .find(|r| r.direction == "state" && r.kind == "RunSpec")
        .and_then(|r| serde_json::from_value::<RunSpec>(r.payload.clone()).ok())
    else {
        return interrupt(
            &pg,
            params.run_id,
            "the log has no run spec: it was written before resume existed, so there is nothing to rebuild the loop from",
            params.job_done.as_ref(),
        )
        .await;
    };

    // Waiting on a job that is not done: nothing to do yet.
    if pending.as_ref().is_some_and(|p| p.kind == "Await") && params.job_done.is_none() {
        return Resumed::StillWaiting;
    }

    let Replayed {
        governor,
        transcript,
        batch,
        logged,
    } = match replay(&spec, &rows) {
        Ok(x) => x,
        Err(reason) => {
            return interrupt(&pg, params.run_id, &reason, params.job_done.as_ref()).await;
        }
    };

    let mut rec = Recorder::new(pg.clone(), params.run_id, params.user_id, params.exposure_out.clone());
    rec.restore(&rows).await;
    rec.retrieved = spec
        .brief
        .as_ref()
        .map(|b| b.retrievals.clone())
        .unwrap_or_default();

    let ws = workspace_tools::workspace_for(&params.workspace_root, params.conversation_id);
    let _ = workspace_tools::ensure(&ws).await;
    let session_skills = skills_mount::session_start(
        &pg,
        &ws,
        params.user_id,
        params.run_id,
        &spec.profile.model_id,
    )
    .await;
    let skill_notice_cursor = crate::skills_store::SkillStore::new(pg.clone())
        .latest_notice()
        .await
        .unwrap_or(0);
    let mcp_ctx = run_context(
        &pg,
        &params.api_base,
        &params.service_token,
        params.run_id,
        spec.demand,
        spec.workflow,
    )
    .await;

    let _ = sqlx::query(
        "UPDATE agent_runs SET status = 'running', finished_at = NULL, error = NULL WHERE run_id = $1",
    )
    .bind(params.run_id)
    .execute(&pg)
    .await;
    rec.append(
        "status",
        json!({
            "phase": "resumed",
            "from_seq": rows.last().map_or(0, |r| r.seq),
            "outstanding": pending.as_ref().map(|p| p.kind.clone()),
            "harness_changed": version_changed,
            "note": "rebuilt from the effect log; nothing already done was done again",
        }),
    )
    .await;
    if version_changed {
        // Recorded so an eval replay can see the run crossed a deploy.
        rec.event(
            "state",
            "VersionChanged",
            json!({ "to": HARNESS_VERSION, "schema": EFFECT_LOG_SCHEMA }),
        )
        .await;
    }

    let rates = rates_for(&spec);
    let native = !spec.profile.output.constrained_decoding;
    let mut pump = Pump {
        pg: pg.clone(),
        client,
        run_id: params.run_id,
        user_id: params.user_id,
        conversation_id: params.conversation_id,
        spec,
        rec,
        governor,
        transcript,
        native,
        mcp_ctx,
        ws,
        cancel,
        rates,
        last_rejection: None,
        delivered_directive: None,
        error_streak: 0,
        skill_notice_cursor,
        skills_resident: session_skills.resident.clone(),
        // A resumed run is tainted if its log shows it read a `web` tool's result.
        consumed_untrusted: rows.iter().any(|r| {
            r.kind == "ToolResult"
                && r.payload.get("source").and_then(Value::as_str) == Some("platform.web")
        }),
    };
    pump.delivered_directive = pump.governor.directive().map(str::to_string);
    pump.rec.consumed_untrusted = pump.consumed_untrusted;

    match pending {
        // The job the run was suspended on has finished: its result is the
        // result of the call that started it, arriving late.
        Some(p) if p.kind == "Await" => {
            let Some(done) = params.job_done else {
                return Resumed::StillWaiting;
            };
            let tool = p
                .payload
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if pump.transcript.is_some() {
                let call_id = pump.open_call_id(&tool, 0);
                let handle = pump.result_handle(&tool, &done.output).await;
                pump.push_turn(harness::transcript::Turn::ToolResult {
                    call_id,
                    name: tool.clone(),
                    body: done.output.clone(),
                    is_error: !done.ok,
                    handle,
                })
                .await;
            }
            pump.rec
                .append(
                    "tool_result",
                    json!({ "name": tool, "is_error": !done.ok, "content": done.output, "job_id": done.job_id }),
                )
                .await;
            let effects = pump
                .feed(Input::JobDone {
                    job_id: done.job_id,
                    output: done.output,
                    source: bridge::source_for("research"),
                    ok: done.ok,
                })
                .await;
            Resumed::Ran(pump.run(effects, false).await)
        }
        // A write that may or may not have landed. Answered from the durable
        // record if it did; never re-run.
        Some(p) if p.kind == "ExecuteTool" && !p.idempotent => {
            let name = p.payload.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let step = p.payload.get("step").and_then(Value::as_u64).and_then(|s| u32::try_from(s).ok()).unwrap_or(0);
            let args = p.payload.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let key = mcp_server_lib::idempotency::key(params.run_id, step, &name, &args);
            let input = answer_interrupted_write(&p, pump.recorded_result(&key).await);
            if pump.transcript.is_some() {
                let (body, is_error) = match &input {
                    Input::ToolResult { output, .. } => (output.clone(), false),
                    Input::ToolError { detail } => (detail.clone(), true),
                    _ => (String::new(), true),
                };
                let call_id = pump.open_call_id(&name, step);
                pump.push_turn(harness::transcript::Turn::ToolResult {
                    call_id,
                    name,
                    body,
                    is_error,
                    handle: None,
                })
                .await;
            }
            let effects = pump.feed(input).await;
            Resumed::Ran(pump.run(effects, false).await)
        }
        // A read, a model call, an approval: the rebuilt batch carries it, and the
        // pump picks up exactly there. Its rows are already in the log.
        _ => Resumed::Ran(pump.run(batch, logged).await),
    }
}

/// What answers a write the restart interrupted (S-1.19, S-2.6).
///
/// Its own result when the durable record holds one — the write landed, and the
/// model gets exactly what it would have got. Otherwise an error the model reads,
/// saying the call was not retried **because it writes**. Never a re-execution:
/// whether it landed cannot be known from here, and a second `create_experiment`
/// is a second counted trial.
pub(crate) fn answer_interrupted_write(
    p: &super::resume::PendingEffect,
    recorded: Option<Value>,
) -> Input {
    match recorded {
        Some(prev) => Input::ToolResult {
            output: match &prev {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            },
            source: bridge::source_for(
                p.payload
                    .get("namespace")
                    .and_then(Value::as_str)
                    .unwrap_or("platform"),
            ),
        },
        None => Input::ToolError {
            detail: p.interrupted_error(),
        },
    }
}

/// The whole log for one run, in order.
pub(crate) async fn load_log(
    pg: &PgPool,
    run_id: Uuid,
) -> Result<Vec<super::resume::LogRow>, sqlx::Error> {
    let rows: Vec<(i32, String, String, Value, String)> = sqlx::query_as(
        "SELECT seq, direction, kind, payload_json, harness_version
           FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
    )
    .bind(run_id)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(seq, direction, kind, payload, harness_version)| super::resume::LogRow {
            seq,
            direction,
            kind,
            payload,
            harness_version,
        })
        .collect())
}

/// Mark a run `interrupted`, honestly, keeping what it had (AGENT-010 §5).
///
/// The working state is not deleted — the log is append-only and the turn
/// summary stays — and a suspended run's job result is attached to the run's
/// timeline with its id, so the compute it bought is readable in the next turn.
async fn interrupt(pg: &PgPool, run_id: Uuid, reason: &str, job: Option<&JobDone>) -> Resumed {
    tracing::warn!(%run_id, %reason, "run not resumable; marking it interrupted");
    let next_seq: i32 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM agent_messages WHERE run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pg)
    .await
    .unwrap_or(1);
    if let Some(j) = job {
        let _ = sqlx::query(
            "INSERT INTO agent_messages (run_id, seq, kind, content_json) VALUES ($1, $2, 'job_result', $3)",
        )
        .bind(run_id)
        .bind(next_seq)
        .bind(json!({ "job_id": j.job_id, "ok": j.ok, "output": j.output,
                      "note": "the run could not resume, so its job's result is attached here rather than lost" }))
        .execute(pg)
        .await;
    }
    let _ = sqlx::query(
        "UPDATE agent_runs SET status = 'interrupted', error = $2, finished_at = now() WHERE run_id = $1",
    )
    .bind(run_id)
    .bind(format!("interrupted: {reason}"))
    .execute(pg)
    .await;
    Resumed::Interrupted(reason.to_string())
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
/// What the model was sent, by layer, in estimated tokens (S-6.16): the charter, the
/// tool schemas / grammar, the body, and the output reserve. Estimated with the same
/// estimator the assembler budgets with, so the desk's cost tab and the budget agree.
fn layer_tokens(call: &harness::drive::ModelCall) -> Value {
    let schemas = serde_json::to_string(&call.decode).unwrap_or_default();
    json!({
        "charter": harness::context::estimate_tokens(&call.system),
        "schemas": harness::context::estimate_tokens(&schemas),
        "body": harness::context::estimate_tokens(&call.prompt),
        "output": call.max_tokens,
    })
}

/// The `usage` row: provider counts, the estimate, latency, what the call cost in
/// tenth-cents (`null` when no priced row exists, never zero) and the layer sizes.
fn usage_note(
    step: u32,
    usage: &llm::Usage,
    estimated: u32,
    latency_ms: u128,
    usd_tenth_cents: Option<i64>,
    layers: Value,
) -> Value {
    json!({ "step": step, "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "prompt_tokens": usage.prompt_tokens(),
            "est_prompt_tokens": estimated,
            "cache_read_tokens": usage.cache_read_input_tokens,
            "latency_ms": u64::try_from(latency_ms).unwrap_or(u64::MAX),
            "usd_tenth_cents": usd_tenth_cents,
            "layers": layers })
}

fn build_request(
    profile: &Profile,
    call: &harness::drive::ModelCall,
    transcript: Option<&harness::transcript::Transcript>,
) -> ChatRequest {
    let (schema, tools) = match &call.decode {
        Decode::Native { tools } => (None, native_tools(tools)),
        other => (other.schema().cloned(), Vec::new()),
    };
    // The transcript is replayed only for a native tool call, and a single user
    // message carries the prompt otherwise. Written as a match on the pair rather
    // than `transcript.filter(|_| ...)`: that form reads as a predicate over the
    // transcript, and a predicate that ignores the value it is given reads as a
    // bug — it is not one, the condition really is about the *call*, and saying so
    // in the shape of the code costs nothing.
    let messages = match (transcript, &call.decode) {
        (Some(t), Decode::Native { .. }) => transcript_messages(t),
        _ => vec![Message::User {
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
                        arguments: serde_json::from_str(&c.arguments).unwrap_or_else(|_| json!({})),
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
            // A correction, a directive or a steer the harness hands a native
            // model between calls (S-2.9, S-2.14).
            Turn::User { text } => out.push(Message::User {
                content: text.clone(),
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

/// Tool results larger than this are cut before the auditor's fixture reads them.
const MAX_TOOL_RESULT_BYTES: usize = 32 * 1024;

/// Truncates an oversized tool result, marking it so a model can tell "the answer
/// is short" from "the answer was cut".
///
/// Moved here from the deleted hosted driver (S-2.10), because the auditor suite
/// (AGENT-004 §4) runs a fixture against it: a silent truncation is a leak of a
/// different kind — the model reasons over a prefix and reports as though it saw
/// the whole thing. On the loop itself the profile's `max_tool_result_bytes`
/// governs, and an over-cap body leaves a `work/` handle rather than a cut.
#[must_use]
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
        "{}\u{2026} [truncated {} of {} bytes \u{2014} request summary detail instead]",
        &text[..end],
        text.len() - end,
        text.len()
    )
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

/// How often a waiting run re-reads its approval when no notification came.
///
/// A backstop, not the mechanism: the answer arrives as a `NOTIFY` on
/// `agent_approval` (migration 0170) and wakes the wait at once. A notification
/// sent while the listener was reconnecting is not queued, so the row is re-read
/// on this interval as well — slowly, because it is only catching what the event
/// missed.
const APPROVAL_BACKSTOP: Duration = Duration::from_secs(30);

/// How often a waiting run checks whether it was cancelled.
const APPROVAL_CANCEL_TICK: Duration = Duration::from_secs(2);

/// The answer, if the approval has one: `Some(Some(..))` answered,
/// `Some(None)` cancelled, `None` still pending (or unreadable).
async fn approval_answer(pg: &PgPool, approval_id: Uuid) -> Option<Option<(bool, String)>> {
    let (state, answer): (String, Option<Value>) =
        sqlx::query_as("SELECT state, answer FROM approval_requests WHERE approval_id = $1")
            .bind(approval_id)
            .fetch_optional(pg)
            .await
            .ok()
            .flatten()?;
    match state.as_str() {
        "answered" | "defaulted" => {
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
            Some(Some((granted, note)))
        }
        "cancelled" => Some(None),
        _ => None,
    }
}

/// Persists the request (or re-attaches to it), audits the pause, and waits for a
/// human — on every tier (AGENT-010 §7; S-2.12).
///
/// * **Durable.** The row is keyed by `(run, step, tool)` and written before the
///   wait, so a restart loses the wait and not the question: a resumed run finds
///   the row it wrote — answered or not — instead of asking twice.
/// * **Audited before and after.** The pause opens an audit record for the
///   action *before* anyone is asked and closes it with what the human decided,
///   so "the agent asked to open the vault and was refused" is a row in the
///   same trail as every dispatch. A trail that cannot be written refuses the
///   action, exactly as a dispatch would be refused.
/// * **An event, not a poll.** The answer wakes the wait through `NOTIFY`.
///
/// Returns `None` if the run was cancelled while waiting.
async fn await_approval(
    pg: &PgPool,
    mcp_ctx: &McpContext,
    run_id: Uuid,
    step: u32,
    tool: &str,
    arguments: &Map<String, Value>,
    ruling: &harness::Ruling,
    cancel: &super::control::RunControl,
) -> Option<(bool, String)> {
    // Before anyone decides anything: the request is on the record.
    let audited = json!({ "approval_pause": true, "step": step, "arguments": arguments,
                          "code": ruling.code });
    let envelope = match mcp_ctx.audit().open(tool, &audited).await {
        Ok(e) => e,
        Err(e) => {
            tracing::error!(%run_id, tool, detail = ?e.detail,
                "refusing the approval pause: the platform opened no audit record");
            return Some((
                false,
                "the audit trail could not record this request, so the action is refused".into(),
            ));
        }
    };

    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT approval_id FROM approval_requests
          WHERE run_id = $1 AND payload->>'tool' = $2 AND (payload->>'step')::bigint = $3
            AND state <> 'cancelled'
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(run_id)
    .bind(tool)
    .bind(i64::from(step))
    .fetch_optional(pg)
    .await
    .ok()
    .flatten();

    let approval_id = match existing {
        Some(id) => id,
        None => {
            let id = Uuid::new_v4();
            let payload = json!({
                "tool": tool,
                "step": step,
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
            .bind(id)
            .bind(run_id)
            .bind("tool_call")
            .bind(&payload)
            .bind(json!(["approve", "refuse"]))
            // No timeout default: an unanswered destructive action stays unanswered.
            .bind(Option::<String>::None)
            .execute(pg)
            .await
            {
                tracing::error!(error = %e, "could not persist the approval request");
                mcp_ctx
                    .audit()
                    .close(
                        envelope,
                        &json!({ "error": { "code": "approval_unrecorded",
                                            "fix": "the approval request could not be stored" } }),
                    )
                    .await;
                return Some((false, "the approval could not be recorded".into()));
            }
            id
        }
    };

    let _ = sqlx::query("UPDATE agent_runs SET status = 'awaiting_approval' WHERE run_id = $1")
        .bind(run_id)
        .execute(pg)
        .await;

    // Subscribe, then read: an answer that landed between the insert and the
    // LISTEN is caught by the read, and one after it by the notification.
    let mut listener = match sqlx::postgres::PgListener::connect_with(pg).await {
        Ok(mut l) => match l.listen("agent_approval").await {
            Ok(()) => Some(l),
            Err(e) => {
                tracing::warn!(error = %e, "approval LISTEN failed; falling back to the backstop read");
                None
            }
        },
        Err(e) => {
            tracing::warn!(error = %e, "approval listener unavailable; falling back to the backstop read");
            None
        }
    };

    let mut since_read = Duration::ZERO;
    let mut answer = approval_answer(pg, approval_id).await;
    while answer.is_none() {
        if cancel.is_cancelled() {
            let _ = sqlx::query(
                "UPDATE approval_requests SET state = 'cancelled' WHERE approval_id = $1
                 AND state = 'pending'",
            )
            .bind(approval_id)
            .execute(pg)
            .await;
            answer = Some(None);
            break;
        }
        let woke = match listener.as_mut() {
            Some(l) => match tokio::time::timeout(APPROVAL_CANCEL_TICK, l.recv()).await {
                Ok(Ok(n)) => n.payload() == approval_id.to_string(),
                Ok(Err(_)) => false,
                Err(_) => false,
            },
            None => {
                tokio::time::sleep(APPROVAL_CANCEL_TICK).await;
                false
            }
        };
        since_read += APPROVAL_CANCEL_TICK;
        if woke || since_read >= APPROVAL_BACKSTOP || listener.is_none() {
            since_read = Duration::ZERO;
            answer = approval_answer(pg, approval_id).await;
        }
    }

    let answer = answer.flatten();
    // After: the human's decision, in the same record.
    let closed = match &answer {
        Some((true, note)) => json!({ "approved": true, "note": note, "approval_id": approval_id }),
        Some((false, note)) => json!({ "error": { "code": "approval_refused", "note": note,
                                                  "approval_id": approval_id,
                                                  "fix": "a human refused this action" } }),
        None => json!({ "error": { "code": "approval_cancelled", "approval_id": approval_id,
                                   "fix": "the run was stopped while waiting" } }),
    };
    mcp_ctx.audit().close(envelope, &closed).await;

    if answer.is_some() {
        let _ = sqlx::query(
            "UPDATE agent_runs SET status = 'running' WHERE run_id = $1
             AND status = 'awaiting_approval'",
        )
        .bind(run_id)
        .execute(pg)
        .await;
    }
    answer
}

#[cfg(test)]
mod tests {
    /// S-6.12 / S-6.16: a usage note carries what the call cost and the layer sizes,
    /// and a call with no priced row says `null`, not zero.
    #[test]
    fn a_usage_note_carries_the_cost_and_the_layers() {
        let call = harness::drive::ModelCall {
            step: 3,
            system: "c".repeat(400),
            prompt: "b".repeat(800),
            decode: Decode::Native { tools: vec![] },
            temperature: 0.0,
            max_tokens: 512,
            num_ctx: 8192,
        };
        let layers = layer_tokens(&call);
        assert_eq!(layers["charter"], 100);
        assert_eq!(layers["body"], 200);
        assert_eq!(layers["output"], 512);
        assert!(layers["schemas"].as_u64().is_some());
        let u = llm::Usage { input_tokens: 10, output_tokens: 5, ..Default::default() };
        let n = usage_note(3, &u, 300, 42, Some(7), layers.clone());
        assert_eq!(n["usd_tenth_cents"], 7);
        assert_eq!(n["layers"], layers);
        assert!(usage_note(3, &u, 300, 42, None, layers)["usd_tenth_cents"].is_null());
    }

    use super::*;
    use harness::profile::ProfileSet;

    fn profile() -> Profile {
        ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
            .expect("the shipped profiles load")
            .get("qwen3.6-35b-a3b")
            .expect("the reference profile is shipped")
            .clone()
    }

    /// The dossier's trial count came from a field assigned thirty lines after
    /// the call that read it, so **every dossier ever compiled recorded zero
    /// trials** — for the one number this platform exists to count.
    ///
    /// Asserted structurally rather than behaviourally, because the behaviour
    /// needs a database, a run and a finished outcome, and the property is about
    /// *which of two copies of one number* a line reads. There are two by
    /// design: `Recorder::trials_spent` is the counter and `Yield::trials_spent`
    /// is the copy the run record carries, and the yield's own doc comment says
    /// it "reads the same number rather than keeping its own". The rule is that
    /// anything running before `finish` writes the row must read the counter.
    ///
    /// This does not re-order the statements. An ordering that has to be
    /// maintained is one that drifts back the first time somebody moves a call.
    #[test]
    fn the_dossier_reads_the_trial_counter_and_not_the_copy_of_it() {
        const SRC: &str = include_str!("local_driver.rs");
        let body = SRC
            .split_once("async fn compile_dossier")
            .expect("compile_dossier exists")
            .1
            .split_once("\n    }")
            .expect("it ends")
            .0;
        // Comment lines are excluded: the line that reads the counter explains
        // why it does not read the copy, and *names the copy* to do it. A scan
        // that read its own explanation would fail on the fix it is checking
        // for - which is what happened the first time this ran.
        let code: String = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code.contains("yielded.trials_spent"),
            "`compile_dossier` reads `yielded.trials_spent`, which `finish` assigns after \
             calling it — so the dossier records zero trials for every run"
        );
        assert!(
            code.contains("self.trials_spent"),
            "`compile_dossier` reads neither the counter nor the copy; it has stopped \
             recording a trial count at all"
        );
    }

    /// The yield reaches the card that reports it.
    ///
    /// The counts have been columns on `agent_runs` since S-10.13 and the desk's
    /// Report card had a headline and a button — so "what did this run produce"
    /// was answerable from the database and not from the conversation that
    /// produced it. The event is the hop that was missing.
    #[test]
    fn the_dossier_event_carries_what_the_run_produced() {
        const SRC: &str = include_str!("local_driver.rs");
        let body = SRC
            .split_once("fn yield_payload")
            .expect("yield_payload exists")
            .1
            .split_once("\n    }")
            .expect("it ends")
            .0;
        // Every count the row carries, so the card and the row cannot disagree
        // about what a run produced.
        for field in [
            "findings_established",
            "findings_suggestive",
            "findings_refuted",
            "negatives",
            "negatives_new_class",
            "questions_opened",
            "questions_closed",
            "contradictions_surfaced",
            "proposals",
            "hypotheses_tested",
            "hypotheses_refuted",
            "hypotheses_supported",
            "trials_spent",
        ] {
            assert!(
                body.contains(field),
                "the yield payload omits `{field}`, so the Report card and `agent_runs` \
                 disagree about what the run produced"
            );
        }
    }


    // ── Resume (S-2.6) ──────────────────────────────────────────────────────

    fn frontier_spec(goal: &str) -> RunSpec {
        let profile = ProfileSet::load_dir(std::path::Path::new("../../config/profiles"))
            .expect("the shipped profiles load")
            .get("claude-opus-5")
            .expect("the frontier profile is shipped")
            .clone();
        RunSpec {
            goal: goal.into(),
            charter: "You are a research agent.".into(),
            prior: String::new(),
            constraints: json!({}),
            profile,
            namespaces: vec!["core".into(), "discovery".into(), "research".into()],
            workflow: harness::state::Workflow::Investigate,
            demand: TaskDemand::MultiStep,
            prior_exposure: Vec::new(),
            brief: None,
            usd_cap_tenth_cents: None,
            priced_on: None,
            cost_basis: None,
            resumed_state: None,
            max_trials: None,
            bytes_per_token: None,
        }
    }

    fn row(seq: i32, direction: &str, kind: &str, payload: Value) -> super::super::resume::LogRow {
        super::super::resume::LogRow {
            seq,
            direction: direction.into(),
            kind: kind.into(),
            payload,
            harness_version: HARNESS_VERSION.into(),
        }
    }

    /// Drive a loop over scripted inputs and write the log exactly as the pump
    /// does: the spec, then for each input a `WorkingState` row, the input, and
    /// the batch it produced. Returns the rows and the live loop.
    fn scripted(spec: &RunSpec, inputs: Vec<Input>) -> (Vec<super::super::resume::LogRow>, Loop) {
        let mut l = build_governor(spec, bridge::registry_with_workspace().unwrap());
        let mut rows = vec![row(1, "state", "RunSpec", serde_json::to_value(spec).unwrap())];
        for input in inputs {
            let n = i32::try_from(rows.len()).unwrap() + 1;
            rows.push(row(
                n,
                "state",
                "WorkingState",
                json!({ "step": l.step(), "hash": state_hash(l.working()) }),
            ));
            let (kind, payload) = describe_input(&input);
            rows.push(row(n + 1, "input", kind, payload));
            for e in l.next(input) {
                let (kind, payload) = describe_effect(&e);
                let n = i32::try_from(rows.len()).unwrap() + 1;
                rows.push(row(n, "effect", kind, payload));
            }
        }
        (rows, l)
    }

    fn experiment_call() -> String {
        json!({
            "name": "create_experiment",
            "arguments": {
                "experiment_id": "ema_cross_btc_1h",
                "strategy_family": "ema_cross",
                "strategy_type": "intraday_momentum",
                "strategy_ref": "ema_cross_v1",
                "universe_ref": "BTC-USD",
                "research_start": "2024-01-01",
                "research_end": "2025-01-01",
                "holdout_start": "2025-01-01",
                "holdout_end": "2025-06-01",
                "objective": { "metric": "sharpe" }
            }
        })
        .to_string()
    }

    fn executes(fx: &[Effect], tool: &str) -> usize {
        fx.iter()
            .filter(|e| matches!(e, Effect::ExecuteTool { name, .. } if name == tool))
            .count()
    }

    /// AGENT-010 §14.3, without a process to kill: a run that died after
    /// deciding `create_experiment` and before hearing back is replayed from its
    /// log and resumes **without** a second `create_experiment` — the model is
    /// told the write was interrupted, or handed its recorded result.
    #[test]
    fn a_resumed_run_does_not_repeat_an_interrupted_create_experiment() {
        let spec = frontier_spec("create an experiment for an ema cross on BTC-USD at 1h");
        let (rows, _) = scripted(
            &spec,
            vec![
                Input::Admission(hardware::Admission::Admit),
                Input::ModelReply {
                    raw: experiment_call(),
                },
                // An unattended write asks first; the human said yes, and the
                // run died with the write decided and not yet answered.
                Input::Approval {
                    granted: true,
                    note: String::new(),
                },
            ],
        );
        let before: usize = rows
            .iter()
            .filter(|r| r.kind == "ExecuteTool" && r.payload["name"] == "create_experiment")
            .count();
        assert_eq!(before, 1, "the run decided the write once before it died: {rows:?}");

        let super::super::resume::Resume::Replay { pending, .. } =
            super::super::resume::decide(&rows, HARNESS_VERSION)
        else {
            panic!("a run with a trailing write resumes");
        };
        let pending = pending.expect("the write is outstanding");
        assert_eq!(pending.kind, "ExecuteTool");
        assert!(!pending.idempotent, "create_experiment writes");

        // No durable record: the model is told, the write is not repeated.
        let mut replayed = replay(&spec, &rows).expect("the log replays");
        assert!(replayed.logged, "the trailing batch is already in the log");
        let input = answer_interrupted_write(&pending, None);
        assert!(matches!(&input, Input::ToolError { detail } if detail.contains("because it writes")));
        let after = replayed.governor.next(input);
        assert_eq!(executes(&after, "create_experiment"), 0, "{after:?}");

        // A durable record: the model gets the original answer, still no repeat.
        let mut replayed = replay(&spec, &rows).expect("the log replays");
        let input = answer_interrupted_write(
            &pending,
            Some(json!({ "experiment_id": "ema_cross_btc_1h", "state": "created" })),
        );
        assert!(matches!(&input, Input::ToolResult { output, .. } if output.contains("created")));
        let after = replayed.governor.next(input);
        assert_eq!(executes(&after, "create_experiment"), 0, "{after:?}");
    }

    /// The replay is a replay: a log whose recorded state disagrees with what
    /// this harness rebuilds is refused, not resumed (S-2.6).
    #[test]
    fn a_log_that_does_not_replay_to_the_same_state_is_refused() {
        let spec = frontier_spec("create an experiment for an ema cross on BTC-USD at 1h");
        let (mut rows, _) = scripted(
            &spec,
            vec![
                Input::Admission(hardware::Admission::Admit),
                Input::ModelReply {
                    raw: experiment_call(),
                },
            ],
        );
        let tampered = rows
            .iter_mut()
            .rev()
            .find(|r| r.kind == "WorkingState")
            .unwrap();
        tampered.payload["hash"] = json!("not-the-state");
        let err = replay(&spec, &rows).err().expect("a divergent state is refused");
        assert!(err.contains("diverged"), "{err}");

        // And an effect the log recorded that this harness would not produce.
        let (mut rows, _) = scripted(&spec, vec![Input::Admission(hardware::Admission::Admit)]);
        let last = rows.iter_mut().rev().find(|r| r.direction == "effect").unwrap();
        last.kind = "ExecuteTool".into();
        assert!(replay(&spec, &rows).is_err());
    }

    /// A crash between logging an input and logging its batch leaves a log that
    /// ends on the input. The rebuilt batch is the next move, marked unlogged.
    #[test]
    fn a_log_ending_on_an_input_replays_to_an_unlogged_batch() {
        let spec = frontier_spec("what instruments do you have data for");
        let (mut rows, _) = scripted(&spec, vec![Input::Admission(hardware::Admission::Admit)]);
        rows.retain(|r| r.direction != "effect");
        let replayed = replay(&spec, &rows).expect("replays");
        assert!(!replayed.logged);
        assert!(replayed
            .batch
            .iter()
            .any(|e| matches!(e, Effect::CallModel(_))));
    }

    /// Every input the pump can feed is read back by resume. A kind written and
    /// never readable would make every run that used it unresumable.
    #[test]
    fn every_input_the_pump_writes_reads_back() {
        for input in [
            Input::Admission(hardware::Admission::Admit),
            Input::ModelReply { raw: "{}".into() },
            Input::ModelTruncated { raw: "{".into() },
            Input::ModelError {
                detail: "x".into(),
                retryable: true,
            },
            Input::ToolResult {
                output: "o".into(),
                source: "platform.data".into(),
            },
            Input::ToolError { detail: "e".into() },
            Input::Approval {
                granted: true,
                note: "ok".into(),
            },
            Input::Steer { text: "only 1h".into() },
            Input::Pin {
                tool: "get_bars".into(),
            },
            Input::JobAccepted {
                job_id: "sweep:1".into(),
            },
            Input::JobDone {
                job_id: "sweep:1".into(),
                output: "{}".into(),
                source: "platform.research".into(),
                ok: true,
            },
            Input::Cancel,
        ] {
            let (kind, payload) = describe_input(&input);
            let back = input_from_row(kind, &payload)
                .unwrap_or_else(|| panic!("{kind} does not read back from {payload}"));
            assert_eq!(describe_input(&back), (kind, payload));
        }
    }

    /// A started campaign suspends the parent on a handle the watcher can read;
    /// a refused or malformed one does not (S-10.11).
    #[test]
    fn a_long_tool_still_running_names_the_job_the_watcher_polls() {
        let none = Map::new();
        let training = json!({"status": "still_running", "model_id": "m1", "run_id": "r9"});
        assert_eq!(long_job_handle("train_model", &none, &training).as_deref(), Some("training:m1:r9"));
        // A finished training run is a result, not a handle.
        let done = json!({"status": "succeeded", "model_id": "m1", "run_id": "r9"});
        assert_eq!(long_job_handle("train_model", &none, &done), None);
        let study = json!({"detached": true, "experiment_id": "e1", "study_id": "s2", "status": "running"});
        assert_eq!(long_job_handle("run_study", &none, &study).as_deref(), Some("study:e1:s2"));
        assert_eq!(long_job_handle("run_study", &none, &json!({"study_id": "s2"})), None);
        let sweep = json!({"timed_out": true, "sweep_id": "w"});
        assert_eq!(long_job_handle("run_sweep", &none, &sweep).as_deref(), Some("sweep:w"));
        assert_eq!(long_job_handle("run_sweep", &none, &json!({"sweep_id": "w"})), None);
        let mut args = Map::new();
        args.insert("backtest_id".into(), json!("b1"));
        assert_eq!(long_job_handle("wait_for_backtest", &args, &json!({"timed_out": true})).as_deref(), Some("backtest:b1"));
        // A tool that is not long never suspends a run.
        assert_eq!(long_job_handle("get_bars", &none, &training), None);
    }

    #[test]
    fn only_a_started_campaign_suspends_its_run() {
        let ok = json!({"campaign_id": "7d1c", "job_id": "job_1"});
        assert_eq!(campaign_handle("define_campaign", false, &ok).as_deref(), Some("campaign:7d1c"));
        assert_eq!(campaign_handle("define_campaign", true, &ok), None, "an error started nothing");
        assert_eq!(campaign_handle("get_campaign", false, &ok), None, "a read is not a start");
        assert_eq!(campaign_handle("define_campaign", false, &json!({"error": "x"})), None);
        assert_eq!(campaign_handle("define_campaign", false, &json!({"campaign_id": " "})), None);
    }

    /// A rate limit waits what the provider asked, capped; anything else backs
    /// off exponentially from two seconds, capped at two minutes.
    #[test]
    fn a_retryable_error_backs_off_and_honours_retry_after() {
        assert_eq!(
            backoff(
                &LlmError::RateLimited {
                    retry_after_secs: Some(7)
                },
                0
            ),
            Duration::from_secs(7)
        );
        assert_eq!(
            backoff(&LlmError::Network("reset".into()), 0),
            Duration::from_secs(2)
        );
        assert_eq!(
            backoff(&LlmError::Network("reset".into()), 1),
            Duration::from_secs(8)
        );
        assert_eq!(
            backoff(&LlmError::Network("reset".into()), 9),
            Duration::from_secs(120)
        );
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

    // ── The database half of resume (S-2.6; S-2.11), against a migrated Postgres ──
    //
    // Ignored by default. A scratch database migrated through the latest
    // migration (pgvector columns substituted on the native 5433 server):
    //
    //   DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/s_runtime_scratch \
    //     cargo test -j 2 -p api --lib resume_pg -- --ignored --test-threads=1
    //
    // The unit tests above replay a truncated log in memory. What they cannot reach is
    // `load_log`, `interrupt`, the idempotency read and the status writes - the seam a
    // restart actually crosses. These put a log in `agent_effect_log`, hand `resume` a
    // fake provider (the Anthropic wire, over a local socket) and a fake platform that
    // counts what is asked of it, and assert on what is left in the database.

    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fakes {
        provider_base: String,
        platform_base: String,
        /// Request bodies the provider received.
        provider_bodies: Arc<std::sync::Mutex<Vec<Value>>>,
        /// Paths the platform was asked for.
        platform_paths: Arc<std::sync::Mutex<Vec<String>>>,
        provider_calls: Arc<AtomicUsize>,
    }

    async fn fakes() -> Fakes {
        use axum::{routing::post, Json, Router};
        let bodies = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (b, c) = (Arc::clone(&bodies), Arc::clone(&calls));
        let provider = Router::new().route(
            "/v1/messages",
            post(move |Json(body): Json<Value>| {
                let (b, c) = (Arc::clone(&b), Arc::clone(&c));
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    b.lock().unwrap().push(body);
                    Json(json!({
                        "id": "msg", "type": "message", "role": "assistant",
                        "model": "claude-opus-5",
                        "content": [{ "type": "text", "text": "Understood; stopping here." }],
                        "stop_reason": "end_turn",
                        "usage": { "input_tokens": 100, "output_tokens": 10 }
                    }))
                }
            }),
        );
        let paths = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let p = Arc::clone(&paths);
        let platform = Router::new().fallback(move |uri: axum::http::Uri| {
            let p = Arc::clone(&p);
            async move {
                p.lock().unwrap().push(uri.path().to_string());
                Json(json!({}))
            }
        });
        let serve = |app: Router| async move {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", l.local_addr().unwrap());
            tokio::spawn(async move {
                let _ = axum::serve(l, app).await;
            });
            base
        };
        Fakes {
            provider_base: serve(provider).await,
            platform_base: serve(platform).await,
            provider_bodies: bodies,
            platform_paths: paths,
            provider_calls: calls,
        }
    }

    async fn scratch() -> PgPool {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://postgres:postgres@127.0.0.1:5433/s_runtime_scratch".into()
        });
        PgPool::connect(&url).await.expect("the scratch database is up")
    }

    /// A user, a conversation and an `agent_runs` row, and the rows of a log
    /// written under it as `harness_version`.
    async fn a_logged_run(
        pg: &PgPool,
        rows: &[super::super::resume::LogRow],
        version: &str,
    ) -> (Uuid, Uuid, Uuid) {
        let user = Uuid::new_v4();
        sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
            .bind(user)
            .bind(format!("test-fixture-resume-{user}@test.invalid"))
            .execute(pg)
            .await
            .unwrap();
        let convo = super::super::conversations::create(pg, user, "anthropic", "claude-opus-5")
            .await
            .unwrap();
        let run = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model, conversation_id)
             VALUES ($1, $2, 'running', 'test-fixture:resume', 'anthropic', 'claude-opus-5', $3)",
        )
        .bind(run)
        .bind(user)
        .bind(convo)
        .execute(pg)
        .await
        .unwrap();
        for r in rows {
            sqlx::query(
                "INSERT INTO agent_effect_log (run_id, seq, direction, kind, payload_json, harness_version)
                 VALUES ($1, $2, $3, $4, $5, $6)",
            )
            .bind(run)
            .bind(r.seq)
            .bind(&r.direction)
            .bind(&r.kind)
            .bind(&r.payload)
            .bind(version)
            .execute(pg)
            .await
            .unwrap();
        }
        (user, convo, run)
    }

    fn params(f: &Fakes, user: Uuid, convo: Uuid, run: Uuid, job: Option<JobDone>) -> ResumeParams {
        ResumeParams {
            run_id: run,
            user_id: user,
            conversation_id: convo,
            workspace_root: std::env::temp_dir().join("s_runtime_resume_ws"),
            api_base: f.platform_base.clone(),
            service_token: "test-token".into(),
            exposure_out: Arc::new(std::sync::Mutex::new(Vec::new())),
            job_done: job,
        }
    }

    fn client(f: &Fakes) -> LlmClient {
        LlmClient::new(
            llm::Provider::Anthropic,
            Some("sk-test-fixture".into()),
            Some(f.provider_base.clone()),
        )
    }

    fn control() -> Arc<super::super::control::RunControl> {
        Arc::new(super::super::control::RunControl::new())
    }

    /// The write the run died inside, as the log has it.
    fn a_log_cut_after_the_approved_write() -> (RunSpec, Vec<super::super::resume::LogRow>) {
        let spec = frontier_spec("create an experiment for an ema cross on BTC-USD at 1h");
        let (rows, _) = scripted(
            &spec,
            vec![
                Input::Admission(hardware::Admission::Admit),
                Input::ModelReply {
                    raw: experiment_call(),
                },
                Input::Approval {
                    granted: true,
                    note: String::new(),
                },
            ],
        );
        (spec, rows)
    }

    /// AGENT-010 §14.3 against a database: the log is read from `agent_effect_log`,
    /// the interrupted write is not run, the model is told so, and no request for
    /// the write reaches the platform.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs DATABASE_URL: a migrated scratch database"]
    async fn resume_pg_an_interrupted_write_is_not_repeated_and_the_model_is_told() {
        let pg = scratch().await;
        let f = fakes().await;
        let (_, rows) = a_log_cut_after_the_approved_write();
        let (user, convo, run) = a_logged_run(&pg, &rows, HARNESS_VERSION).await;

        let out = resume(pg.clone(), client(&f), params(&f, user, convo, run, None), control()).await;
        assert!(matches!(out, Resumed::Ran(_)), "{out:?}");

        let sent = f.provider_bodies.lock().unwrap().clone();
        assert!(!sent.is_empty(), "the model was asked what to do about the interruption");
        let all = serde_json::to_string(&sent).unwrap();
        assert!(
            all.contains("because it writes"),
            "the model was not told the write was interrupted: {all}"
        );
        let paths = f.platform_paths.lock().unwrap().clone();
        assert!(
            !paths.iter().any(|p| p.contains("experiments")),
            "the interrupted create_experiment reached the platform again: {paths:?}"
        );
        let resumed: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM agent_messages WHERE run_id = $1 AND kind = 'status'
                AND content_json->>'phase' = 'resumed'",
        )
        .bind(run)
        .fetch_one(&pg)
        .await
        .unwrap();
        assert_eq!(resumed, 1);
    }

    /// The same cut, but the write had landed and its answer was recorded durably
    /// (`agent_idempotency`): the model gets the original result, still no repeat.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs DATABASE_URL: a migrated scratch database"]
    async fn resume_pg_a_write_that_landed_is_answered_from_the_durable_record() {
        let pg = scratch().await;
        let f = fakes().await;
        let (_, rows) = a_log_cut_after_the_approved_write();
        let (user, convo, run) = a_logged_run(&pg, &rows, HARNESS_VERSION).await;
        let pending = rows.iter().rev().find(|r| r.kind == "ExecuteTool").unwrap();
        let name = pending.payload["name"].as_str().unwrap();
        let step = u32::try_from(pending.payload["step"].as_u64().unwrap_or(0)).unwrap();
        let key = mcp_server_lib::idempotency::key(run, step, name, &pending.payload["arguments"]);
        sqlx::query(
            "INSERT INTO agent_idempotency (run_id, key, tool, result) VALUES ($1, $2, $3, $4)",
        )
        .bind(run)
        .bind(&key)
        .bind(name)
        .bind(json!({ "experiment_id": "ema_cross_btc_1h", "state": "created-before-the-crash" }))
        .execute(&pg)
        .await
        .unwrap();

        let out = resume(pg.clone(), client(&f), params(&f, user, convo, run, None), control()).await;
        assert!(matches!(out, Resumed::Ran(_)), "{out:?}");
        let all = serde_json::to_string(&*f.provider_bodies.lock().unwrap()).unwrap();
        assert!(
            all.contains("created-before-the-crash"),
            "the recorded answer did not reach the model: {all}"
        );
        assert!(!all.contains("because it writes"));
        assert!(!f.platform_paths.lock().unwrap().iter().any(|p| p.contains("experiments")));
    }

    /// A mid-step run written by another harness version is interrupted, with its
    /// log intact; nothing is asked of the model or the platform.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs DATABASE_URL: a migrated scratch database"]
    async fn resume_pg_a_mid_step_run_under_another_harness_is_interrupted_in_the_database() {
        let pg = scratch().await;
        let f = fakes().await;
        let (_, rows) = a_log_cut_after_the_approved_write();
        let (user, convo, run) = a_logged_run(&pg, &rows, "0.0.0-older").await;
        let out = resume(pg.clone(), client(&f), params(&f, user, convo, run, None), control()).await;
        assert!(matches!(out, Resumed::Interrupted(_)), "{out:?}");
        let (status, error): (String, Option<String>) =
            sqlx::query_as("SELECT status, error FROM agent_runs WHERE run_id = $1")
                .bind(run)
                .fetch_one(&pg)
                .await
                .unwrap();
        assert_eq!(status, "interrupted");
        assert!(error.unwrap_or_default().starts_with("interrupted:"));
        assert_eq!(f.provider_calls.load(Ordering::SeqCst), 0, "an interrupted run asked the model");
        let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_effect_log WHERE run_id = $1")
            .bind(run)
            .fetch_one(&pg)
            .await
            .unwrap();
        assert_eq!(kept as usize, rows.len(), "the log is the evidence and it is kept");
    }

    /// A log with no `RunSpec` (written before resume existed) cannot be rebuilt:
    /// interrupted, and a suspended run's job result is attached rather than lost.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs DATABASE_URL: a migrated scratch database"]
    async fn resume_pg_an_incompatible_log_attaches_the_job_result() {
        let pg = scratch().await;
        let f = fakes().await;
        let rows = vec![row(
            1,
            "effect",
            "Await",
            json!({ "step": 3, "job_id": "sweep:7", "tool": "run_sweep" }),
        )];
        let (user, convo, run) = a_logged_run(&pg, &rows, HARNESS_VERSION).await;
        let done = JobDone {
            job_id: "sweep:7".into(),
            output: r#"{"status":"completed","best":1.4}"#.into(),
            ok: true,
        };
        let out = resume(pg.clone(), client(&f), params(&f, user, convo, run, Some(done)), control()).await;
        assert!(matches!(out, Resumed::Interrupted(_)), "{out:?}");
        let attached: Option<Value> = sqlx::query_scalar(
            "SELECT content_json FROM agent_messages WHERE run_id = $1 AND kind = 'job_result'",
        )
        .bind(run)
        .fetch_optional(&pg)
        .await
        .unwrap();
        let attached = attached.expect("the job result was attached to the run, not lost");
        assert_eq!(attached["job_id"], "sweep:7");
        assert!(attached["output"].as_str().unwrap().contains("1.4"));
    }

    /// A suspended run waiting on a job that has not finished stays waiting, and
    /// once the job is done it resumes - across a harness version change, because
    /// the effect-log schema is unchanged.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs DATABASE_URL: a migrated scratch database"]
    async fn resume_pg_a_suspended_run_resumes_on_job_done_across_a_version_change() {
        let pg = scratch().await;
        let f = fakes().await;
        let spec = frontier_spec("sweep the ema cross");
        // Built by driving the loop itself, so the log is exactly what the pump
        // would have written: the sweep call, its approval, the handle, the wait.
        // `run_sweep` is not exposed until a step asks for it, so the call that starts
        // the job is one that is: what the loop does with a handle does not depend
        // on which tool produced it.
        let sweep = experiment_call();
        let (mut rows, _) = scripted(
            &spec,
            vec![
                Input::Admission(hardware::Admission::Admit),
                Input::ModelReply { raw: sweep },
                Input::Approval { granted: true, note: String::new() },
                Input::JobAccepted { job_id: "sweep:7".into() },
            ],
        );
        assert!(
            rows.iter().any(|r| r.kind == "Await"),
            "the loop did not suspend on the handle: {:?}",
            rows.iter().filter(|r| r.kind == "Note").map(|r| r.payload.to_string().chars().take(300).collect::<String>()).collect::<Vec<_>>()
        );
        let n = i32::try_from(rows.len()).unwrap();
        rows.push(row(
            n + 1,
            "state",
            "Suspended",
            json!({ "job_id": "sweep:7", "schema": EFFECT_LOG_SCHEMA }),
        ));
        let (user, convo, run) = a_logged_run(&pg, &rows, "0.0.0-older").await;

        let still = resume(pg.clone(), client(&f), params(&f, user, convo, run, None), control()).await;
        assert!(matches!(still, Resumed::StillWaiting), "{still:?}");
        assert_eq!(f.provider_calls.load(Ordering::SeqCst), 0, "waiting costs no model call");

        let done = JobDone {
            job_id: "sweep:7".into(),
            output: r#"{"status":"completed"}"#.into(),
            ok: true,
        };
        let out = resume(pg.clone(), client(&f), params(&f, user, convo, run, Some(done)), control()).await;
        assert!(matches!(out, Resumed::Ran(_)), "{out:?}");
        assert!(f.provider_calls.load(Ordering::SeqCst) >= 1, "the run thought about the result");
        let all = serde_json::to_string(&*f.provider_bodies.lock().unwrap()).unwrap();
        assert!(all.contains("completed"), "the job result reached the model: {all}");
    }
}
