//! Agent run lifecycle: capacity, run rows, run-scoped tokens, spawn, and the
//! run controls — cancel, and the bounded pause/resume of ADR-D-27.
//!
//! Follows the platform's job pattern (ModelManager::start_train): insert a
//! status row, `tokio::spawn` a driver that updates it as it goes, poll over
//! REST.
//!
//! **One driver, every tier** (AGENT-010 §2; S-2.10). The tier decides the
//! profile and the profile decides the transport; nothing here branches on it.
//! Runs survive a restart: `recover_orphans` resumes each from its effect log
//! (S-2.6), and a run suspended on a platform job is resumed by the job watcher
//! when the job ends (S-2.11).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use cfg::model::AgentConfig;
use llm::{LlmClient, Provider};

use crate::credentials::LlmCredentialStore;

use super::control::{PauseRefused, PauseWindow, ResumeRefused, RunControl};
use super::conversations;
use super::local_driver::{self, Ended, LocalRunParams};
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
    /// Runs suspended on a platform job (AGENT-010 §8; S-2.11).
    ///
    /// Not in `running`: a suspended run holds no task, no model connection and
    /// no capacity slot. What it does hold is a service token, kept so the
    /// watcher can ask the platform about its job and so the resumed run can
    /// carry on under the identity it started with.
    waiting: Mutex<HashMap<Uuid, WaitingRun>>,
    /// Set once the job watcher is running, so a second `recover_orphans` does
    /// not start a second one.
    watcher_started: AtomicBool,
    /// The credential store, read from the environment on first use. Resume and
    /// escalation need a client for a run nobody is currently sending a message
    /// to, and there is no request to take one from.
    creds: Mutex<Option<LlmCredentialStore>>,
}

/// A run suspended on a job, and what is needed to resume it.
#[derive(Debug, Clone)]
struct WaitingRun {
    user_id: Uuid,
    conversation_id: Uuid,
    job_id: String,
    token: String,
}

/// What the conversation door may say about a turn beyond its text
/// (AGENT-012 §3.3; S-2.14).
///
/// Every field optional, so `{text}` alone still means what it always meant: a
/// Research turn under the default workflow.
#[derive(Debug, Clone, Default)]
pub struct TurnOptions {
    /// The stage skeleton. `None` is `investigate`.
    pub workflow: Option<harness::state::Workflow>,
    /// Chat or Research. `None` is Research, the front door's existing default.
    pub mode: Option<harness::state::Mode>,
    /// Deliver the text to the conversation's live run as a steer rather than
    /// start a second turn. With no live run it starts a turn, so the message is
    /// never dropped.
    pub steer: bool,
    /// Stop after this many trials, with one report turn (a session child's
    /// lease: `lease.max_trials_per_task()`; S-11.6). `None` is no cap, which is
    /// what every existing caller gets.
    pub max_trials: Option<u32>,
}

/// What [`AgentManager::drain_run`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainRun {
    /// This process is not driving the run (finished, suspended, or elsewhere).
    NotRunning,
    /// The run stopped at a step boundary, still `running`, with its next effect
    /// in the log. [`AgentManager::resume_drained`] — or the next boot — picks it
    /// up. Nothing was killed and nothing was lost.
    Drained,
    /// The deadline passed with the run still inside a tool or a model call. It
    /// has been asked to stop at its next boundary and will; it was not killed.
    StillInStep,
}

/// How often the job watcher looks at the runs it is holding.
///
/// One read per suspended run per interval, and no model call ever. A run
/// waiting on a forty-minute sweep costs this and nothing else.
const JOB_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// What the conversation front door asks the tier for.
///
/// `SingleLookup`: the front door is **Chat** (AGENT-013 §2.1; S-10.2). A Chat
/// turn is ring 0 with no trial-spending tool in its slice, and a goal that reads
/// as research ends the turn in `Outcome::Offered` -- the card the desk renders --
/// or, for a user who has said "always start research" for the scope, starts a
/// Research run itself (`Ended::StartResearch`, `start_offered_research`). A
/// caller who wants a Research run says so with `StartOptions::mode`.
///
/// This was `MultiStep` while the offer, its Start action and the standing
/// preference did not exist: declaring Chat then would have removed a capability
/// and built no door in its place. All three exist now. ADR-S10-12 (rejected: keep
/// the front door as Research and let Chat be opt-in -- it leaves every plain
/// conversation able to spend trials the user never asked for, which is the
/// property Chat mode exists to remove).
const CHAT_DEMAND: harness::hardware::TaskDemand = harness::hardware::TaskDemand::SingleLookup;

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
    // A run whose log could not be replayed (S-2.6): stopped with its working
    // state kept, never silently restarted.
    "interrupted",
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
            waiting: Mutex::new(HashMap::new()),
            watcher_started: AtomicBool::new(false),
            creds: Mutex::new(None),
        }
    }

    /// The credential store, built from the environment the first time it is
    /// needed.
    fn credentials(&self) -> LlmCredentialStore {
        let mut slot = self.creds.lock().expect("agent creds lock");
        slot.get_or_insert_with(|| {
            LlmCredentialStore::new(
                self.pg.clone(),
                crate::credentials::CredentialCrypto::from_env()
                    .ok()
                    .map(Arc::new),
            )
        })
        .clone()
    }

    /// A client for `provider` on behalf of `user_id`, from their stored key.
    async fn client_for(&self, user_id: Uuid, provider: &str) -> Result<LlmClient, StartError> {
        let provider = Provider::from_str(provider).map_err(StartError::InvalidRequest)?;
        let cred = self
            .credentials()
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
        Ok(crate::model_policy::client(provider, api_key, base_url))
    }

    /// The profile a turn runs under (AGENT-010 §2; S-2.9, S-2.10).
    ///
    /// Every model runs on the one loop, so every model needs a profile. In
    /// order:
    ///
    /// 1. its own, when one is shipped;
    /// 2. on a **hosted** provider, the shipped frontier profile with the model
    ///    id swapped in — the hosted driver this replaces ran every hosted model
    ///    at frontier settings, and the loop under a frontier profile is the same
    ///    shape with budgets, approvals and resume added;
    /// 3. on a **local** provider, the configured conservative default
    ///    (`TBOT_FALLBACK_PROFILE`), as guide §1.2 asks;
    /// 4. otherwise refused, naming the fix. Running an unknown local model at
    ///    frontier settings is the failure §1.2 exists to prevent.
    ///
    /// The substitution is returned so the run says which profile it actually
    /// ran under.
    fn profile_for(
        &self,
        provider: &str,
        model: &str,
    ) -> Result<(harness::Profile, Option<String>), StartError> {
        if let Some(p) = self.profiles.get(model) {
            return Ok((p.clone(), None));
        }
        let hosted = Provider::from_str(provider).is_ok_and(|p| !p.is_local());
        if hosted {
            // The provider's own frontier profile when one is shipped, so an
            // OpenAI model runs under OpenAI's window and budgets rather than
            // Claude's (PROVIDERS track, ADR-PROV-03). Any frontier profile
            // otherwise, which is what every hosted model got before.
            let frontier: Vec<&harness::Profile> = self
                .profiles
                .ids()
                .into_iter()
                .filter_map(|id| self.profiles.get(id))
                .filter(|p| p.tier == harness::profile::Tier::Frontier)
                .collect();
            let template = frontier
                .iter()
                .find(|p| p.provider.contains(provider))
                .or_else(|| frontier.first())
                .map(|p| (*p).clone());
            if let Some(mut p) = template {
                let from = p.model_id.clone();
                p.model_id = model.to_string();
                p.provider = provider.to_string();
                return Ok((
                    p,
                    Some(format!(
                        "no capability profile for {model:?}; it runs under the frontier profile {from:?} with its model id swapped in"
                    )),
                ));
            }
        }
        match self.profiles.resolve_for_execution(model) {
            harness::profile::Resolution::Exact(p) => Ok((p.clone(), None)),
            harness::profile::Resolution::Substituted {
                profile, reason, ..
            } => {
                let mut p = profile.clone();
                p.model_id = model.to_string();
                Ok((p, Some(reason)))
            }
            harness::profile::Resolution::Unknown { requested } => {
                Err(StartError::InvalidRequest(format!(
                    "no capability profile for {requested:?} and no fallback is configured. Add \
                     config/profiles/{requested}.yaml, or set TBOT_FALLBACK_PROFILE to a profile \
                     this model may run under."
                )))
            }
        }
    }

    /// Overrides the profile set. For tests, and for a caller that has already
    /// loaded one.
    #[must_use]
    pub fn with_profiles(mut self, profiles: Arc<harness::ProfileSet>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Resume runs orphaned by a previous process (AGENT-010 §5; S-2.6).
    ///
    /// The policy used to be the same as backtests': everything unfinished became
    /// `failed`, and a forty-minute sweep nineteen minutes in lost nineteen
    /// minutes and the model's findings with them. Now each orphan is resumed
    /// from its effect log:
    ///
    /// * a run that was **mid-step** (running, or paused on a human) is replayed
    ///   and continues — an interrupted write is answered from the durable record
    ///   or reported to the model, never re-run, and a pending approval is
    ///   re-attached to the row a human may already have answered;
    /// * a run **suspended on a job** is handed to the job watcher, which resumes
    ///   it when the job ends;
    /// * a run whose log cannot be replayed is marked `interrupted`, with its
    ///   working state and any job result kept.
    ///
    /// Then the job watcher starts. It is started here because this is the one
    /// call every boot makes.
    pub async fn recover_orphans(self: &Arc<Self>) {
        self.report_resumable_orphans().await;
        let orphans: Vec<(Uuid, Uuid, Option<Uuid>, String, String, String)> = match sqlx::query_as(
            "SELECT r.run_id, r.user_id, r.conversation_id, r.status, r.provider, r.model
               FROM agent_runs r
              WHERE r.status IN ('queued', 'running', 'waiting_backtest', 'awaiting_approval')",
        )
        .fetch_all(&self.pg)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "could not list orphaned agent runs");
                Vec::new()
            }
        };

        for (run_id, user_id, conversation_id, status, provider, _model) in orphans {
            let Some(conversation_id) = conversation_id else {
                // A run from before conversations existed has no workspace to
                // resume into.
                self.mark_failed(run_id, "interrupted by platform restart")
                    .await;
                continue;
            };
            if status == "waiting_backtest" {
                match self.suspended_job(run_id).await {
                    Some(job_id) => match self.mint_run_token(user_id, run_id).await {
                        Ok(token) => {
                            self.waiting.lock().expect("agent waiting lock").insert(
                                run_id,
                                WaitingRun {
                                    user_id,
                                    conversation_id,
                                    job_id,
                                    token,
                                },
                            );
                        }
                        Err(e) => {
                            tracing::warn!(%run_id, error = %e, "could not re-issue a token for a suspended run");
                        }
                    },
                    // Waiting under the old driver, which kept no resume contract.
                    None => {
                        self.mark_failed(run_id, "interrupted by platform restart")
                            .await;
                    }
                }
                continue;
            }
            let Ok(client) = self.client_for(user_id, &provider).await else {
                self.mark_failed(
                    run_id,
                    "interrupted by platform restart: no credential to resume it with",
                )
                .await;
                continue;
            };
            self.spawn_resume(run_id, user_id, conversation_id, client, None)
                .await;
        }

        // Approvals whose runs did not survive (failed, interrupted) are
        // cancelled: an approval a human can still answer, to no effect, is
        // worse than one that says plainly it is gone. Approvals of resumed runs
        // are left alone — the resumed run re-attaches to them.
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

        if !self.watcher_started.swap(true, Ordering::SeqCst) {
            let manager = Arc::clone(self);
            tokio::spawn(async move { manager.watch_jobs().await });
        }
    }

    async fn mark_failed(&self, run_id: Uuid, why: &str) {
        let _ = sqlx::query(
            "UPDATE agent_runs SET status = 'failed', error = $2, finished_at = now()
              WHERE run_id = $1",
        )
        .bind(run_id)
        .bind(why)
        .execute(&self.pg)
        .await;
    }

    /// The job a suspended run is waiting on, from its resume contract.
    async fn suspended_job(&self, run_id: Uuid) -> Option<String> {
        sqlx::query_scalar(
            "SELECT payload_json->>'job_id' FROM agent_effect_log
              WHERE run_id = $1 AND direction = 'state' AND kind = 'Suspended'
              ORDER BY seq DESC LIMIT 1",
        )
        .bind(run_id)
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()
        .flatten()
    }

    /// A run-scoped service token, labelled with the run so the audit trail can
    /// attribute the run's calls to it (and so it can be found to delete).
    async fn mint_run_token(&self, user_id: Uuid, run_id: Uuid) -> Result<String, sqlx::Error> {
        let token = crate::auth::handlers::new_session_token();
        // No wall clock (§6): an expiry short enough to matter would kill long
        // work. Seven days, as for a fresh turn.
        let expires_at = Utc::now() + ChronoDuration::days(7);
        sqlx::query(
            "INSERT INTO sessions (token, user_id, label, kind, expires_at)
             VALUES ($1, $2, $3, 'service', $4)",
        )
        .bind(&token)
        .bind(user_id)
        .bind(format!("agent-run-{run_id}"))
        .bind(expires_at)
        .execute(&self.pg)
        .await?;
        Ok(token)
    }

    /// Delete a run's service tokens. By label rather than by value: the store
    /// may hold a hash of the token rather than the token (migration 0150), and a
    /// delete keyed on the raw value would then silently match nothing.
    async fn revoke_run_tokens(&self, run_id: Uuid) {
        let _ = sqlx::query("DELETE FROM sessions WHERE label = $1 AND kind = 'service'")
            .bind(format!("agent-run-{run_id}"))
            .execute(&self.pg)
            .await;
    }

    /// Resume one run in the background, under a fresh token.
    async fn spawn_resume(
        self: &Arc<Self>,
        run_id: Uuid,
        user_id: Uuid,
        conversation_id: Uuid,
        client: LlmClient,
        job_done: Option<local_driver::JobDone>,
    ) {
        let token = match self.mint_run_token(user_id, run_id).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(%run_id, error = %e, "could not mint a token to resume this run");
                return;
            }
        };
        let control = Arc::new(RunControl::new());
        self.running
            .lock()
            .expect("agent running lock")
            .insert(run_id, Arc::clone(&control));
        let exposure_out = Arc::new(std::sync::Mutex::new(Vec::new()));
        let params = local_driver::ResumeParams {
            run_id,
            user_id,
            conversation_id,
            workspace_root: workspace_root(),
            api_base: self.base_url.clone(),
            service_token: token.clone(),
            exposure_out: Arc::clone(&exposure_out),
            job_done,
        };
        let manager = Arc::clone(self);
        let pg = self.pg.clone();
        tokio::spawn(async move {
            let resumed = local_driver::resume(pg, client, params, control).await;
            let ended = match resumed {
                local_driver::Resumed::Ran(e) => e,
                local_driver::Resumed::Interrupted(_) => Ended::Finished,
                local_driver::Resumed::StillWaiting => {
                    // Back to the watcher; it asked too early.
                    manager.finish(run_id);
                    return;
                }
            };
            manager
                .after_run(run_id, user_id, conversation_id, token, ended)
                .await;
        });
    }

    /// The job watcher (AGENT-010 §8; S-2.11).
    ///
    /// Every interval it asks the platform, once per suspended run, whether that
    /// run's job has ended — a read on the platform's own API, no model call, no
    /// tokens. When one has, the run is resumed from its effect log with the
    /// job's result delivered as `Input::JobDone`, which is the result of the
    /// tool call that started it, arriving late.
    ///
    /// A watcher polling the job service is still a poll, and it is the honest
    /// form of one: sweeps live in the research manager's memory rather than in
    /// `crates/jobs`, so there is no completion event to subscribe to. What
    /// §8 forbids is a *run* polling — holding a model connection and writing a
    /// status row every thirty seconds for forty minutes — and no run does.
    ///
    /// Boxed: the watcher resumes runs, a resumed run can suspend again, and a
    /// suspension can start the watcher — a cycle an `async fn` cannot name.
    fn watch_jobs(
        self: Arc<Self>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move { self.watch_jobs_inner().await })
    }

    async fn watch_jobs_inner(self: Arc<Self>) {
        loop {
            tokio::time::sleep(JOB_WATCH_INTERVAL).await;
            if self.is_draining() {
                return;
            }
            let held: Vec<(Uuid, WaitingRun)> = self
                .waiting
                .lock()
                .expect("agent waiting lock")
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect();
            for (run_id, w) in held {
                // Cancelled while it waited: the row says so, and there is nothing
                // to resume.
                let status: Option<String> =
                    sqlx::query_scalar("SELECT status FROM agent_runs WHERE run_id = $1")
                        .bind(run_id)
                        .fetch_optional(&self.pg)
                        .await
                        .ok()
                        .flatten();
                if status.as_deref() != Some("waiting_backtest") {
                    self.waiting
                        .lock()
                        .expect("agent waiting lock")
                        .remove(&run_id);
                    self.revoke_run_tokens(run_id).await;
                    continue;
                }
                let Some(done) = job_status(&self.base_url, &w.token, &w.job_id).await else {
                    continue;
                };
                self.waiting
                    .lock()
                    .expect("agent waiting lock")
                    .remove(&run_id);
                let provider: Option<String> =
                    sqlx::query_scalar("SELECT provider FROM agent_runs WHERE run_id = $1")
                        .bind(run_id)
                        .fetch_optional(&self.pg)
                        .await
                        .ok()
                        .flatten();
                let client = match provider {
                    Some(p) => self.client_for(w.user_id, &p).await,
                    None => Err(StartError::Internal("the run row is gone".into())),
                };
                match client {
                    Ok(client) => {
                        self.revoke_run_tokens(run_id).await;
                        self.spawn_resume(run_id, w.user_id, w.conversation_id, client, Some(done))
                            .await;
                    }
                    Err(e) => {
                        tracing::warn!(%run_id, error = %e, "the job ended but the run cannot be resumed without a client");
                        self.mark_failed(
                            run_id,
                            &format!("its job ended and it could not resume: {e}"),
                        )
                        .await;
                    }
                }
            }
        }
    }

    /// What happens once a run's time in this process ends, however it ends.
    async fn after_run(
        self: &Arc<Self>,
        run_id: Uuid,
        user_id: Uuid,
        conversation_id: Uuid,
        token: String,
        ended: Ended,
    ) {
        conversations::touch(&self.pg, conversation_id).await;
        match ended {
            Ended::Suspended { job_id, .. } => {
                // The token stays: the watcher asks about the job with it.
                self.waiting.lock().expect("agent waiting lock").insert(
                    run_id,
                    WaitingRun {
                        user_id,
                        conversation_id,
                        job_id,
                        token,
                    },
                );
                let manager = Arc::clone(self);
                if !self.watcher_started.swap(true, Ordering::SeqCst) {
                    tokio::spawn(async move { manager.watch_jobs().await });
                }
            }
            Ended::StartResearch { spec, workflow } => {
                self.revoke_run_tokens(run_id).await;
                self.start_offered_research(user_id, conversation_id, *spec, workflow)
                    .await;
            }
            Ended::Escalated { to, state, spec } => {
                self.revoke_run_tokens(run_id).await;
                self.escalate(user_id, conversation_id, run_id, &to, state, *spec)
                    .await;
            }
            Ended::Finished | Ended::Detached => {
                self.revoke_run_tokens(run_id).await;
            }
        }
        self.finish(run_id);
    }

    /// Start the Research run a Chat turn offered, because the user's standing
    /// preference says to (S-10.2). The same conversation, the same goal and
    /// profile, but a multi-step turn under the offered workflow. Every failure
    /// is logged: the Chat turn has already ended with its offer on record.
    async fn start_offered_research(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        spec: local_driver::RunSpec,
        workflow: harness::state::Workflow,
    ) {
        let provider = if spec.profile.provider.contains("anthropic") {
            "anthropic".to_string()
        } else {
            spec.profile.provider.clone()
        };
        let client = match self.client_for(user_id, &provider).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(%conversation_id, error = %e, "could not auto-start the offered research run");
                return;
            }
        };
        let launch = Launch {
            user_id,
            conversation_id,
            text: spec.goal.clone(),
            model: crate::model_policy::or_baseline(&provider, spec.profile.model_id.clone()),
            provider,
            profile: spec.profile.clone(),
            substituted: None,
            workflow,
            demand: harness::hardware::TaskDemand::MultiStep,
            constraints: spec.constraints.clone(),
            resumed_state: None,
            escalated_from: None,
            max_trials: spec.max_trials,
        };
        if let Err(e) = self.launch(launch, client).await {
            tracing::warn!(%conversation_id, error = %e, "the offered research run did not start");
        }
    }

    /// Start the escalation target with the working state the lower tier ended
    /// on (AGENT-008 §4, AGENT-010 §6; S-2.12).
    ///
    /// A new turn in the same conversation, so the timeline shows both: the tier
    /// that could not finish, and the one that resumed it. The new run's loop is
    /// seeded with the state, so the plan steps already done stay done and the
    /// trials already spent are counted once. Every failure here is recorded on
    /// the escalated run rather than raised: it has already ended.
    async fn escalate(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        from: Uuid,
        to: &str,
        state: Option<Box<harness::state::WorkingState>>,
        spec: local_driver::RunSpec,
    ) {
        let Some(state) = state else {
            // Admission-time escalation: nothing was done, so nothing to carry.
            return;
        };
        let Some(profile) = self.profiles.get(to).cloned() else {
            tracing::warn!(%from, to, "escalation target has no profile; the run stays escalated");
            return;
        };
        let provider = if profile.provider.contains("anthropic") {
            "anthropic"
        } else {
            profile.provider.as_str()
        };
        let client = match self.client_for(user_id, provider).await {
            Ok(c) => c,
            Err(e) => {
                let _ = sqlx::query(
                    "UPDATE agent_runs SET error = COALESCE(error, '') || $2 WHERE run_id = $1",
                )
                .bind(from)
                .bind(format!(" — escalation to {to} could not start: {e}"))
                .execute(&self.pg)
                .await;
                return;
            }
        };
        let launch = Launch {
            user_id,
            conversation_id,
            text: spec.goal.clone(),
            provider: provider.to_string(),
            model: profile.model_id.clone(),
            profile,
            substituted: None,
            workflow: spec.workflow,
            demand: spec.demand,
            constraints: spec.constraints.clone(),
            resumed_state: Some(*state),
            escalated_from: Some(from),
            max_trials: spec.max_trials,
        };
        if let Err(e) = self.launch(launch, client).await {
            tracing::warn!(%from, to, error = %e, "escalation did not start");
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

        // Every run in flight stops at its next boundary still `running`, with
        // its next effect already in the log, and the next process resumes it
        // (S-2.15). Not cancelled: a deploy is not a reason to end a run.
        for control in self.running.lock().expect("agent running lock").values() {
            control.detach();
        }

        // Tell the monitor (COMP-007 §4.3). A rolling deploy otherwise reads as a
        // platform heartbeat loss and can trip a global halt; an open window is
        // the one thing allowed to suppress that, and only that. Opened by the
        // platform, never by an agent — `MaintenanceWindow::open` refuses an
        // `agent:` opener.
        match monitor::window::MaintenanceWindow::open(
            "platform:deploy",
            "agent runtime draining for a deploy (AGENT-010 §10.1)",
            monitor::window::DEFAULT_WINDOW_MINUTES,
            Utc::now(),
        ) {
            Ok(w) => {
                if let Err(e) = monitor::store::MonitorStore::new(self.pg.clone())
                    .open_window(&w)
                    .await
                {
                    tracing::warn!(error = %e, "could not open a maintenance window for the deploy");
                } else {
                    tracing::info!(window = %w.window_id, "maintenance window opened for the deploy");
                }
            }
            Err(e) => tracing::warn!(error = %e, "maintenance window refused"),
        }

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

    /// Drain **one** run: stop it at its next step boundary, leaving it
    /// `running` with its next effect in the log (S-2.15, S-11.6, S-11.7).
    ///
    /// For a session child at a schedule-window boundary or when the kill switch
    /// trips. The same contract as the platform drain, per run: never killed
    /// mid-write, resumable by [`Self::resume_drained`] or the next boot. A run
    /// suspended on a job needs nothing — it holds no process — and reports
    /// [`DrainRun::NotRunning`].
    pub async fn drain_run(&self, run_id: Uuid, deadline: std::time::Duration) -> DrainRun {
        let Some(control) = self.control(run_id) else {
            return DrainRun::NotRunning;
        };
        control.detach();
        let started = std::time::Instant::now();
        while started.elapsed() < deadline {
            if self.control(run_id).is_none() {
                return DrainRun::Drained;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        if self.control(run_id).is_none() {
            DrainRun::Drained
        } else {
            DrainRun::StillInStep
        }
    }

    /// Resume a run [`Self::drain_run`] stopped, from its effect log.
    ///
    /// `Ok(false)` when there is nothing to resume: the run is not `running`, or
    /// this process is already driving it.
    pub async fn resume_drained(self: &Arc<Self>, run_id: Uuid) -> Result<bool, StartError> {
        if self.control(run_id).is_some() || self.is_draining() {
            return Ok(false);
        }
        let row: Option<(Uuid, Option<Uuid>, String, String)> = sqlx::query_as(
            "SELECT user_id, conversation_id, status, provider FROM agent_runs WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| StartError::Internal(e.to_string()))?;
        let Some((user_id, Some(conversation_id), status, provider)) = row else {
            return Ok(false);
        };
        if status != "running" {
            return Ok(false);
        }
        let client = self.client_for(user_id, &provider).await?;
        self.spawn_resume(run_id, user_id, conversation_id, client, None)
            .await;
        Ok(true)
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
    /// The session driver's and the research door's entry point; the
    /// conversation door uses [`Self::send_turn`], which can also say Chat or
    /// steer a live run.
    pub async fn send_message(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        text: &str,
        creds: &LlmCredentialStore,
        workflow: Option<harness::state::Workflow>,
    ) -> Result<Uuid, StartError> {
        self.send_message_capped(user_id, conversation_id, text, creds, workflow, None)
            .await
    }

    /// [`Self::send_message`] with a trial cap: the run stops after
    /// `max_trials` trials, with one report turn (S-11.6). `None` is exactly
    /// `send_message`.
    pub async fn send_message_capped(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        text: &str,
        creds: &LlmCredentialStore,
        workflow: Option<harness::state::Workflow>,
        max_trials: Option<u32>,
    ) -> Result<Uuid, StartError> {
        self.send_turn(
            user_id,
            conversation_id,
            text,
            creds,
            TurnOptions {
                workflow,
                max_trials,
                ..TurnOptions::default()
            },
        )
        .await
    }

    /// One user message into a conversation: a new turn, or a steer for the
    /// live one (S-2.14).
    ///
    /// No instrument, no timeframe, no iteration cap, no time budget: the agent
    /// chooses what to look at and when it is done.
    pub async fn send_turn(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        text: &str,
        creds: &LlmCredentialStore,
        opts: TurnOptions,
    ) -> Result<Uuid, StartError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(StartError::InvalidRequest("the message is empty".into()));
        }
        // Remember the store the caller holds, so resume and escalation can
        // build a client later without a request to take one from.
        {
            let mut slot = self.creds.lock().expect("agent creds lock");
            if slot.is_none() {
                *slot = Some(creds.clone());
            }
        }

        let convo = conversations::get(&self.pg, user_id, conversation_id)
            .await
            .map_err(|_| StartError::InvalidRequest("conversation not found".into()))?;

        let (_, live_run) = conversations::next_turn(&self.pg, conversation_id)
            .await
            .map_err(|e| StartError::Internal(e.to_string()))?;
        if let Some(run_id) = live_run {
            if opts.steer {
                // Delivered at the run's next step boundary (S-2.14).
                return match self.control(run_id) {
                    Some(c) if c.steer(text) => {
                        let _ = sqlx::query(
                            "INSERT INTO agent_messages (run_id, seq, kind, content_json)
                             SELECT $1, COALESCE(MAX(seq), 0) + 1, 'user', $2
                               FROM agent_messages WHERE run_id = $1",
                        )
                        .bind(run_id)
                        .bind(serde_json::json!({ "content": text, "steer": true }))
                        .execute(&self.pg)
                        .await;
                        Ok(run_id)
                    }
                    Some(_) => Err(StartError::InvalidRequest(format!(
                        "run {run_id} is stopping; a steer would reach nobody"
                    ))),
                    None => Err(StartError::InvalidRequest(format!(
                        "run {run_id} is not taking steps right now (it is waiting on a job or \
                         on another process), so a steer cannot reach it until it resumes"
                    ))),
                };
            }
            // Named, because the three things a caller can do about this all need
            // the id: look at it, stop it, or decide it is stale (S-2.14).
            return Err(StartError::InvalidRequest(format!(
                "this conversation is still working on run {run_id} — watch it, stop it, or \
                 send this with `steer: true` to redirect it. Two turns in one conversation \
                 would share a workspace with no lock, and two working states for one goal is \
                 not something the desk can render."
            )));
        }

        // Refused before capacity: a caller told "at capacity" during a deploy
        // waits for a run to finish that is never going to start.
        if self.is_draining() {
            return Err(StartError::Draining {
                seconds: DRAIN_DEADLINE_SECS,
            });
        }
        if self.active_count() >= self.cfg.max_concurrent_runs {
            return Err(StartError::Busy);
        }

        let (profile, substituted) = self.profile_for(&convo.provider, &convo.model)?;
        let client = self.client_for(user_id, &convo.provider).await?;

        let demand = match opts.mode {
            Some(harness::state::Mode::Chat) => harness::hardware::TaskDemand::SingleLookup,
            // A Research turn is multi-step work. Before this arm it fell through to the
            // single-lookup default, so a session's children ran as Chat and ended in an offer.
            Some(harness::state::Mode::Research) => harness::hardware::TaskDemand::MultiStep,
            _ => CHAT_DEMAND,
        };
        self.launch(
            Launch {
                user_id,
                conversation_id,
                text: text.to_string(),
                provider: convo.provider.clone(),
                model: convo.model.clone(),
                profile,
                substituted,
                workflow: opts
                    .workflow
                    .unwrap_or(harness::state::Workflow::Investigate),
                demand,
                constraints: serde_json::json!({}),
                resumed_state: None,
                escalated_from: None,
                max_trials: opts.max_trials,
            },
            client,
        )
        .await
    }

    /// Write the run row and start the one driver (S-2.10).
    #[allow(clippy::too_many_lines)]
    ///
    /// Boxed for the same reason as the watcher: a launched run can escalate, and
    /// escalation launches.
    fn launch<'a>(
        self: &'a Arc<Self>,
        l: Launch,
        llm_client: LlmClient,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Uuid, StartError>> + Send + 'a>>
    {
        Box::pin(self.launch_inner(l, llm_client))
    }

    async fn launch_inner(
        self: &Arc<Self>,
        l: Launch,
        llm_client: LlmClient,
    ) -> Result<Uuid, StartError> {
        let (turn_index, _) = conversations::next_turn(&self.pg, l.conversation_id)
            .await
            .map_err(|e| StartError::Internal(e.to_string()))?;
        let run_id = Uuid::new_v4();
        // The row says what the loop will say, derived from the same input
        // (AGENT-013 §2.1, §4; S-10.1): both read `TaskDemand`.
        let (mode, ring) = match l.demand {
            harness::hardware::TaskDemand::SingleLookup => {
                (harness::state::Mode::Chat, harness::state::Ring::Research)
            }
            harness::hardware::TaskDemand::MultiStep => (
                harness::state::Mode::Research,
                harness::state::Ring::PlatformArtifacts,
            ),
        };
        sqlx::query(
            "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model,
                 constraints_json, conversation_id, turn_index, mode, ring, workflow)
             VALUES ($1, $2, 'queued', $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(run_id)
        .bind(l.user_id)
        .bind(&l.text)
        .bind(&l.provider)
        .bind(&l.model)
        .bind(&l.constraints)
        .bind(l.conversation_id)
        .bind(turn_index)
        .bind(enum_name(&mode))
        .bind(enum_name(&ring))
        .bind(enum_name(&l.workflow))
        .execute(&self.pg)
        .await
        .map_err(|e| StartError::Internal(e.to_string()))?;

        conversations::touch(&self.pg, l.conversation_id).await;

        if mode == harness::state::Mode::Research && l.escalated_from.is_none() {
            self.ensure_session(l.user_id, l.conversation_id, &l.model, &l.text, run_id)
                .await;
        }

        // The prompt is the first thing in the transcript, written here so it is
        // there even if the driver never starts.
        let mut opening = serde_json::json!({ "content": l.text });
        if let Some(from) = l.escalated_from {
            opening["escalated_from"] = serde_json::json!(from);
        }
        if let Some(why) = &l.substituted {
            opening["profile_substituted"] = serde_json::json!(why);
        }
        let _ = sqlx::query(
            "INSERT INTO agent_messages (run_id, seq, kind, content_json)
             VALUES ($1, 0, 'user', $2)",
        )
        .bind(run_id)
        .bind(&opening)
        .execute(&self.pg)
        .await;

        if turn_index == 0 {
            conversations::set_title(
                &self.pg,
                l.conversation_id,
                &conversations::provisional_title(&l.text),
            )
            .await;
            let pg = self.pg.clone();
            let client = llm_client.clone();
            let model = l.model.clone();
            let prompt = l.text.clone();
            let conversation_id = l.conversation_id;
            tokio::spawn(async move {
                let title = conversations::generate_title(&client, &model, &prompt).await;
                conversations::set_title(&pg, conversation_id, &title).await;
            });
        }

        let prior = conversations::prior_context(&self.pg, l.conversation_id, turn_index)
            .await
            .unwrap_or_default();

        let run_token = self
            .mint_run_token(l.user_id, run_id)
            .await
            .map_err(|e| StartError::Internal(e.to_string()))?;

        let cancel = Arc::new(RunControl::new());
        self.running
            .lock()
            .expect("agent running lock")
            .insert(run_id, cancel.clone());

        // Per-kind charter (§21), from the same classifier the loop uses.
        let charter = prompt::charter_for(harness::state::GoalKind::classify(&l.text, &[]));
        // The slice the previous turn ended with, so this one appends to it
        // (§21): a cached prefix matches by bytes.
        let prior_exposure = self
            .prefixes
            .lock()
            .expect("agent prefix lock")
            .get(&l.conversation_id)
            .map(|p| p.tools().to_vec())
            .unwrap_or_default();
        let exposure_out = Arc::new(std::sync::Mutex::new(Vec::new()));
        let charter_for_prefix = charter.clone();
        let params = LocalRunParams {
            run_id,
            user_id: l.user_id,
            conversation_id: l.conversation_id,
            prior_exposure,
            exposure_out: Arc::clone(&exposure_out),
            workflow: l.workflow,
            goal: l.text.clone(),
            charter,
            prior,
            constraints: l.constraints.clone(),
            profile: l.profile,
            // `data` up front: a live test on 2026-10-05 asked for the latest BTC price, the
            // run had no bars tool exposed, and gpt-4o-mini looped on `list_lanes` until fenced.
            namespaces: vec!["core".into(), "discovery".into(), "data".into()],
            demand: l.demand,
            api_base: self.base_url.clone(),
            service_token: run_token.clone(),
            workspace_root: workspace_root(),
            resumed_state: l.resumed_state,
            max_trials: l.max_trials,
        };
        let manager = Arc::clone(self);
        let pg = self.pg.clone();
        let (user_id, conversation_id) = (l.user_id, l.conversation_id);
        tokio::spawn(async move {
            let ended = local_driver::run(pg, llm_client, params, cancel).await;
            // Fold this turn into the conversation's prefix. A conversation paying
            // more than one cold prefix is a defect, and this is where it shows.
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
            manager
                .after_run(run_id, user_id, conversation_id, run_token, ended)
                .await;
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

    /// Pin a tool for a run's next step (S-6.11). The tool must exist in the
    /// catalogue the run is routed from; the loop reads the pin at its next
    /// boundary and routes the tool first.
    ///
    /// # Errors
    ///
    /// Propagates the ownership lookup's database error.
    pub async fn pin_tool(
        &self,
        user_id: Uuid,
        run_id: Uuid,
        tool: &str,
    ) -> Result<PinOutcome, sqlx::Error> {
        let Some(status) = self.owned_status(user_id, run_id).await? else {
            return Ok(PinOutcome::NoSuchRun);
        };
        if Self::is_terminal(&status) {
            return Ok(PinOutcome::AlreadyFinished(status));
        }
        let known = super::bridge::registry()
            .map(|r| r.get(tool).is_some())
            .unwrap_or(false);
        if !known {
            return Ok(PinOutcome::UnknownTool);
        }
        let Some(control) = self.control(run_id) else {
            return Ok(PinOutcome::NotDrivenHere(status));
        };
        if control.pin(tool) {
            Ok(PinOutcome::Pinned)
        } else {
            Ok(PinOutcome::Stopping)
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

/// Everything [`AgentManager::launch`] needs for one run.
struct Launch {
    user_id: Uuid,
    conversation_id: Uuid,
    text: String,
    provider: String,
    model: String,
    profile: harness::Profile,
    /// Why the profile is not the model's own, when it is not.
    substituted: Option<String>,
    workflow: harness::state::Workflow,
    demand: harness::hardware::TaskDemand,
    constraints: serde_json::Value,
    resumed_state: Option<harness::state::WorkingState>,
    escalated_from: Option<Uuid>,
    max_trials: Option<u32>,
}

/// The platform's answer about a job, when it has ended (S-2.11).
///
/// `None` while it runs, and on a transient failure to ask — the watcher asks
/// again next interval. A job the platform no longer knows (a 4xx) has ended as
/// far as the run is concerned, and the run is told so rather than left waiting
/// for a job that will never finish.
async fn job_status(base_url: &str, token: &str, job_id: &str) -> Option<local_driver::JobDone> {
    let api = mcp_server_lib::ApiClient::new(base_url.to_string(), token.to_string());
    let path = if let Some(id) = job_id.strip_prefix("sweep:") {
        format!("/api/research/sweeps/{id}")
    } else if let Some(id) = job_id.strip_prefix("backtest:") {
        format!("/api/backtests/{id}")
    } else if let Some(id) = job_id.strip_prefix("campaign:") {
        format!("/api/lifecycle/campaigns/{id}")
    } else if let Some(rest) = job_id.strip_prefix("training:") {
        // `training:<model_id>:<run_id>`; ids are UUIDs or slugs without a colon.
        match rest.split_once(':') {
            Some((model, run)) => format!("/api/models/{model}/runs/{run}"),
            None => {
                return Some(local_driver::JobDone {
                    job_id: job_id.to_string(),
                    output: format!("{job_id} is not a training handle"),
                    ok: false,
                })
            }
        }
    } else if let Some(rest) = job_id.strip_prefix("study:") {
        match rest.split_once(':') {
            Some((exp, _)) => format!("/api/backtest/experiments/{exp}/studies"),
            None => {
                return Some(local_driver::JobDone {
                    job_id: job_id.to_string(),
                    output: format!("{job_id} is not a study handle"),
                    ok: false,
                })
            }
        }
    } else {
        // A job kind this watcher does not know cannot be resumed by it.
        return Some(local_driver::JobDone {
            job_id: job_id.to_string(),
            output: format!("the platform cannot report on job {job_id}"),
            ok: false,
        });
    };
    match api.get(&path).await {
        Ok(snapshot) => {
            // A study's list holds every study of the experiment; the run is
            // handed only its own.
            let output = study_entry(job_id, &snapshot).unwrap_or_else(|| snapshot.clone());
            snapshot_outcome(job_id, &snapshot).map(|ok| local_driver::JobDone {
                job_id: job_id.to_string(),
                output: output.to_string(),
                ok,
            })
        }
        Err(e) if matches!(e.status, Some(s) if s < 500) => Some(local_driver::JobDone {
            job_id: job_id.to_string(),
            output: e.to_tool_error().to_string(),
            ok: false,
        }),
        Err(_) => None,
    }
}

/// The one study a `study:<experiment>:<study>` handle names, from the
/// experiment's study list, or its failure record.
fn study_entry(job_id: &str, snapshot: &serde_json::Value) -> Option<serde_json::Value> {
    let study = job_id.strip_prefix("study:")?.split_once(':')?.1;
    snapshot
        .get("studies")?
        .as_array()?
        .iter()
        .find(|s| s.get("study_id").and_then(serde_json::Value::as_str) == Some(study))
        .cloned()
}

/// Whether a job's snapshot says it has ended, and if so whether it succeeded.
///
/// A sweep or a backtest reports a `status`. A campaign has no status: it reports
/// the phase it is in and a `terminal` phase once it has ended, so a campaign is
/// over exactly when `terminal` is a string, and it succeeded unless it was
/// halted. `None` is "still running".
fn snapshot_outcome(job_id: &str, snapshot: &serde_json::Value) -> Option<bool> {
    if let Some(rest) = job_id.strip_prefix("study:") {
        let study = rest.split_once(':').map_or("", |(_, s)| s);
        // Sealed: it is in the list, and that is success.
        if study_entry(job_id, snapshot).is_some() {
            return Some(true);
        }
        return match snapshot
            .pointer(&format!("/detached/{study}/status"))
            .and_then(serde_json::Value::as_str)
        {
            Some("running") => None,
            Some(_) => Some(false),
            // Neither sealed nor running: the process that was running it is gone
            // (the registry is in memory). Ended, and not a success.
            None => Some(false),
        };
    }
    if job_id.starts_with("training:") {
        let status = snapshot
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        return matches!(
            status,
            "succeeded" | "completed" | "failed" | "cancelled" | "error"
        )
        .then(|| matches!(status, "succeeded" | "completed"));
    }
    if job_id.starts_with("campaign:") {
        let terminal = snapshot
            .get("terminal")
            .and_then(serde_json::Value::as_str)?;
        return Some(terminal != "halted");
    }
    let status = snapshot
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    matches!(status, "completed" | "failed" | "cancelled").then(|| status == "completed")
}

#[cfg(test)]
mod snapshot_outcome_tests {
    use super::snapshot_outcome;
    use serde_json::json;

    #[test]
    fn a_study_and_a_training_run_resume_their_run_when_they_end() {
        let sealed = json!({"studies": [{"study_id": "s1"}], "detached": {}});
        assert_eq!(snapshot_outcome("study:e:s1", &sealed), Some(true));
        let running = json!({"studies": [], "detached": {"s1": {"status": "running"}}});
        assert_eq!(snapshot_outcome("study:e:s1", &running), None);
        let failed = json!({"studies": [], "detached": {"s1": {"status": "failed", "error": "x"}}});
        assert_eq!(snapshot_outcome("study:e:s1", &failed), Some(false));
        // Gone with the process that was running it: ended, not still waiting.
        assert_eq!(
            snapshot_outcome("study:e:s1", &json!({"studies": [], "detached": {}})),
            Some(false)
        );
        assert_eq!(
            snapshot_outcome("training:m:r", &json!({"status": "running"})),
            None
        );
        assert_eq!(
            snapshot_outcome("training:m:r", &json!({"status": "succeeded"})),
            Some(true)
        );
        assert_eq!(
            snapshot_outcome("training:m:r", &json!({"status": "failed"})),
            Some(false)
        );
        assert_eq!(
            super::study_entry("study:e:s1", &sealed),
            Some(json!({"study_id": "s1"}))
        );
    }

    #[test]
    fn a_campaign_is_over_when_it_has_a_terminal_phase_and_not_before() {
        let running = json!({"campaign_id": "c", "phase": "experiment", "terminal": null});
        assert_eq!(snapshot_outcome("campaign:c", &running), None);
        let done = json!({"phase": "converged", "terminal": "converged"});
        assert_eq!(snapshot_outcome("campaign:c", &done), Some(true));
        let halted = json!({"phase": "halted", "terminal": "halted"});
        assert_eq!(snapshot_outcome("campaign:c", &halted), Some(false));
        // A sweep still reads its status; a campaign's `status` key means nothing.
        assert_eq!(
            snapshot_outcome("sweep:1", &json!({"status": "completed"})),
            Some(true)
        );
        assert_eq!(
            snapshot_outcome("sweep:1", &json!({"status": "running"})),
            None
        );
        assert_eq!(
            snapshot_outcome("campaign:c", &json!({"status": "completed"})),
            None
        );
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

/// What [`AgentManager::pin_tool`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinOutcome {
    Pinned,
    NoSuchRun,
    AlreadyFinished(String),
    NotDrivenHere(String),
    /// The name is not in the catalogue; nothing was queued.
    UnknownTool,
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

#[cfg(test)]
mod one_runtime_tests {
    //! AGENT-010 §14.1 and S-2.10's acceptance, held structurally: the property is
    //! about which code exists, and a behavioural test would need a provider.

    use std::path::Path;

    fn code_of(src: &str) -> String {
        // Production code only: this module names the strings it forbids.
        let src = src.split("#[cfg(test)]").next().unwrap_or(src);
        src.lines()
            .filter(|l| {
                let t = l.trim_start();
                !t.starts_with("//") && !t.starts_with("///") && !t.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `crates/api/src/agent` contains one driver. The hosted loop is gone, and a
    /// second one appearing would be the two-runtimes problem this phase removed.
    #[test]
    fn the_agent_module_contains_one_driver() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent");
        let drivers: Vec<String> = std::fs::read_dir(&dir)
            .expect("the agent module exists")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".rs") && n.contains("driver"))
            .collect();
        assert_eq!(drivers, vec!["local_driver.rs".to_string()], "{drivers:?}");
    }

    /// `manager.rs` has no tier branch: every profile goes to the one driver.
    #[test]
    fn the_manager_does_not_branch_on_tier() {
        let code = code_of(include_str!("manager.rs"));
        for forbidden in ["tier.is_local()", "driver::drive(", "DriverParams"] {
            assert!(
                !code.contains(forbidden),
                "manager.rs still contains `{forbidden}`: a second path by tier"
            );
        }
        assert!(
            code.contains("local_driver::run("),
            "the one driver is reached"
        );
    }

    /// AGENT-005 P6: nothing the runtime writes can read `[elided` without a
    /// handle. The only elision left is the transcript's, which always leaves
    /// one, and the driver writes the file that handle names.
    #[test]
    fn no_runtime_code_writes_an_unrecoverable_elision() {
        let agent = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/agent");
        for entry in std::fs::read_dir(&agent).unwrap().filter_map(Result::ok) {
            let src = std::fs::read_to_string(entry.path()).unwrap();
            let code = code_of(&src);
            assert!(
                !code.contains("[elided"),
                "{} writes `[elided …]` with no handle",
                entry.path().display()
            );
        }
        let driver = include_str!("local_driver.rs");
        assert!(
            driver.contains("work/transcript/"),
            "the driver no longer writes the bodies the transcript's handles name"
        );
    }
}
