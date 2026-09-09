//! HTTP surface for agent runs: start, list, detail, transcript, cancel.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::credentials::LlmCredentialStore;
use crate::{auth::BearerToken, state::AppState};

use super::manager::{StartError, StartRunRequest};

// ── POST /api/agent/runs ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StartRunBody {
    pub goal: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub constraints: Option<Value>,
    #[serde(default)]
    pub max_iterations: Option<i32>,
    #[serde(default)]
    pub max_total_tokens: Option<i64>,
    #[serde(default)]
    pub wallclock_budget_secs: Option<i64>,
}

pub async fn start_run(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<StartRunBody>,
) -> Response {
    let creds = LlmCredentialStore::new(state.pg.clone(), state.cred_crypto.clone());
    let req = StartRunRequest {
        goal: body.goal,
        provider: body.provider,
        model: body.model,
        constraints: body.constraints.unwrap_or_else(|| json!({})),
        max_iterations: body.max_iterations,
        max_total_tokens: body.max_total_tokens,
        wallclock_budget_secs: body.wallclock_budget_secs,
    };
    match state.agent.start_run(token.user_id(), req, &creds).await {
        Ok(run_id) => (StatusCode::CREATED, Json(json!({ "run_id": run_id }))).into_response(),
        Err(e) => {
            let (status, code) = match &e {
                StartError::InvalidRequest(_) => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request")
                }
                StartError::NotConfigured(_) => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "provider_not_configured")
                }
                StartError::Busy => (StatusCode::CONFLICT, "agent_busy"),
                StartError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            };
            (
                status,
                Json(json!({ "error": code, "message": e.to_string() })),
            )
                .into_response()
        }
    }
}

// ── GET /api/agent/runs ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

const RUN_COLUMNS: &str = "run_id, status, goal, provider, model, constraints_json, iterations, \
                           max_iterations, tokens_in, tokens_out, max_total_tokens, \
                           wallclock_budget_secs, error, summary, final_strategy_id, \
                           best_backtest_id, created_at, started_at, finished_at";

fn run_row_to_json(row: &sqlx::postgres::PgRow) -> Value {
    use sqlx::Row;
    let ts = |name: &str| -> Value {
        row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name)
            .ok()
            .flatten()
            .map(|t| json!(t.to_rfc3339()))
            .unwrap_or(Value::Null)
    };
    json!({
        "run_id": row.try_get::<Uuid, _>("run_id").map(|u| u.to_string()).unwrap_or_default(),
        "status": row.try_get::<String, _>("status").unwrap_or_default(),
        "goal": row.try_get::<String, _>("goal").unwrap_or_default(),
        "provider": row.try_get::<String, _>("provider").unwrap_or_default(),
        "model": row.try_get::<String, _>("model").unwrap_or_default(),
        "constraints": row.try_get::<Value, _>("constraints_json").unwrap_or(Value::Null),
        "iterations": row.try_get::<i32, _>("iterations").unwrap_or(0),
        "max_iterations": row.try_get::<i32, _>("max_iterations").unwrap_or(0),
        "tokens_in": row.try_get::<i64, _>("tokens_in").unwrap_or(0),
        "tokens_out": row.try_get::<i64, _>("tokens_out").unwrap_or(0),
        "max_total_tokens": row.try_get::<Option<i64>, _>("max_total_tokens").ok().flatten(),
        "wallclock_budget_secs": row.try_get::<i32, _>("wallclock_budget_secs").unwrap_or(0),
        "error": row.try_get::<Option<String>, _>("error").ok().flatten(),
        "summary": row.try_get::<Option<String>, _>("summary").ok().flatten(),
        "final_strategy_id": row.try_get::<Option<String>, _>("final_strategy_id").ok().flatten(),
        "best_backtest_id": row.try_get::<Option<Uuid>, _>("best_backtest_id").ok().flatten()
            .map(|u| json!(u.to_string())).unwrap_or(Value::Null),
        "created_at": ts("created_at"),
        "started_at": ts("started_at"),
        "finished_at": ts("finished_at"),
    })
}

pub async fn list_runs(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(25).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);
    let rows = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE user_id = $1
         ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(token.user_id())
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pg)
    .await;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs WHERE user_id = $1")
        .bind(token.user_id())
        .fetch_one(&state.pg)
        .await
        .unwrap_or(0);

    match rows {
        Ok(rows) => {
            let runs: Vec<Value> = rows.iter().map(run_row_to_json).collect();
            Json(json!({ "runs": runs, "total": total })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list agent runs failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "list_failed" })),
            )
                .into_response()
        }
    }
}

// ── GET /api/agent/runs/{id} ─────────────────────────────────────────────────

pub async fn get_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
) -> Response {
    let row = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE run_id = $1 AND user_id = $2"
    ))
    .bind(run_id)
    .bind(token.user_id())
    .fetch_optional(&state.pg)
    .await;

    match row {
        Ok(Some(row)) => Json(run_row_to_json(&row)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get agent run failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "get_failed" })),
            )
                .into_response()
        }
    }
}

// ── GET /api/agent/runs/{id}/messages ────────────────────────────────────────

#[derive(Deserialize)]
pub struct MessagesParams {
    #[serde(default)]
    after_seq: Option<i32>,
    #[serde(default)]
    limit: Option<i64>,
}

pub async fn get_messages(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
    Query(params): Query<MessagesParams>,
) -> Response {
    // Ownership check via the run row (messages have no user column).
    let owned: Option<(Uuid,)> =
        sqlx::query_as("SELECT run_id FROM agent_runs WHERE run_id = $1 AND user_id = $2")
            .bind(run_id)
            .bind(token.user_id())
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();
    if owned.is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response();
    }

    let after_seq = params.after_seq.unwrap_or(0);
    let limit = params.limit.unwrap_or(200).clamp(1, 500);
    // Row shape: (seq, kind, content_json, created_at).
    type MessageRow = (i32, String, Value, chrono::DateTime<chrono::Utc>);
    let rows: Result<Vec<MessageRow>, _> = sqlx::query_as(
        "SELECT seq, kind, content_json, created_at FROM agent_messages
         WHERE run_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3",
    )
    .bind(run_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => {
            let last_seq = rows.last().map(|(seq, _, _, _)| *seq).unwrap_or(after_seq);
            let messages: Vec<Value> = rows
                .into_iter()
                .map(|(seq, kind, content, created_at)| {
                    json!({
                        "seq": seq,
                        "kind": kind,
                        "content": content,
                        "created_at": created_at.to_rfc3339(),
                    })
                })
                .collect();
            Json(json!({ "messages": messages, "last_seq": last_seq })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "get agent messages failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "messages_failed" })),
            )
                .into_response()
        }
    }
}

// ── POST /api/agent/runs/{id}/cancel ─────────────────────────────────────────

pub async fn cancel_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
) -> Response {
    match state.agent.cancel(token.user_id(), run_id).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "cancel agent run failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "cancel_failed" })),
            )
                .into_response()
        }
    }
}
