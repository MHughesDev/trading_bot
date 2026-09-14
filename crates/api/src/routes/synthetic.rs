//! The synthetic venue's HTTP surface (DATA-005 §9, DA-13).
//!
//! Three endpoints and one asymmetry. Creating an instrument and listing the
//! catalogue are open to any authenticated caller — synthetic instruments are meant
//! to flow through every other endpoint exactly like real ones. Reading what was
//! *planted* needs `evals.truth`, a scope migration 0039 forbids on any
//! project-bound session, so an agent has no token that can reach it.
//!
//! Without that asymmetry the `noise` and `planted_edge` suites measure nothing: an
//! agent that can read `{"mechanism": "ar1_return_autocorrelation", "strength":
//! 0.05}` does not have to find the edge, and its power curve is a report on the
//! catalogue API.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use jobs::{JobKind, Submission, SubmittedBy};

use crate::{auth::BearerToken, state::AppState};

/// The scope that reads the answer key. Never granted to an agent session.
pub const EVALS_TRUTH: &str = "evals.truth";

#[derive(Debug, Deserialize)]
pub struct CreateSyntheticRequest {
    pub generator: String,
    #[serde(default)]
    pub params: Option<Value>,
    pub seed: u64,
    #[serde(default = "default_tf")]
    pub tf: String,
    pub length: u64,
    #[serde(default)]
    pub start: Option<DateTime<Utc>>,
    /// The project the generated instrument is attributed to. Optional: the bars
    /// themselves are visible to anyone, since a synthetic series has no holdout to
    /// protect.
    #[serde(default)]
    pub project_id: Option<Uuid>,
}

fn default_tf() -> String {
    "1h".into()
}

/// POST /api/data/synthetic — submit a `simulate_paths` job (DATA-005 §9).
///
/// Returns the job handle and the instrument id the job will claim. The id is
/// derived from `(generator, seed)` and is therefore knowable before the job runs,
/// which is what lets a caller submit the generation and the read in one pass
/// instead of polling to discover a name.
pub async fn create_synthetic(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateSyntheticRequest>,
) -> impl IntoResponse {
    let Some(store) = state.jobs.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "jobs_unavailable",
                "message": "the job service is not configured",
            })),
        )
            .into_response();
    };

    let mut manifest = json!({
        "generator": req.generator,
        "seed": req.seed,
        "tf": req.tf,
        "length": req.length,
    });
    if let Some(p) = req.params {
        manifest["params"] = p;
    }
    if let Some(s) = req.start {
        manifest["start"] = json!(s);
    }

    // Validate here rather than inside the worker. A bad generator name should be a
    // 422 on the request that made it, not a failed job the caller has to poll for.
    let spec = match crate::synthetic_worker::parse_synthetic_manifest(&manifest) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": e.code,
                    "field": e.field,
                    "fix": e.fix,
                })),
            )
                .into_response()
        }
    };
    if let Err(e) = backtest::synthetic::validate(&spec) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "generator_refused", "message": e.to_string() })),
        )
            .into_response();
    }

    let mut submission = Submission::new(JobKind::SimulatePaths, token.user_id(), manifest);
    submission.project_id = req.project_id;
    submission.submitted_by = SubmittedBy::User;

    match store.submit(submission).await {
        Ok(result) => (
            if result.deduplicated {
                StatusCode::OK
            } else {
                StatusCode::ACCEPTED
            },
            Json(json!({
                "job_id": result.job_id,
                "deduplicated": result.deduplicated,
                "state": result.state.as_str(),
                "instrument_id": spec.instrument_id(),
                "venue_id": "synthetic",
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "submit_failed", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// One redacted catalogue row: id, generator, timeframe, length, start, public meta.
type CatalogueRow = (String, String, String, i64, DateTime<Utc>, Value);

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub generator: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/data/synthetic — the redacted catalogue.
///
/// `public_meta` only: instrument id, timeframe, bar count and span. Not the
/// generator parameters, and not the truth. The generator *name* is included
/// because it is already in the instrument id and pretending otherwise would be
/// theatre — but "this is a `planted_ar1`" without phi is not the answer, and a
/// suite that mixes planted and noise tasks does not tell the agent which is which
/// on any given task anyway.
pub async fn list_synthetic(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let rows: Result<Vec<CatalogueRow>, _> = sqlx::query_as(
        "SELECT instrument_id, generator, timeframe, length, start_time, public_meta \
           FROM synthetic_instruments \
          WHERE ($1::text IS NULL OR generator = $1) \
          ORDER BY created_at DESC LIMIT $2",
    )
    .bind(q.generator.as_deref())
    .bind(limit)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "instruments": rows.iter().map(|(id, gen, tf, len, start, meta)| json!({
                    "instrument_id": id,
                    "generator": gen,
                    "venue_id": "synthetic",
                    "timeframe": tf,
                    "length": len,
                    "start": start,
                    "public_meta": meta,
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

/// GET /api/data/synthetic/{instrument}/truth — the answer key.
///
/// Requires `evals.truth`. The refusal deliberately does not say whether the
/// instrument exists, because "404 vs 403" on a guessed id is itself a signal about
/// which instruments are planted.
pub async fn get_truth(
    State(state): State<AppState>,
    token: BearerToken,
    Path(instrument_id): Path<String>,
) -> impl IntoResponse {
    if !token.permits(EVALS_TRUTH) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "scope_required",
                "message": format!("reading a planted mechanism requires the {EVALS_TRUTH} scope"),
                "fix": "grade the run with the eval harness's own credentials; \
                        a research session is never granted this scope",
            })),
        )
            .into_response();
    }

    let row: Result<Option<(Value, Value)>, _> =
        sqlx::query_as("SELECT params, truth FROM synthetic_instruments WHERE instrument_id = $1")
            .bind(&instrument_id)
            .fetch_optional(&state.pg)
            .await;

    match row {
        Ok(Some((params, truth))) => (
            StatusCode::OK,
            Json(json!({
                "instrument_id": instrument_id,
                "params": params,
                "truth": truth,
            })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "instrument_id": instrument_id})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "query_failed", "message": e.to_string()})),
        )
            .into_response(),
    }
}
