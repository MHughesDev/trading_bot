//! Agent session orchestrator (AGENT-001 §4, §8, §11, ADR-0024).
//!
//! Owns the lifecycle of a research session: mint a scoped token, start the
//! project's container, stream its events, and clean up. The container is where the
//! agent's freedom lives; this module is where its authority is decided.
//!
//! The single most important function here is [`SessionOrchestrator::mint_token`].
//! Everything the agent can do follows from the scopes it attaches, and everything
//! the agent cannot do follows from the ones it refuses to.

use std::process::Stdio;

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use sqlx::{PgPool, Row};
use tokio::io::{AsyncBufReadExt, BufReader};
use uuid::Uuid;

use crate::auth::scopes;
use crate::projects::{Project, ProjectError, ProjectStore};

/// How long a session token lives. Long enough for a research session, short enough
/// that a leaked one expires on its own.
pub const SESSION_TOKEN_HOURS: i64 = 24;

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("project: {0}")]
    Project(#[from] ProjectError),
    #[error("container: {0}")]
    Container(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub session_id: Uuid,
    pub project_id: Uuid,
    pub user_id: Uuid,
    pub state: String,
    pub container_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A freshly minted session token. The plaintext is returned once and never stored
/// in a form this process can read back.
pub struct MintedToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub scopes: Vec<String>,
}

pub struct SessionOrchestrator {
    pool: PgPool,
    image: String,
}

impl SessionOrchestrator {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            image: std::env::var("TBOT_AGENT_IMAGE").unwrap_or_else(|_| "tbot-agent:dev".into()),
        }
    }

    /// Mints a project-scoped session token (AGENT-001 §6, ADR-0025).
    ///
    /// The scope list is [`scopes::RESEARCH_SCOPES`] and nothing else. It is not a
    /// parameter, and there is no variant of this function that takes one: an
    /// orchestrator that could be *asked* for extra authority would eventually be
    /// asked for it by a bug. Widening what an agent may do requires editing that
    /// constant, which is a reviewable change, and the database refuses the
    /// forbidden set independently (migration 0038).
    pub async fn mint_token(
        &self,
        user_id: Uuid,
        project_id: Uuid,
    ) -> Result<MintedToken, OrchestratorError> {
        let granted: Vec<String> = scopes::RESEARCH_SCOPES
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Belt and braces against a future edit to the constant.
        scopes::validate_agent_scopes(&granted).map_err(OrchestratorError::Invalid)?;

        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let expires_at = Utc::now() + Duration::hours(SESSION_TOKEN_HOURS);

        sqlx::query(
            "INSERT INTO sessions (token, user_id, kind, label, scopes, project_id, expires_at) \
             VALUES ($1,$2,'service',$3,$4,$5,$6)",
        )
        .bind(&token)
        .bind(user_id)
        .bind(format!("agent session for project {project_id}"))
        .bind(&granted)
        .bind(project_id)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        Ok(MintedToken {
            token,
            expires_at,
            scopes: granted,
        })
    }

    /// Revokes a session token. Called when a session ends, so that a container that
    /// outlives its session cannot keep working.
    pub async fn revoke_token(&self, token: &str) -> Result<(), OrchestratorError> {
        sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        project_id: Uuid,
    ) -> Result<Session, OrchestratorError> {
        let store = ProjectStore::new(self.pool.clone());
        let project = store.get(project_id).await?;
        if project.user_id != user_id {
            return Err(OrchestratorError::Project(ProjectError::NotFound(
                project_id.to_string(),
            )));
        }

        let session_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO agent_sessions (session_id, project_id, user_id, state) \
             VALUES ($1,$2,$3,'starting')",
        )
        .bind(session_id)
        .bind(project_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(Session {
            session_id,
            project_id,
            user_id,
            state: "starting".into(),
            container_id: None,
            created_at: Utc::now(),
        })
    }

    pub async fn get_session(&self, session_id: Uuid) -> Result<Session, OrchestratorError> {
        let row = sqlx::query("SELECT * FROM agent_sessions WHERE session_id=$1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| OrchestratorError::Invalid(format!("no session {session_id}")))?;
        Ok(Session {
            session_id: row.get("session_id"),
            project_id: row.get("project_id"),
            user_id: row.get("user_id"),
            state: row.get("state"),
            container_id: row.get("container_id"),
            created_at: row.get("created_at"),
        })
    }

    pub async fn set_state(&self, session_id: Uuid, state: &str) -> Result<(), OrchestratorError> {
        sqlx::query("UPDATE agent_sessions SET state=$2 WHERE session_id=$1")
            .bind(session_id)
            .bind(state)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Records one event from the container's stream.
    pub async fn record_event(
        &self,
        session_id: Uuid,
        seq: i32,
        kind: &str,
        payload: serde_json::Value,
    ) -> Result<(), OrchestratorError> {
        sqlx::query(
            "INSERT INTO agent_events (session_id, seq, kind, payload) VALUES ($1,$2,$3,$4) \
             ON CONFLICT (session_id, seq) DO NOTHING",
        )
        .bind(session_id)
        .bind(seq)
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The `docker run` arguments for a session's container (AGENT-001 §7).
    ///
    /// Split out from the spawn so the security-relevant flags can be asserted in a
    /// test without a Docker daemon. Every one of them is load-bearing:
    /// no privileges, no capabilities, a read-only root, and a network that has no
    /// default route off it.
    pub fn container_args(
        &self,
        project: &Project,
        session_id: Uuid,
        token: &str,
        api_url: &str,
        task: &str,
    ) -> Vec<String> {
        let volume = format!("tbot-ws-{}", project.project_id);
        vec![
            "run".into(),
            "--rm".into(),
            "--name".into(),
            format!("tbot-agent-{session_id}"),
            // No privilege escalation, no capabilities, read-only root.
            "--security-opt".into(),
            "no-new-privileges".into(),
            "--cap-drop".into(),
            "ALL".into(),
            "--read-only".into(),
            "--user".into(),
            "10001:10001".into(),
            // The only writable paths.
            "-v".into(),
            format!("{volume}:/workspace"),
            "--tmpfs".into(),
            "/tmp:size=2g".into(),
            "--tmpfs".into(),
            "/home/agent/.cache:size=1g".into(),
            // Resource ceilings, so one runaway session cannot take the box down.
            "--cpus".into(),
            "4".into(),
            "--memory".into(),
            "8g".into(),
            "--pids-limit".into(),
            "512".into(),
            // An `internal: true` network: the platform and a package mirror
            // resolve, nothing else does. This is the containment that matters —
            // the SDK's own sandbox is a second layer inside it.
            "--network".into(),
            "tbot-agent-net".into(),
            "-e".into(),
            format!("TBOT_API_URL={api_url}"),
            "-e".into(),
            format!("TBOT_TOKEN={token}"),
            "-e".into(),
            format!("TBOT_PROJECT_ID={}", project.project_id),
            "-e".into(),
            format!("TBOT_SESSION_ID={session_id}"),
            "-e".into(),
            format!(
                "TBOT_MODEL={}",
                std::env::var("TBOT_MODEL").unwrap_or_else(|_| "claude-opus-5".into())
            ),
            self.image.clone(),
            task.to_string(),
        ]
    }

    /// Starts the container and streams its line-delimited events into
    /// `agent_events` until it exits.
    pub async fn run_session(
        &self,
        session_id: Uuid,
        api_url: &str,
        task: &str,
    ) -> Result<i32, OrchestratorError> {
        let session = self.get_session(session_id).await?;
        let project = ProjectStore::new(self.pool.clone())
            .get(session.project_id)
            .await?;
        let minted = self.mint_token(session.user_id, session.project_id).await?;

        let args = self.container_args(&project, session_id, &minted.token, api_url, task);
        self.set_state(session_id, "running").await?;

        let mut child = tokio::process::Command::new("docker")
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| OrchestratorError::Container(format!("spawning docker: {e}")))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| OrchestratorError::Container("no stdout".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut seq = 0i32;

        while let Ok(Some(line)) = lines.next_line().await {
            let payload: serde_json::Value =
                serde_json::from_str(&line).unwrap_or_else(|_| serde_json::json!({"raw": line}));
            let kind = payload
                .get("kind")
                .and_then(|k| k.as_str())
                .unwrap_or("raw")
                .to_string();
            self.record_event(session_id, seq, &kind, payload).await?;
            seq += 1;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| OrchestratorError::Container(format!("waiting on docker: {e}")))?;

        // The token dies with the session, whatever happened to the container. A
        // container that outlived its session would otherwise keep full research
        // authority for the rest of the token's 24 hours.
        self.revoke_token(&minted.token).await?;
        let final_state = if status.success() {
            "succeeded"
        } else {
            "failed"
        };
        self.set_state(session_id, final_state).await?;

        sqlx::query("UPDATE agent_sessions SET ended_at = now() WHERE session_id=$1")
            .bind(session_id)
            .execute(&self.pool)
            .await?;

        Ok(status.code().unwrap_or(-1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::ProjectKind;

    fn project() -> Project {
        Project {
            project_id: Uuid::nil(),
            user_id: Uuid::nil(),
            kind: ProjectKind::Research,
            name: "t".into(),
            goal: None,
            instruments: vec![],
            research_cutoff: Some(Utc::now() - Duration::days(90)),
            status: "active".into(),
            created_at: Utc::now(),
        }
    }

    fn args() -> Vec<String> {
        let orchestrator = SessionOrchestrator {
            // Never connected: `container_args` touches no database.
            pool: PgPool::connect_lazy("postgres://invalid/invalid").expect("lazy"),
            image: "tbot-agent:test".into(),
        };
        orchestrator.container_args(
            &project(),
            Uuid::nil(),
            "session-token",
            "http://platform:7080",
            "do research",
        )
    }

    #[test]
    fn the_container_runs_unprivileged_with_a_read_only_root() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let args = args();
        let joined = args.join(" ");
        assert!(joined.contains("--security-opt no-new-privileges"));
        assert!(joined.contains("--cap-drop ALL"));
        assert!(joined.contains("--read-only"));
        assert!(joined.contains("--user 10001:10001"));
    }

    #[test]
    fn the_container_is_on_the_internal_network_only() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let joined = args().join(" ");
        // The whole containment story rests on this: a network with no default
        // route, on which only the platform resolves.
        assert!(joined.contains("--network tbot-agent-net"));
        assert!(
            !joined.contains("--network host"),
            "host networking would put the agent on the machine's network"
        );
    }

    #[test]
    fn the_container_has_resource_ceilings() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let joined = args().join(" ");
        for flag in ["--cpus", "--memory", "--pids-limit"] {
            assert!(
                joined.contains(flag),
                "{flag} missing: one runaway session could take the box down"
            );
        }
    }

    #[test]
    fn no_provider_credential_reaches_the_container() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let joined = args().join(" ");
        // The container gets a session token and the proxy's address. An API key
        // here would defeat the entire point of the proxy.
        assert!(joined.contains("TBOT_TOKEN=session-token"));
        for forbidden in ["ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN", "CRED_KEK"] {
            assert!(
                !joined.contains(forbidden),
                "{forbidden} must never be passed into the sandbox"
            );
        }
    }

    #[test]
    fn only_the_workspace_volume_is_mounted() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let _guard = runtime.enter();
        let args = args();
        let mounts: Vec<&String> = args
            .iter()
            .enumerate()
            .filter(|(i, _)| *i > 0 && args[i - 1] == "-v")
            .map(|(_, v)| v)
            .collect();
        assert_eq!(mounts.len(), 1, "exactly one bind: {mounts:?}");
        assert!(mounts[0].ends_with(":/workspace"));
        // A docker socket mount would let the agent escape the sandbox entirely.
        assert!(!mounts[0].contains("docker.sock"));
    }
}
