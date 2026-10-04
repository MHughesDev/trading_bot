//! Selection runs over HTTP (Set U U-6.3; FEAT-007 §4, §10).
//!
//! `POST /api/selections/run` runs a selection strategy now, or as of a past
//! instant, and journals it; `GET /api/selections/runs` lists the journal. The
//! web screener and the agent's `run_selection` tool both call this one route,
//! which calls `backtest::selection_run::run_selection` — so a person and the
//! agent asking about the same strategy at the same instant get the same list.
//! Who asked changes only the journal's `requested_by`.
//!
//! `POST /api/selections/watch`, `GET /api/selections/watches` and
//! `DELETE /api/selections/watch/{id}` (migration 0187): a screener someone
//! watches runs on its own `triggers.schedule`, through the same function, as
//! a scheduled run (`apps/platform/src/screener_jobs.rs`).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use backtest::selection_run::{self, RequestedBy, SelectionRunError};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{scopes, BearerToken};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSelection {
    /// A revision hash, or a strategy id (its head revision).
    pub strategy: String,
    /// Run as of this instant; now when absent.
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
}

/// A web session is a person; a service session is the agent.
fn caller(token: &BearerToken) -> RequestedBy {
    if token.scopes.iter().any(|s| s == scopes::WEB_FULL) {
        RequestedBy::Person
    } else {
        RequestedBy::Agent
    }
}

fn status_of(e: &SelectionRunError) -> StatusCode {
    match e {
        SelectionRunError::NotFound(_) => StatusCode::NOT_FOUND,
        SelectionRunError::NotSelection(_)
        | SelectionRunError::Compile(_)
        | SelectionRunError::Universe(_) => StatusCode::UNPROCESSABLE_ENTITY,
        SelectionRunError::Bars(_) => StatusCode::BAD_GATEWAY,
        SelectionRunError::Engine(_) | SelectionRunError::Db(_) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

/// `POST /api/selections/run`
pub async fn run(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<RunSelection>,
) -> impl IntoResponse {
    if !token.permits(scopes::DATA_READ) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "forbidden", "message": "running a selection reads market data"})),
        )
            .into_response();
    }
    let now = Utc::now();
    let as_of = body.as_of.unwrap_or(now);
    if as_of > now {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "as_of_in_future",
                        "message": "a selection can be run now or as of the past, not ahead of the data"})),
        )
            .into_response();
    }
    let report = match selection_run::run_selection(
        &state.pg,
        &state.clickhouse_url,
        &body.strategy,
        as_of,
        state.run_models.as_ref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                status_of(&e),
                Json(json!({"error": "selection_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    let requested_by = caller(&token);
    match selection_run::journal(&state.pg, &report, requested_by).await {
        Ok(run_id) => (
            StatusCode::OK,
            Json(json!({
                "run_id": run_id,
                "requested_by": requested_by,
                "strategy_ref": report.strategy_ref,
                "revision": report.revision,
                "as_of": report.as_of,
                "universe_point_in_time": report.universe_point_in_time,
                "digest": report.digest,
                "selection": report.run.selection,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "journal_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListRuns {
    #[serde(default)]
    pub strategy: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

const fn default_limit() -> i64 {
    50
}

/// `GET /api/selections/runs`
pub async fn list(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(q): Query<ListRuns>,
) -> impl IntoResponse {
    match selection_run::list(&state.pg, q.strategy.as_deref(), q.limit).await {
        Ok(rows) => (StatusCode::OK, Json(json!({ "runs": rows }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "list_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchSelection {
    /// A selection strategy's id (its head is pinned) or revision.
    pub strategy: String,
}

/// POST /api/selections/watch — run this selection on its own schedule.
pub async fn watch(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<WatchSelection>,
) -> impl IntoResponse {
    if !token.permits(scopes::DATA_READ) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "forbidden", "message": "watching a selection reads market data"})),
        )
            .into_response();
    }
    let user = token.user_id().to_string();
    match selection_run::create_watch(&state.pg, &user, &body.strategy).await {
        Ok(w) => (StatusCode::CREATED, Json(json!(w))).into_response(),
        Err(selection_run::WatchError::Selection(e)) => {
            let status = match e {
                SelectionRunError::NotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            };
            (status, Json(json!({"error": "invalid_selection", "message": e.to_string()})))
                .into_response()
        }
        Err(e @ selection_run::WatchError::NoSchedule(_)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "no_schedule", "message": e.to_string()})),
        )
            .into_response(),
        Err(e @ selection_run::WatchError::AlreadyWatching(_)) => (
            StatusCode::CONFLICT,
            Json(json!({"error": "already_watching", "message": e.to_string()})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "watch_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/selections/watches — the caller's watches.
pub async fn watches(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    match selection_run::watches_for(&state.pg, &token.user_id().to_string()).await {
        Ok(rows) => (StatusCode::OK, Json(json!({ "watches": rows }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "list_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/selections/watch/{id} — stop watching.
pub async fn unwatch(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<uuid::Uuid>,
) -> impl IntoResponse {
    match selection_run::stop_watch(&state.pg, &token.user_id().to_string(), id).await {
        Ok(true) => (StatusCode::OK, Json(json!({"watch_id": id, "stopped": true}))).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "message": "no running watch of yours with that id"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "stop_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(scopes: &[&str]) -> BearerToken {
        BearerToken {
            token: "t".into(),
            user_id: uuid::Uuid::nil(),
            scopes: scopes.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn a_web_session_is_a_person_and_a_service_session_is_the_agent() {
        assert_eq!(caller(&token(&[scopes::WEB_FULL])), RequestedBy::Person);
        assert_eq!(caller(&token(scopes::RESEARCH_SCOPES)), RequestedBy::Agent);
        assert!(
            token(scopes::RESEARCH_SCOPES).permits(scopes::DATA_READ),
            "the agent may run one"
        );
    }

    #[test]
    fn a_request_names_only_the_strategy_and_the_instant() {
        let ok: RunSelection =
            serde_json::from_value(json!({"strategy": "s", "as_of": "2025-03-01T00:00:00Z"}))
                .unwrap();
        assert_eq!(ok.strategy, "s");
        // Nothing a caller sends can change what is computed beyond these two.
        assert!(
            serde_json::from_value::<RunSelection>(json!({"strategy": "s", "top_n": 3})).is_err()
        );
    }
}
