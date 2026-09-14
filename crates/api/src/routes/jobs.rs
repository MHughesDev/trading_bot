//! Durable job service and artifact HTTP API (COMP-005 §11, Set L Phase 1).
//!
//! Thin translation only: manifests in, handles out. Everything that matters —
//! idempotency, trial counting, leasing — lives in `jobs::JobStore`, so a caller
//! cannot reach the interesting behaviour by going around this layer.

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use jobs::{JobKind, JobState, JobStoreError, Submission, SubmittedBy};

use crate::{auth::BearerToken, state::AppState};

fn map_err(e: JobStoreError) -> axum::response::Response {
    let (status, code) = match &e {
        JobStoreError::ExperimentRequired(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "experiment_required")
        }
        JobStoreError::MaxGpuHoursRequired(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "max_gpu_hours_required")
        }
        JobStoreError::BudgetExhausted(_) => (StatusCode::CONFLICT, "budget_exhausted"),
        JobStoreError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
        JobStoreError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
        JobStoreError::Json(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_json"),
        JobStoreError::Sqlx(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        JobStoreError::Ledger(ledger::LedgerError::Backend(_)) => (StatusCode::INTERNAL_SERVER_ERROR, "ledger_unavailable"),
        JobStoreError::Ledger(_) => (StatusCode::UNPROCESSABLE_ENTITY, "trial_refused"),
    };
    (
        status,
        Json(json!({ "error": code, "message": e.to_string() })),
    )
        .into_response()
}

/// A job rendered for the wire.
fn job_json(job: &jobs::Job) -> serde_json::Value {
    json!({
        "job_id": job.job_id,
        "kind": job.kind.as_str(),
        "project_id": job.project_id,
        "experiment_id": job.experiment_id,
        "parent_job_id": job.parent_job_id,
        "queue": job.queue.as_str(),
        "worker_class": job.worker_class.as_str(),
        "state": job.state.as_str(),
        "progress": job.progress,
        "result_summary": job.result_summary,
        "result": job.result,
        "error": job.error,
        "attempts": job.attempts,
        "created_at": job.created_at,
        "started_at": job.started_at,
        "finished_at": job.finished_at,
    })
}

#[derive(Debug, Deserialize)]
pub struct SubmitJobRequest {
    pub kind: String,
    pub manifest: serde_json::Value,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub experiment_id: Option<String>,
    #[serde(default)]
    pub priority: Option<i16>,
    #[serde(default)]
    pub code_hashes: Vec<String>,
    #[serde(default)]
    pub data_snapshot_id: Option<String>,
}

/// POST /api/jobs — submit work.
///
/// `201` for a new job, `200` when an identical job already existed (JB-02). The
/// distinction matters to a caller deciding whether it just spent a trial.
pub async fn submit_job(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<SubmitJobRequest>,
) -> impl IntoResponse {
    let Some(store) = state.jobs.as_ref() else {
        return service_unavailable();
    };
    let Some(kind) = JobKind::parse(&req.kind) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "unknown_kind",
                "message": format!("unknown job kind {:?}", req.kind),
                "known": JobKind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
            })),
        )
            .into_response();
    };

    let mut submission = Submission::new(kind, token.user_id(), req.manifest);
    submission.project_id = req.project_id;
    submission.experiment_id = req.experiment_id;
    submission.submitted_by = SubmittedBy::User;
    submission.priority = req.priority.unwrap_or(5).clamp(0, 9);
    submission.code_hashes = req.code_hashes;
    if let Some(snapshot) = req.data_snapshot_id {
        submission.data_snapshot_id = snapshot;
    }

    match store.submit(submission).await {
        Ok(result) => {
            let status = if result.deduplicated {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (
                status,
                Json(json!({
                    "job_id": result.job_id,
                    "deduplicated": result.deduplicated,
                    "state": result.state.as_str(),
                })),
            )
                .into_response()
        }
        Err(e) => map_err(e),
    }
}

/// GET /api/jobs/{id}
pub async fn get_job(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.jobs.as_ref() else {
        return service_unavailable();
    };
    match store.get(&job_id).await {
        Ok(job) => (StatusCode::OK, Json(job_json(&job))).into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListJobsQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/jobs
pub async fn list_jobs(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(q): Query<ListJobsQuery>,
) -> impl IntoResponse {
    let Some(store) = state.jobs.as_ref() else {
        return service_unavailable();
    };
    let job_state = q.state.as_deref().and_then(JobState::parse);
    let kind = q.kind.as_deref().and_then(JobKind::parse);

    match store
        .list(q.project_id, job_state, kind, q.limit.unwrap_or(50))
        .await
    {
        Ok(list) => (
            StatusCode::OK,
            Json(json!({
                "jobs": list.iter().map(job_json).collect::<Vec<_>>(),
                "count": list.len(),
            })),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

/// POST /api/jobs/{id}/cancel
pub async fn cancel_job(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = state.jobs.as_ref() else {
        return service_unavailable();
    };
    match store.cancel(&job_id).await {
        Ok(state) => (
            StatusCode::OK,
            Json(json!({"job_id": job_id, "state": state.as_str()})),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub after_id: Option<i64>,
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/jobs/events — poll-style event feed, resumable by `after_id` (JB-06).
///
/// Deliberately a plain JSON page rather than SSE at this layer: `tbot jobs wait`
/// consumes it with a cursor, and a resumable cursor is what makes a wait survive a
/// dropped connection. The SSE lane is added alongside it in Phase 4 for the UI.
pub async fn job_events(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(q): Query<EventsQuery>,
) -> impl IntoResponse {
    let Some(store) = state.jobs.as_ref() else {
        return service_unavailable();
    };
    match store
        .events_after(
            q.after_id.unwrap_or(0),
            q.project_id,
            q.limit.unwrap_or(200),
        )
        .await
    {
        Ok(events) => {
            let last = events.last().map(|(id, _, _, _)| *id).unwrap_or(0);
            (
                StatusCode::OK,
                Json(json!({
                    "events": events.iter().map(|(id, job_id, kind, payload)| json!({
                        "id": id, "job_id": job_id, "kind": kind, "payload": payload,
                    })).collect::<Vec<_>>(),
                    "last_id": last,
                })),
            )
                .into_response()
        }
        Err(e) => map_err(e),
    }
}

// ── artifacts ────────────────────────────────────────────────────────────────

/// GET /api/artifacts/{handle} — manifest and summary, never the bytes.
pub async fn get_artifact(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(handle): Path<String>,
) -> impl IntoResponse {
    let Some(registry) = state.artifacts.as_ref() else {
        return service_unavailable();
    };
    match registry.get(&handle).await {
        Ok(Some(a)) => (
            StatusCode::OK,
            Json(json!({
                "handle": a.handle,
                "type": a.artifact_type,
                "project_id": a.project_id,
                "size_bytes": a.size_bytes,
                "sha256": a.sha256,
                "manifest": a.manifest,
                "producer_job": a.producer_job,
                "pinned": a.pinned,
                "expires_at": a.expires_at,
                "created_at": a.created_at,
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "handle": handle})),
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ArtifactContentQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

/// GET /api/artifacts/{handle}/content — the bytes.
pub async fn get_artifact_content(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(handle): Path<String>,
    Query(q): Query<ArtifactContentQuery>,
) -> impl IntoResponse {
    let Some(registry) = state.artifacts.as_ref() else {
        return service_unavailable();
    };
    match registry.read(&handle, q.project_id).await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(e) => map_err(e),
    }
}

fn service_unavailable() -> axum::response::Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "jobs_unavailable",
            "message": "the job service is not configured on this instance",
        })),
    )
        .into_response()
}
