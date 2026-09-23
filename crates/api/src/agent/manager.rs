//! Agent run lifecycle: capacity, run rows, run-scoped tokens, spawn, and the
//! run controls — cancel, and the bounded pause/resume of ADR-D-27.
//!
//! Follows the platform's job pattern (ModelManager::start_train): insert a
//! status row, `tokio::spawn` a driver that updates it as it goes, poll over
//! REST. Runs do not survive a restart — `recover_orphans` marks them failed.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use cfg::model::AgentConfig;
use llm::{LlmClient, Provider};
use mcp_server_lib::ApiClient;

use crate::credentials::LlmCredentialStore;

use super::control::{PauseRefused, PauseWindow, ResumeRefused, RunControl};
use super::conversations;
use super::driver::{self, DriverParams};
use super::local_driver::{self, LocalRunParams};
use super::prompt;

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("no stored credential for provider '{0}' — configure it in Settings first")]
    NotConfigured(String),
    #[error("agent is at capacity — wait for a run to finish")]
    Busy,
    /// The platform is shutting down for a deploy (AGENT-010 §10.1, S-2.15).
    ///
    /// Separate from `Busy` because the two call for different things from the
    /// caller and from the desk: `Busy` means try again when something finishes,
    /// and this means try again in about a minute. A desk that cannot tell them
    /// apart shows "at capacity" through a deploy and teaches the user that the
    /// message means nothing.
    #[error("the platform is restarting for a deploy; try again in about {seconds}s")]
    Draining { seconds: u64 },
    #[error("internal error: {0}")]
    Internal(String),
}

/// How long a deploy waits for in-flight turns before giving up on a clean stop.
///
/// Thirty seconds, which is a long time for "one model call plus one tool call" and
/// a short time for a deploy. Longer would hold a rolling restart on a run that is
/// stuck rather than working; shorter would cut steps that were about to finish.
const DRAIN_DEADLINE_SECS: u64 = 30;

/// Where agent workspaces live on disk.
///
/// One directory per conversation underneath. Configurable because a deployment may
/// want it on a different volume from the binary; defaulted because nobody should
/// have to set an env var to make the agent able to take notes.
#[must_use]
pub fn workspace_root() -> std::path::PathBuf {
    std::env::var("TBOT_AGENT_WORKSPACE_ROOT").map_or_else(
        |_| std::path::PathBuf::from("agent-workspaces"),
        std::path::PathBuf::from,
    )
}

pub struct AgentManager {
    pg: PgPool,
    /// Capability profiles (ADR-0031). A run whose model resolves to a local tier is
    /// driven by the GOVERNOR loop; one that resolves to nothing keeps the legacy
    /// path, so adding a profile is what opts a model in.
    profiles: Arc<harness::ProfileSet>,
    /// Loopback base URL of this platform's own API (tool execution path).
    base_url: String,
    cfg: AgentConfig,
    /// The control handle for every run this process is driving.
    ///
    /// This used to be an `Arc<AtomicBool>` per run — the cancel flag, and
    /// nothing else. It is a [`RunControl`] because pause had to be built *on*
    /// the cancel mechanism rather than beside it: the drivers already poll this
    /// map's value at their step boundaries, so making the value able to hold
    /// them there is the difference between a pause the loop obeys and a pause
    /// that is a column.
    ///
    /// Membership of this map is also the honest answer to "can this run be
    /// paused?". A run this process is not driving has no loop to hold, so
    /// [`Self::pause`] refuses it rather than recording a state nothing honours.
    running: Mutex<HashMap<Uuid, Arc<RunControl>>>,
    /// Set on `SIGTERM`: no new turns, and the ones in flight get to finish
    /// (§10.1, S-2.15).
    ///
    /// Resume makes a *crash* survivable; a deploy is not a crash, and treating it
    /// as one throws away work that did not need to be lost. A step is one model
    /// call plus one tool call — seconds — so waiting for a boundary costs a status
    /// line rather than a turn.
    draining: AtomicBool,
    /// One prefix ledger per conversation (AGENT-005 §21; S-9.9).
    ///
    /// Every turn used to re-assemble from scratch, so the provider cache was
    /// cold at the start of every turn even though the charter and most of the
    /// tool slice were identical. Ten turns paid ten cold prefixes where one is
    /// correct. This is what carries the slice across the boundary.
    ///
    /// In memory rather than in Postgres, deliberately: it is a cache-shaped
    /// optimisation, a restart costs one cold prefix per live conversation, and
    /// a persisted copy would be a second thing that can disagree with what the
    /// provider actually has.
    prefixes: Mutex<HashMap<Uuid, harness::context::prefix::Prefix>>,
}

/// What the conversation front door asks the tier for.
///
/// `MultiStep` rather than `SingleLookup`, and the two are not the same
/// question: `TaskDemand` is a statement about the *tier* — can this run take
/// more than one step — and `Mode` is a statement about the *product*. They are
/// derived from each other here only because nothing yet sets the mode
/// independently (S-10.2).
const CHAT_DEMAND: harness::hardware::TaskDemand = harness::hardware::TaskDemand::MultiStep;

/// States a run can be in once it is over.
///
/// The complement of [`conversations::ACTIVE_RUN_STATES`] over the words the
/// drivers actually write, and it is longer than the obvious five. The local
/// driver also ends a run `suspended` (waiting on a job it cannot yet resume)
/// and `offered` (the turn ended asking for a Research run) — both with
/// `finished_at` set. A list that stopped at the five would leave those two in
/// neither list, and every caller keyed on "is this over" would answer *no*
/// about a run that has been over for a week.
///
/// That is the `ended == "succeeded"` defect in its other direction — a branch
/// keyed on a set of words the column does not actually take — and
/// `the_terminal_list_holds_every_status_the_drivers_write` is what keeps this
/// list honest as the drivers change.
pub const TERMINAL_RUN_STATES: &[&str] = &[
    "completed",
    "failed",
    "cancelled",
    "fenced",
    "refused",
    "suspended",
    "offered",
];

/// How an enum serialises, which is what the column stores.
///
/// Taken from the enum rather than written out, so a row cannot carry a word
/// `0126`'s CHECK refuses or one nothing reads back.
fn enum_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|j| j.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl AgentManager {
    pub fn new(pg: PgPool, base_url: String, cfg: AgentConfig) -> Self {
        Self {
            pg,
            // The same loader `AppState` uses, not a second one. Two loaders would be
            // two answers to "which profile is this model?", and the disagreement
            // would surface as a run executing on a tier the UI says it is not.
            profiles: crate::state::load_profiles(),
            base_url,
            cfg,
            running: Mutex::new(HashMap::new()),
            draining: AtomicBool::new(false),
            prefixes: Mutex::new(HashMap::new()),
        }
    }

    /// Overrides the profile set. For tests, and for a caller that has already
    /// loaded one.
    #[must_use]
    pub fn with_profiles(mut self, profiles: Arc<harness::ProfileSet>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Classify runs orphaned by a previous process (AGENT-010 §5, S-2.6).
    ///
    /// The policy used to be the same as backtests': everything unfinished becomes
    /// `failed`. That is honest and expensive — a forty-minute sweep nineteen minutes
    /// in loses nineteen minutes, and the model's findings go with it.
    ///
    /// The effect log (S-2.5) makes a better answer possible, and this reports what
    /// it would be for each orphan before doing anything. **Still marking them
    /// failed**, deliberately: replaying inputs through a fresh `Loop` needs the
    /// driver to accept a pre-seeded loop, which it does not yet, and a recovery that
    /// half-works is worse than one that says it did not.
    ///
    /// So this is the diagnosis without the treatment, and it is worth having on its
    /// own: the log says which orphans *could* have resumed, which is how anyone
    /// finds out whether the rest of S-2.6 is worth building.
    pub async fn recover_orphans(&self) {
        self.report_resumable_orphans().await;
        match sqlx::query(
            "UPDATE agent_runs SET status = 'failed',
                 error = 'interrupted by platform restart', finished_at = now()
             WHERE status IN ('queued', 'running', 'waiting_backtest', 'awaiting_approval')",
        )
        .execute(&self.pg)
        .await
        {
            Ok(r) if r.rows_affected() > 0 => {
                tracing::info!(
                    count = r.rows_affected(),
                    "agent runs marked failed on restart"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "agent orphan recovery failed"),
        }

        // A run that died holding a pending approval leaves a question nobody is
        // listening to. The approval row is deliberately written before the wait so
        // it outlives the process (guide 14.3) - but the loop state is not persisted
        // yet, so there is nothing left to resume when it is answered.
        //
        // Cancelling is the honest form of that: an approval a human can still answer,
        // to no effect, is worse than one that says plainly it is gone. Durable
        // resume is what would let this become a real resume instead.
        match sqlx::query(
            "UPDATE approval_requests a SET state = 'cancelled'
               FROM agent_runs r
              WHERE a.run_id = r.run_id
                AND a.state = 'pending'
                AND r.status NOT IN ('queued', 'running', 'waiting_backtest', 'awaiting_approval')",
        )
        .execute(&self.pg)
        .await
        {
            Ok(r) if r.rows_affected() > 0 => tracing::info!(
                count = r.rows_affected(),
                "pending approvals cancelled: their runs did not survive the restart"
            ),
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "approval orphan recovery failed"),
        }
    }

    /// What the effect log says about each orphan, logged rather than acted on.
    ///
    /// Reads the log for every run about to be failed and reports the decision
    /// `super::resume::decide` would make. A run whose log ends on a **non-idempotent**
    /// effect is the interesting one: it is the case where resuming naively would
    /// duplicate a write, and the count of those is the argument for finishing the
    /// rest of S-2.6 or for leaving it.
    async fn report_resumable_orphans(&self) {
        let runs: Vec<uuid::Uuid> = match sqlx::query_scalar(
            "SELECT run_id FROM agent_runs              WHERE status IN ('queued', 'running', 'waiting_backtest', 'awaiting_approval')",
        )
        .fetch_all(&self.pg)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "could not list orphaned runs");
                return;
            }
        };

        for run_id in runs {
            let rows: Vec<(i32, String, String, serde_json::Value, String)> = match sqlx::query_as(
                "SELECT seq, direction, kind, payload_json, harness_version                  FROM agent_effect_log WHERE run_id = $1 ORDER BY seq",
            )
            .bind(run_id)
            .fetch_all(&self.pg)
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(%run_id, error = %e, "could not read the effect log");
                    continue;
                }
            };
            let rows: Vec<super::resume::LogRow> = rows
                .into_iter()
                .map(
                    |(seq, direction, kind, payload, harness_version)| super::resume::LogRow {
                        seq,
                        direction,
                        kind,
                        payload,
                        harness_version,
                    },
                )
                .collect();

            match super::resume::decide(&rows, super::local_driver::HARNESS_VERSION) {
                super::resume::Resume::Replay { inputs, pending } => match pending {
                    Some(p) if !p.idempotent => tracing::info!(
                        %run_id, inputs = inputs.len(), effect = %p.kind,
                        "orphan: resumable, with an interrupted WRITE that must not be retried"
                    ),
                    Some(p) => tracing::info!(
                        %run_id, inputs = inputs.len(), effect = %p.kind,
                        "orphan: resumable; the outstanding effect is idempotent"
                    ),
                    None => tracing::info!(
                        %run_id, inputs = inputs.len(),
                        "orphan: resumable cleanly; nothing was outstanding"
                    ),
                },
                super::resume::Resume::Interrupted { reason } => {
                    tracing::info!(%run_id, %reason, "orphan: not resumable");
                }
                super::resume::Resume::Nothing { reason } => {
                    tracing::debug!(%run_id, %reason, "orphan: nothing to resume");
                }
            }
        }
    }

    /// Stop accepting turns, and wait for the ones in flight to reach a boundary
    /// (§10.1, S-2.15).
    ///
    /// Three cases, and the third is the one worth being careful about:
    ///
    /// * a **suspended** run needs nothing — it holds no process, and S-2.6's schema
    ///   rule lets it resume across the version change;
    /// * a **mid-step** run finishes its step and suspends at the boundary. A step is
    ///   one model call plus one tool call, so this is seconds;
    /// * a run still **inside a tool** past the deadline is left `running` and
    ///   recovered by resume. It is never killed: a tool killed mid-write is exactly
    ///   the duplicated-write case the effect log exists to avoid, and a deploy is
    ///   not a good enough reason to create one.
    ///
    /// Returns how many runs were still going when the deadline passed, so the caller
    /// can say so rather than claiming a clean stop it did not observe.
    pub async fn drain(&self, deadline: std::time::Duration) -> usize {
        self.draining.store(true, Ordering::SeqCst);
        tracing::info!("draining: no new agent turns will be accepted");

        let started = std::time::Instant::now();
        while started.elapsed() < deadline {
            let left = self.active_count();
            if left == 0 {
                tracing::info!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "drained cleanly"
                );
                return 0;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        let left = self.active_count();
        if left > 0 {
            // Reported, not killed. These are recovered by resume, and saying so is
            // what stops the next reader assuming they were lost.
            tracing::warn!(
                runs = left,
                "drain deadline passed with runs still inside a tool; leaving them                  `running` for resume rather than killing them mid-write"
            );
        }
        left
    }

    #[must_use]
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::SeqCst)
    }

    fn active_count(&self) -> usize {
        self.running.lock().expect("agent running lock").len()
    }

    fn finish(&self, run_id: Uuid) {
        self.running
            .lock()
            .expect("agent running lock")
            .remove(&run_id);
    }

    /// Sends one user message into a conversation, starting a turn.
    ///
    /// This is the whole user-facing surface now. No instrument, no timeframe, no
    /// iteration cap, no time budget: the agent chooses what to look at, how many
    /// backtests to run and when it is done, because those were never decisions a
    /// user could make correctly in advance.
    /// `workflow` is `None` from the conversation front door, which offers no
    /// workflow chip (AGENT-012 §3.3) and therefore starts every turn as the
    /// default. The session driver passes the agenda item's workflow, so a run
    /// started to replicate a stale finding says `replicate` on its row and in
    /// its loop rather than claiming to be an investigation. One value feeds
    /// both, which is what keeps the row and the loop from disagreeing.
    pub async fn send_message(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        text: &str,
        creds: &LlmCredentialStore,
        workflow: Option<harness::state::Workflow>,
    ) -> Result<Uuid, StartError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(StartError::InvalidRequest("the message is empty".into()));
        }

        let convo = conversations::get(&self.pg, user_id, conversation_id)
            .await
            .map_err(|_| StartError::InvalidRequest("conversation not found".into()))?;

        // One turn at a time. Two agents on one thread would interleave their tool
        // calls and their notes in one workspace, and the transcript would stop being
        // a sequence anyone could read.
        let (turn_index, live_run) = conversations::next_turn(&self.pg, conversation_id)
            .await
            .map_err(|e| StartError::Internal(e.to_string()))?;
        if let Some(run_id) = live_run {
            // Named, because the three things a caller can do about this all need
            // the id: look at it, stop it, or decide it is stale (S-2.14).
            return Err(StartError::InvalidRequest(format!(
                "this conversation is still working on run {run_id} — watch it, or stop it                  first. Two turns in one conversation would share a workspace with no lock,                  and two working states for one goal is not something the desk can render."
            )));
        }

        let provider = Provider::from_str(&convo.provider).map_err(StartError::InvalidRequest)?;
        // Refused before capacity, because the two answers differ and this one is
        // the more specific: a caller told "at capacity" during a deploy waits for a
        // run to finish that is never going to start.
        if self.is_draining() {
            return Err(StartError::Draining {
                seconds: DRAIN_DEADLINE_SECS,
            });
        }
        if self.active_count() >= self.cfg.max_concurrent_runs {
            return Err(StartError::Busy);
        }

        let cred = creds
            .load(user_id, provider.as_str())
            .await
            .map_err(|e| StartError::Internal(e.to_string()))?;
        let (api_key, base_url) = match cred {
            Some(c) => ((!c.api_key.is_empty()).then_some(c.api_key), c.base_url),
            None if provider.requires_api_key() => {
                return Err(StartError::NotConfigured(provider.as_str().to_string()))
            }
            None => (None, None),
        };
        let llm_client = LlmClient::new(provider, api_key, base_url);

        let run_id = Uuid::new_v4();
        let workflow = workflow.unwrap_or(harness::state::Workflow::Investigate);
        // The row says what the loop will say, derived from the same input
        // (AGENT-013 §2.1, §4; S-10.1).
        //
        // **This path writes `research`, not `chat`, and that is deliberate.**
        // §2.2 makes the conversation front door a Chat turn — ring 0, lookups
        // only, a Research *offer* when the question needs more. The offer and
        // its Start action are S-10.2's other half and are not built yet, so
        // declaring this front door Chat today would take the platform's main
        // capability away and leave nothing in its place. Recorded in the phase
        // plan as the one thing S-10.2 is waiting on rather than left as a
        // surprise.
        //
        // What matters here is that the row and the loop cannot disagree: both
        // read `TaskDemand`, through the same enum, so "what was this run
        // allowed to do" has one answer.
        let (mode, ring) = match CHAT_DEMAND {
            harness::hardware::TaskDemand::SingleLookup => {
                (harness::state::Mode::Chat, harness::state::Ring::Research)
            }
            harness::hardware::TaskDemand::MultiStep => (
                harness::state::Mode::Research,
                harness::state::Ring::PlatformArtifacts,
            ),
        };
        let mode_str = enum_name(&mode);
        let ring_str = enum_name(&ring);
        sqlx::query(
            "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model,
                 constraints_json, conversation_id, turn_index, mode, ring, workflow)
             VALUES ($1, $2, 'queued', $3, $4, $5, '{}'::jsonb, $6, $7, $8, $9, $10)",
        )
        .bind(run_id)
        .bind(user_id)
        .bind(text)
        .bind(&convo.provider)
        .bind(&convo.model)
        .bind(conversation_id)
        .bind(turn_index)
        // What this run is allowed to do, on the row rather than only inside the
        // run (AGENT-013 §2.1, §4; S-10.1). "Could this run have armed an
        // automation?" must be answerable without re-deriving it from a
        // transcript.
        //
        // Derived here from the same two facts `Loop::new` derives them from —
        // the demand and the mode — so the row and the state card cannot
        // disagree about what the run was.
        .bind(mode_str)
        .bind(ring_str)
        // The workflow is on the run *and* on every trial it spends (§3), so the
        // ledger can answer "how many of our trials are replications" and the
        // scorecard can answer "which workflows does the 7B actually complete".
        .bind(enum_name(&workflow))
        .execute(&self.pg)
        .await
        .map_err(|e| StartError::Internal(e.to_string()))?;

        conversations::touch(&self.pg, conversation_id).await;

        // Every Research conversation is a session (AGENT-014 §2.1, §10;
        // S-11.1). A bounded run is `continuous = false` with one agenda item,
        // so the desk, the digest and the agenda have one shape to read rather
        // than two — and a run started here and a run started by the session
        // driver are the same kind of thing.
        //
        // Best-effort and after the run row: a session row that could not be
        // written must not stop a turn the user asked for. What it costs is that
        // the conversation has no agenda card, which is visible; what refusing
        // would cost is the turn, which is the thing they came for.
        if mode == harness::state::Mode::Research {
            self.ensure_session(user_id, conversation_id, &convo.model, text, run_id)
                .await;
        }

        // The prompt is the first thing in the transcript, so a reader sees the
        // question before the work. Written here rather than by the driver because it
        // must be there even if the driver never starts.
        let _ = sqlx::query(
            "INSERT INTO agent_messages (run_id, seq, kind, content_json)
             VALUES ($1, 0, 'user', $2)",
        )
        .bind(run_id)
        .bind(serde_json::json!({ "content": text }))
        .execute(&self.pg)
        .await;

        // Title the conversation from its first prompt, in the background. It is one
        // cheap call and nothing waits on it — a sidebar row that says the first few
        // words for a second is fine; a request that blocks on a summariser is not.
        if turn_index == 0 {
            // A name immediately, so the sidebar row is never anonymous, and a better
            // one when the summariser answers. On a cold local model that second call
            // is nearly three minutes.
            conversations::set_title(
                &self.pg,
                conversation_id,
                &conversations::provisional_title(text),
            )
            .await;

            let pg = self.pg.clone();
            let client = llm_client.clone();
            let model = convo.model.clone();
            let prompt = text.to_string();
            tokio::spawn(async move {
                let title = conversations::generate_title(&client, &model, &prompt).await;
                conversations::set_title(&pg, conversation_id, &title).await;
            });
        }

        let prior = conversations::prior_context(&self.pg, conversation_id, turn_index)
            .await
            .unwrap_or_default();

        let run_token = crate::auth::handlers::new_session_token();
        // The token outlives any single turn by a wide margin: there is no wall clock
        // any more, so an expiry short enough to be useful would be an expiry that
        // kills long work.
        let expires_at = Utc::now() + ChronoDuration::days(7);
        sqlx::query(
            "INSERT INTO sessions (token, user_id, label, kind, expires_at)
             VALUES ($1, $2, $3, 'service', $4)",
        )
        .bind(&run_token)
        .bind(user_id)
        .bind(format!("agent-run-{run_id}"))
        .bind(expires_at)
        .execute(&self.pg)
        .await
        .map_err(|e| StartError::Internal(e.to_string()))?;

        let cancel = Arc::new(RunControl::new());
        self.running
            .lock()
            .expect("agent running lock")
            .insert(run_id, cancel.clone());

        let manager = Arc::clone(self);
        let pg = self.pg.clone();
        let base_url = self.base_url.clone();
        let model = convo.model.clone();

        let local = self
            .profiles
            .get(&model)
            .filter(|p| p.tier.is_local())
            .cloned();

        if let Some(profile) = local {
            // Per-kind (§21), from the same classifier the loop uses.
            let charter = prompt::charter_for(harness::state::GoalKind::classify(text, &[]));
            // The slice the previous turn ended with, so this one appends to it
            // rather than re-choosing an order (§21). A cached prefix matches by
            // bytes, so the same tools in a different order is a miss.
            let prior_exposure = self
                .prefixes
                .lock()
                .expect("agent prefix lock")
                .get(&conversation_id)
                .map(|p| p.tools().to_vec())
                .unwrap_or_default();
            let exposure_out = Arc::new(std::sync::Mutex::new(Vec::new()));
            let charter_for_prefix = charter.clone();
            let params = LocalRunParams {
                run_id,
                user_id,
                conversation_id,
                prior_exposure,
                exposure_out: Arc::clone(&exposure_out),
                // The same value the row above was written from. The
                // conversation front door still offers no chip and passes
                // `None`; the session driver passes its agenda item's.
                workflow,
                goal: text.to_string(),
                charter,
                prior,
                // The chat path carries none; the field exists so the research
                // path can, and so both drivers frame a turn identically.
                constraints: serde_json::json!({}),
                profile,
                namespaces: vec!["core".into(), "discovery".into()],
                demand: CHAT_DEMAND,
                api_base: base_url,
                service_token: run_token.clone(),
                workspace_root: workspace_root(),
            };
            tokio::spawn(async move {
                local_driver::run(pg.clone(), llm_client, params, cancel).await;
                // Fold this turn into the conversation's prefix. `extend` says
                // whether the turn continued it; a conversation paying more than
                // one cold prefix is a defect, and this is where it becomes
                // visible rather than showing up only as a bill.
                let used = exposure_out.lock().map(|s| s.clone()).unwrap_or_default();
                let continuity = manager
                    .prefixes
                    .lock()
                    .expect("agent prefix lock")
                    .entry(conversation_id)
                    .or_default()
                    .extend(&charter_for_prefix, &used);
                if !continuity.is_continuous() {
                    tracing::debug!(
                        %conversation_id,
                        reason = continuity.broken_by.as_deref().unwrap_or(""),
                        "turn paid a cold prefix"
                    );
                }
                conversations::touch(&pg, conversation_id).await;
                let _ = sqlx::query("DELETE FROM sessions WHERE token = $1")
                    .bind(&run_token)
                    .execute(&pg)
                    .await;
                manager.finish(run_id);
            });
            return Ok(run_id);
        }

        let budget = self
            .profiles
            .get(&model)
            .map(harness::Profile::input_budget_tokens);
        let params = DriverParams {
            run_id,
            user_id,
            goal: text.to_string(),
            model,
            constraints: serde_json::json!({}),
            prior,
            conversation_id: Some(conversation_id),
            workspace_root: workspace_root(),
            // The same 95% trigger the local tier uses, from the model's own profile.
            input_budget_tokens: budget,
            llm_max_tokens_per_call: self.cfg.llm_max_tokens_per_call,
        };
        let api = ApiClient::new(base_url, run_token.clone());
        tokio::spawn(async move {
            driver::drive(&pg, api, llm_client, params, cancel).await;
            conversations::touch(&pg, conversation_id).await;
            let _ = sqlx::query("DELETE FROM sessions WHERE token = $1")
                .bind(&run_token)
                .execute(&pg)
                .await;
            manager.finish(run_id);
        });

        Ok(run_id)
    }

    /// Give this conversation its session row, if it does not have one.
    ///
    /// Three things are deliberately *not* done here.
    ///
    /// **A child of a continuous session is skipped.** It already belongs to a
    /// session — the driver opened its conversation under the journal — and
    /// giving it a second one would be refused by the unique index on
    /// `conversation_id` anyway. Checking the parent is how a child is
    /// recognised without the driver having to race the front door.
    ///
    /// **Only the first turn writes.** A conversation's session is its first
    /// question; later turns are turns in the same session, and the `ON
    /// CONFLICT` in [`sessions::create`] would make a second attempt a no-op in
    /// any case.
    ///
    /// **A failure is logged, not returned.** The caller is in the middle of
    /// starting a turn the user asked for.
    async fn ensure_session(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
        tier: &str,
        question: &str,
        run_id: Uuid,
    ) {
        let parent: Option<(Option<Uuid>,)> = sqlx::query_as(
            "SELECT parent_conversation_id FROM agent_conversations WHERE conversation_id = $1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pg)
        .await
        .unwrap_or(None);
        if matches!(parent, Some((Some(_),))) {
            return;
        }

        let existing = crate::sessions::for_conversation(&self.pg, conversation_id)
            .await
            .ok()
            .flatten();
        let session = match existing {
            Some(s) => s,
            None => {
                let session = research_session::ResearchSession::bounded(
                    Uuid::new_v4(),
                    conversation_id,
                    question,
                    tier,
                );
                if let Err(e) = crate::sessions::create(&self.pg, user_id, &session).await {
                    tracing::warn!(%conversation_id, error = %e, "could not open the session row for this conversation");
                    return;
                }
                // The one item it is. Written here rather than by a driver
                // because a bounded session has no driver: the turn is the run.
                if let Some(item) = session.sole_item() {
                    let started = crate::sessions::insert_item(&self.pg, &item).await;
                    if let Ok(Some(id)) = started {
                        let _ = crate::sessions::set_status(
                            &self.pg,
                            id,
                            &research_session::ItemStatus::Running { run_id },
                        )
                        .await;
                        let _ = crate::sessions::append(
                            &self.pg,
                            session.id,
                            research_session::EventKind::SessionStarted,
                            serde_json::json!({ "bounded": true }),
                        )
                        .await;
                        let _ = crate::sessions::append(
                            &self.pg,
                            session.id,
                            research_session::EventKind::AgendaItemAdded,
                            serde_json::json!({ "item": id, "source": "user" }),
                        )
                        .await;
                        let _ = crate::sessions::append(
                            &self.pg,
                            session.id,
                            research_session::EventKind::RunStarted,
                            serde_json::json!({ "item": id, "run_id": run_id }),
                        )
                        .await;
                        let _ =
                            crate::sessions::attribute_run(&self.pg, run_id, session.id, id).await;
                    }
                }
                return;
            }
        };
        // A later turn in a conversation that already has its session. Nothing
        // to write; named so the branch is not mistaken for a gap.
        let _ = session;
    }

    /// Cancels whatever is running in a conversation.
    pub async fn cancel_conversation(
        &self,
        user_id: Uuid,
        conversation_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        let running: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT r.run_id FROM agent_runs r
               JOIN agent_conversations c ON c.conversation_id = r.conversation_id
              WHERE r.conversation_id = $1 AND c.user_id = $2
                AND r.status = ANY($3)",
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(conversations::ACTIVE_RUN_STATES)
        .fetch_all(&self.pg)
        .await?;

        let mut any = false;
        for (run_id,) in running {
            any |= self.cancel(user_id, run_id).await?;
        }
        Ok(any)
    }

    /// Request cancellation. In-process runs get their flag set (the driver
    /// marks the row); orphaned non-terminal rows are updated directly.
    pub async fn cancel(&self, user_id: Uuid, run_id: Uuid) -> Result<bool, sqlx::Error> {
        let owned: Option<(String,)> =
            sqlx::query_as("SELECT status FROM agent_runs WHERE run_id = $1 AND user_id = $2")
                .bind(run_id)
                .bind(user_id)
                .fetch_optional(&self.pg)
                .await?;
        let Some((status,)) = owned else {
            return Ok(false);
        };
        if Self::is_terminal(&status) {
            return Ok(true);
        }

        let flag = self
            .running
            .lock()
            .expect("agent running lock")
            .get(&run_id)
            .cloned();
        match flag {
            Some(f) => f.cancel(),
            None => {
                // Not driven by this process — orphan; close it directly.
                sqlx::query(
                    "UPDATE agent_runs SET status = 'cancelled', finished_at = now()
                     WHERE run_id = $1 AND status IN ('queued','running','waiting_backtest','awaiting_approval')",
                )
                .bind(run_id)
                .execute(&self.pg)
                .await?;
            }
        }
        Ok(true)
    }

    /// The control handle, if this process is the one driving the run.
    fn control(&self, run_id: Uuid) -> Option<Arc<RunControl>> {
        self.running
            .lock()
            .expect("agent running lock")
            .get(&run_id)
            .cloned()
    }

    /// `(status, is_terminal)` for a run this user owns, or `None` if they do
    /// not own one with that id.
    ///
    /// The terminal list is the same one [`Self::cancel`] uses. Two copies of
    /// it would be two answers to "is this run over", which is exactly the shape
    /// of the `ended == "succeeded"` defect: a branch keyed on a word the column
    /// never takes.
    async fn owned_status(
        &self,
        user_id: Uuid,
        run_id: Uuid,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT status FROM agent_runs WHERE run_id = $1 AND user_id = $2")
                .bind(run_id)
                .bind(user_id)
                .fetch_optional(&self.pg)
                .await?;
        Ok(row.map(|(s,)| s))
    }

    /// Terminal statuses. Named once; see [`Self::owned_status`].
    fn is_terminal(status: &str) -> bool {
        TERMINAL_RUN_STATES.contains(&status)
    }

    /// Hold a run at its next step boundary for a bounded window (ADR-D-27).
    ///
    /// # Errors
    ///
    /// Propagates the ownership lookup's database error.
    pub async fn pause(
        &self,
        user_id: Uuid,
        run_id: Uuid,
        window: PauseWindow,
    ) -> Result<PauseOutcome, sqlx::Error> {
        let Some(status) = self.owned_status(user_id, run_id).await? else {
            return Ok(PauseOutcome::NoSuchRun);
        };
        if Self::is_terminal(&status) {
            return Ok(PauseOutcome::AlreadyFinished(status));
        }
        // A run this process is not driving has no loop to hold. Recording a
        // pause for it would be a state nothing honours — the defect this
        // control was built to stop telling.
        let Some(control) = self.control(run_id) else {
            return Ok(PauseOutcome::NotDrivenHere(status));
        };
        match control.pause(window) {
            Ok(until) => Ok(PauseOutcome::Paused {
                until,
                seconds: window.as_secs(),
            }),
            Err(PauseRefused::AlreadyPaused) => Ok(PauseOutcome::AlreadyPaused(
                control.paused_until().unwrap_or_else(chrono::Utc::now),
            )),
            Err(PauseRefused::Stopping) => Ok(PauseOutcome::Stopping),
        }
    }

    /// Let a paused run take its next step.
    ///
    /// # Errors
    ///
    /// Propagates the ownership lookup's database error.
    pub async fn resume(&self, user_id: Uuid, run_id: Uuid) -> Result<ResumeOutcome, sqlx::Error> {
        let Some(status) = self.owned_status(user_id, run_id).await? else {
            return Ok(ResumeOutcome::NoSuchRun);
        };
        if Self::is_terminal(&status) {
            return Ok(ResumeOutcome::AlreadyFinished(status));
        }
        let Some(control) = self.control(run_id) else {
            return Ok(ResumeOutcome::NotDrivenHere(status));
        };
        match control.resume() {
            Ok(()) => Ok(ResumeOutcome::Resumed),
            Err(ResumeRefused::NotPaused) => Ok(ResumeOutcome::NotPaused),
            // The expiry case reaches here: the deadline passed, the boundary
            // flipped the run to stopping, and the operator's resume arrived
            // after. Saying so is the point — fail-closed means the run is gone,
            // not that the click did nothing.
            Err(ResumeRefused::Stopping) => Ok(ResumeOutcome::Stopping),
        }
    }

    /// What this process can honestly say about a run's pause.
    ///
    /// The run reads go through here rather than through a bare
    /// `Option<DateTime>`, because the interesting case is the one an `Option`
    /// flattens: a run this replica is not driving. Nothing here observed it, so
    /// the answer is *unknown* — and reporting unknown as `paused: false` would
    /// be an absent measurement dressed as a zero, in the one response an
    /// operator would use to decide whether a run is spending.
    ///
    /// `status` is passed rather than re-read: the caller already has the row,
    /// and a second query could disagree with the one it is about to serialise.
    #[must_use]
    pub fn pause_view(&self, run_id: Uuid, status: &str) -> PauseView {
        if Self::is_terminal(status) {
            // Over, and therefore not held — the one absence this process can
            // assert about a run it is not driving.
            return PauseView::Finished;
        }
        match self.control(run_id) {
            Some(c) => c.paused_until().map_or(PauseView::Running, PauseView::Held),
            None => PauseView::Unobserved,
        }
    }
}

/// What a reader may say about a run's pause (ADR-D-27).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseView {
    /// Held at a step boundary until this deadline. On expiry it stops.
    Held(chrono::DateTime<chrono::Utc>),
    /// This process is driving it and it is taking steps.
    Running,
    /// It has ended; a finished run holds no pause.
    Finished,
    /// Not driven here, so this process measured nothing. **Not `false`.**
    Unobserved,
}

impl PauseView {
    /// `(paused, paused_until)` as the run reads serialise them.
    ///
    /// Here rather than in the route so there is one mapping. Two would be two
    /// answers to "is this run held", and the second one written would be the
    /// one that forgot that [`Self::Unobserved`] is not `false`.
    #[must_use]
    pub fn as_json(self) -> (serde_json::Value, serde_json::Value) {
        match self {
            Self::Held(until) => (
                serde_json::Value::Bool(true),
                serde_json::Value::String(until.to_rfc3339()),
            ),
            Self::Running | Self::Finished => {
                (serde_json::Value::Bool(false), serde_json::Value::Null)
            }
            // An absent measurement is never a zero.
            Self::Unobserved => (serde_json::Value::Null, serde_json::Value::Null),
        }
    }
}

/// What [`AgentManager::pause`] decided. The route maps these to status codes;
/// the manager does not know about HTTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseOutcome {
    Paused {
        until: chrono::DateTime<chrono::Utc>,
        seconds: u64,
    },
    /// No run with that id belongs to this user.
    NoSuchRun,
    /// It is over; the status says how it ended.
    AlreadyFinished(String),
    /// Not driven by this process — an orphan, or a run of another replica.
    NotDrivenHere(String),
    AlreadyPaused(chrono::DateTime<chrono::Utc>),
    /// Cancelled, or its pause already expired.
    Stopping,
}

/// What [`AgentManager::resume`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed,
    NoSuchRun,
    AlreadyFinished(String),
    NotDrivenHere(String),
    NotPaused,
    Stopping,
}
