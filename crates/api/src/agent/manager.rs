//! Agent run lifecycle: capacity, run rows, run-scoped tokens, spawn/cancel.
//!
//! Follows the platform's job pattern (ModelManager::start_train): insert a
//! status row, `tokio::spawn` a driver that updates it as it goes, poll over
//! REST. Runs do not survive a restart — `recover_orphans` marks them failed.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use chrono::{Duration as ChronoDuration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use cfg::model::AgentConfig;
use llm::{LlmClient, Provider};
use mcp_server_lib::ApiClient;

use crate::credentials::LlmCredentialStore;

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
    #[error("internal error: {0}")]
    Internal(String),
}

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
    /// Cancel flags for in-process runs.
    running: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
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
        }
    }

    /// Overrides the profile set. For tests, and for a caller that has already
    /// loaded one.
    #[must_use]
    pub fn with_profiles(mut self, profiles: Arc<harness::ProfileSet>) -> Self {
        self.profiles = profiles;
        self
    }

    /// Mark runs orphaned by a previous process as failed (same policy as
    /// backtests: jobs do not survive a platform restart).
    pub async fn recover_orphans(&self) {
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
    pub async fn send_message(
        self: &Arc<Self>,
        user_id: Uuid,
        conversation_id: Uuid,
        text: &str,
        creds: &LlmCredentialStore,
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
        let (turn_index, already_running) = conversations::next_turn(&self.pg, conversation_id)
            .await
            .map_err(|e| StartError::Internal(e.to_string()))?;
        if already_running {
            return Err(StartError::InvalidRequest(
                "this conversation is still working — wait for it, or stop it first".into(),
            ));
        }

        let provider = Provider::from_str(&convo.provider).map_err(StartError::InvalidRequest)?;
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
        sqlx::query(
            "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model,
                 constraints_json, conversation_id, turn_index)
             VALUES ($1, $2, 'queued', $3, $4, $5, '{}'::jsonb, $6, $7)",
        )
        .bind(run_id)
        .bind(user_id)
        .bind(text)
        .bind(&convo.provider)
        .bind(&convo.model)
        .bind(conversation_id)
        .bind(turn_index)
        .execute(&self.pg)
        .await
        .map_err(|e| StartError::Internal(e.to_string()))?;

        conversations::touch(&self.pg, conversation_id).await;

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

        let cancel = Arc::new(AtomicBool::new(false));
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
            let charter = prompt::system_prompt(&serde_json::json!({}));
            let params = LocalRunParams {
                run_id,
                user_id,
                conversation_id,
                goal: text.to_string(),
                charter,
                prior,
                profile,
                namespaces: vec!["core".into(), "discovery".into()],
                demand: harness::hardware::TaskDemand::MultiStep,
                api_base: base_url,
                service_token: run_token.clone(),
                workspace_root: workspace_root(),
            };
            tokio::spawn(async move {
                local_driver::run(pg.clone(), llm_client, params, cancel).await;
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
        if matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "fenced" | "refused"
        ) {
            return Ok(true);
        }

        let flag = self
            .running
            .lock()
            .expect("agent running lock")
            .get(&run_id)
            .cloned();
        match flag {
            Some(f) => f.store(true, std::sync::atomic::Ordering::SeqCst),
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
}
