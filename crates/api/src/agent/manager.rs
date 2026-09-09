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
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use cfg::model::AgentConfig;
use llm::{LlmClient, Provider};
use mcp_server_lib::ApiClient;

use crate::credentials::LlmCredentialStore;

use super::driver::{self, DriverParams};

/// Request to start a run (already validated by the route layer).
pub struct StartRunRequest {
    pub goal: String,
    pub provider: String,
    pub model: String,
    pub constraints: Value,
    pub max_iterations: Option<i32>,
    pub max_total_tokens: Option<i64>,
    pub wallclock_budget_secs: Option<i64>,
}

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

pub struct AgentManager {
    pg: PgPool,
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
            base_url,
            cfg,
            running: Mutex::new(HashMap::new()),
        }
    }

    /// Mark runs orphaned by a previous process as failed (same policy as
    /// backtests: jobs do not survive a platform restart).
    pub async fn recover_orphans(&self) {
        match sqlx::query(
            "UPDATE agent_runs SET status = 'failed',
                 error = 'interrupted by platform restart', finished_at = now()
             WHERE status IN ('queued', 'running', 'waiting_backtest')",
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

    /// Validate, persist, and spawn a run. Returns its id.
    pub async fn start_run(
        self: &Arc<Self>,
        user_id: Uuid,
        req: StartRunRequest,
        creds: &LlmCredentialStore,
    ) -> Result<Uuid, StartError> {
        let provider = Provider::from_str(&req.provider).map_err(StartError::InvalidRequest)?;
        if req.goal.trim().is_empty() {
            return Err(StartError::InvalidRequest("goal must not be empty".into()));
        }
        if req.model.trim().is_empty() {
            return Err(StartError::InvalidRequest("model must not be empty".into()));
        }
        if self.active_count() >= self.cfg.max_concurrent_runs {
            return Err(StartError::Busy);
        }

        // Resolve the provider credential up front so a bad config fails the
        // request, not the run.
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
        let max_iterations = req
            .max_iterations
            .unwrap_or(self.cfg.default_max_iterations)
            .clamp(1, 200);
        let wallclock_budget_secs = req
            .wallclock_budget_secs
            .unwrap_or(self.cfg.default_wallclock_budget_secs)
            .clamp(60, 86_400);

        sqlx::query(
            "INSERT INTO agent_runs (run_id, user_id, status, goal, provider, model,
                 constraints_json, max_iterations, max_total_tokens, wallclock_budget_secs)
             VALUES ($1, $2, 'queued', $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(run_id)
        .bind(user_id)
        .bind(req.goal.trim())
        .bind(provider.as_str())
        .bind(req.model.trim())
        .bind(&req.constraints)
        .bind(max_iterations)
        .bind(req.max_total_tokens)
        .bind(wallclock_budget_secs as i32)
        .execute(&self.pg)
        .await
        .map_err(|e| StartError::Internal(e.to_string()))?;

        // Run-scoped service token: tool calls run as this user through the
        // normal API auth path, and the run survives a UI logout.
        let run_token = crate::auth::handlers::new_session_token();
        let expires_at = Utc::now() + ChronoDuration::seconds(wallclock_budget_secs + 3600);
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
        let params = DriverParams {
            run_id,
            user_id,
            goal: req.goal.trim().to_string(),
            model: req.model.trim().to_string(),
            constraints: req.constraints,
            max_iterations,
            max_total_tokens: req.max_total_tokens,
            wallclock_budget_secs,
            llm_max_tokens_per_call: self.cfg.llm_max_tokens_per_call,
        };
        let pg = self.pg.clone();
        let api = ApiClient::new(self.base_url.clone(), run_token.clone());
        tokio::spawn(async move {
            driver::drive(&pg, api, llm_client, params, cancel).await;
            // Best-effort cleanup of the run-scoped token.
            let _ = sqlx::query("DELETE FROM sessions WHERE token = $1")
                .bind(&run_token)
                .execute(&pg)
                .await;
            manager.finish(run_id);
        });

        Ok(run_id)
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
        if matches!(status.as_str(), "completed" | "failed" | "cancelled") {
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
                     WHERE run_id = $1 AND status IN ('queued','running','waiting_backtest')",
                )
                .bind(run_id)
                .execute(&self.pg)
                .await?;
            }
        }
        Ok(true)
    }
}
