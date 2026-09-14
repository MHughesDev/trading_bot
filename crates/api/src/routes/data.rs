//! Data API v2 (DATA-005 §4, §5; DA-01…DA-09, DA-15).
//!
//! This is where the research cutoff is enforced. Every read resolves the caller's
//! project, clips the window to that project's horizon, and records what it did in
//! the response manifest. The agent cannot bypass it, because there is no other way
//! to reach bars — and that, not any instruction in a prompt, is what makes the
//! holdout mean something (D-10, ADR-0025).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::projects::{Project, ProjectError};
use crate::{auth::BearerToken, state::AppState};

fn map_project_err(e: ProjectError) -> axum::response::Response {
    let (status, code) = match &e {
        ProjectError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        ProjectError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
        ProjectError::LiveDataDeskOnly(_) => (StatusCode::FORBIDDEN, "live_data_desk_only"),
        ProjectError::DeskExploratoryOnly(_) => (StatusCode::FORBIDDEN, "desk_exploratory_only"),
        ProjectError::Sqlx(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    };
    let mut body = json!({ "error": code, "message": e.to_string() });
    // A refusal that does not say what to do instead just invites a retry.
    if matches!(e, ProjectError::LiveDataDeskOnly(_)) {
        body["fix"] = json!("re-issue this request against your Desk project");
    }
    (status, Json(body)).into_response()
}

/// Resolves the project a data request is made in.
///
/// Defaults to the caller's Desk when no project is named. That default is
/// deliberately the *restrictive* one for research purposes: the Desk can see live
/// data but cannot run confirmatory work, so an unscoped read can never quietly
/// consume a research project's holdout.
async fn resolve_project(
    state: &AppState,
    user_id: Uuid,
    project_id: Option<Uuid>,
) -> Result<Project, ProjectError> {
    let store = crate::projects::ProjectStore::new(state.pg.clone());
    match project_id {
        Some(id) => {
            let project = store.get(id).await?;
            if project.user_id != user_id {
                // Reported as missing rather than forbidden: a caller should not be
                // able to discover which project ids exist.
                return Err(ProjectError::NotFound(id.to_string()));
            }
            Ok(project)
        }
        None => store.desk(user_id).await,
    }
}

#[derive(Debug, Deserialize)]
pub struct BarsQuery {
    pub instrument: String,
    #[serde(default = "default_timeframe")]
    pub tf: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub as_of: Option<DateTime<Utc>>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

fn default_timeframe() -> String {
    "1m".into()
}

/// GET /api/data/bars — point-in-time bars, clipped to the project's cutoff.
pub async fn get_bars(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<BarsQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };

    // The three clips that matter, in order. `as_of` and `end` are separate concepts —
    // `end` bounds which bars, `as_of` bounds which *revisions* of them — and both
    // must respect the horizon or a late correction could leak the future.
    let (effective_end, cutoff_applied) = project.clip(q.end);
    let requested_as_of = q.as_of.unwrap_or(effective_end);
    let (effective_as_of, _) = project.clip(requested_as_of);

    if q.start >= effective_end {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "empty_window",
                "message": format!(
                    "start {} is at or after the effective end {}",
                    q.start, effective_end
                ),
                "fix": if cutoff_applied.is_some() {
                    "the window was clipped to this project's research cutoff; \
                     request an earlier window, or use the Desk for recent data"
                } else {
                    "request a window with start < end"
                },
                "cutoff_applied": cutoff_applied,
            })),
        )
            .into_response();
    }

    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(effective_as_of);
    let timeframe =
        match <domain::payloads::bar::Timeframe as backtest::types::TimeframeExt>::from_key(&q.tf) {
            Some(tf) => tf,
            None => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "unknown_timeframe",
                        "message": format!("unknown timeframe {:?}", q.tf),
                        "known": ["1s", "1m", "5m", "15m", "1h", "4h", "1d"],
                    })),
                )
                    .into_response()
            }
        };

    let bars = match store
        .load_bars(&q.instrument, timeframe, q.start, effective_end)
        .await
    {
        Ok(bars) => bars,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "query_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };

    let limit = q.limit.unwrap_or(5_000).clamp(1, 50_000) as usize;
    let truncated = bars.len() > limit;
    let rows: Vec<serde_json::Value> = bars
        .iter()
        .take(limit)
        .map(|b| {
            json!({
                "ts_ns": b.ts_ns,
                "open": b.open.to_string(),
                "high": b.high.to_string(),
                "low": b.low.to_string(),
                "close": b.close.to_string(),
                "volume": b.volume.to_string(),
                "trade_count": b.trade_count,
            })
        })
        .collect();

    // Every read is exploration, and exploration is logged rather than counted
    // (D-13, JB-11). The variables are recorded so the hypothesis registry can tell
    // later whether a hypothesis was formed before or after looking.
    if let Some(jobs) = state.jobs.as_ref() {
        let _ = jobs
            .log_exploration(jobs::ExplorationEntry {
                project_id: project.project_id,
                user_id: token.user_id(),
                session_id: None,
                source: if project.is_desk() {
                    "desk"
                } else {
                    "data_api"
                },
                instruments: vec![q.instrument.clone()],
                timeframe: Some(q.tf.clone()),
                window_start: Some(q.start),
                window_end: Some(effective_end),
                variables: vec![
                    "open".into(),
                    "high".into(),
                    "low".into(),
                    "close".into(),
                    "volume".into(),
                ],
                description: format!(
                    "bars {} {} [{}, {})",
                    q.instrument, q.tf, q.start, effective_end
                ),
                handle: None,
            })
            .await;
    }

    (
        StatusCode::OK,
        Json(json!({
            "manifest": {
                "type": "bars",
                "instrument": q.instrument,
                "timeframe": q.tf,
                "window": {"start": q.start, "end": effective_end},
                "as_of": effective_as_of,
                // Stated, not hidden: the caller asked for more than this project may
                // see, and the manifest says exactly where the line was drawn.
                "cutoff_applied": cutoff_applied,
                "project_id": project.project_id,
                "project_kind": project.kind.as_str(),
                "rows": rows.len(),
                "truncated": truncated,
            },
            "bars": rows,
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/data/catalog — instruments with coverage, as the project may see them.
pub async fn get_catalog(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<CatalogQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };

    // Coverage is computed through the same horizon as the data itself. A catalogue
    // that advertised bars the project cannot read would be a slow leak of the
    // holdout: "history ends on the 4th" is information about the future.
    let store = backtest::store::BarStore::connect(&state.clickhouse_url).as_of(project.horizon());
    match store.list_coverage().await {
        Ok(coverage) => (
            StatusCode::OK,
            Json(json!({
                "project_id": project.project_id,
                "project_kind": project.kind.as_str(),
                "horizon": project.horizon(),
                "instruments": coverage.iter().map(|c| json!({
                    "instrument_id": c.instrument_id,
                    "timeframe": c.timeframe,
                    "bars": c.bars,
                    "first_ns": c.first_ns,
                    "last_ns": c.last_ns,
                })).collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "query_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/data/live/{instrument} — Desk only (DA-15).
pub async fn get_live(
    State(state): State<AppState>,
    token: BearerToken,
    Path(instrument): Path<String>,
    Query(q): Query<CatalogQuery>,
) -> impl IntoResponse {
    let project = match resolve_project(&state, token.user_id(), q.project_id).await {
        Ok(p) => p,
        Err(e) => return map_project_err(e),
    };
    if let Err(e) = project.allow_live() {
        return map_project_err(e);
    }

    let store = backtest::store::BarStore::connect(&state.clickhouse_url);
    let timeframe = domain::payloads::bar::Timeframe::Minutes1;
    match store.last_bar_time(&instrument, timeframe).await {
        Ok(Some(last)) => {
            let age_s = (Utc::now() - last).num_seconds();
            (
                StatusCode::OK,
                Json(json!({
                    "instrument": instrument,
                    "last_bar_time": last,
                    "last_bar_age_s": age_s,
                    "project_id": project.project_id,
                })),
            )
                .into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "no_data", "instrument": instrument})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "query_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}

// ── projects ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub instruments: Vec<String>,
    #[serde(default)]
    pub research_cutoff: Option<DateTime<Utc>>,
    #[serde(default)]
    pub holdout_days: Option<i64>,
}

/// POST /api/projects — create a research project.
pub async fn create_project(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    let store = crate::projects::ProjectStore::new(state.pg.clone());
    match store
        .create_research(
            token.user_id(),
            &req.name,
            req.goal.as_deref(),
            &req.instruments,
            req.research_cutoff,
            req.holdout_days,
        )
        .await
    {
        Ok(p) => (StatusCode::CREATED, Json(p)).into_response(),
        Err(e) => map_project_err(e),
    }
}

/// GET /api/projects — the caller's projects, Desk included (created on demand).
pub async fn list_projects(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let store = crate::projects::ProjectStore::new(state.pg.clone());
    // Touch the Desk so it exists from the first listing rather than appearing later.
    let _ = store.desk(token.user_id()).await;
    match store.list(token.user_id()).await {
        Ok(list) => (StatusCode::OK, Json(json!({"projects": list}))).into_response(),
        Err(e) => map_project_err(e),
    }
}

/// GET /api/projects/{id}
pub async fn get_project(
    State(state): State<AppState>,
    token: BearerToken,
    Path(project_id): Path<Uuid>,
) -> impl IntoResponse {
    match resolve_project(&state, token.user_id(), Some(project_id)).await {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => map_project_err(e),
    }
}
