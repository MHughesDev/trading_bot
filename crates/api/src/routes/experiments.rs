//! Backtest-suite HTTP API (Set J, J-5.4).
//!
//! REST surface over the honest-evaluation core: experiments, their research
//! studies, the null picker, the staged-gate funnel, the one-shot vault, and the
//! reconciliation/suite-calibration read-outs. All rows are user-scoped by the
//! bearer-token-derived id (MASTER §8); the heavy lifting lives in
//! `backtest::suite::SuiteManager`, and these handlers only translate manager
//! results into HTTP responses. Progress streams over the `/ws/backtest-suite`
//! lane.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use backtest::gates::tickets::{check as check_tickets, Checkpoint};
use backtest::gates::Gate;
use backtest::nulls::NullKind;
use backtest::suite::{CreateExperimentSpec, RunStudySpec, SuiteError};

use crate::routes::ledger;
use crate::{auth::BearerToken, state::AppState};

/// Translate a [`SuiteError`] into an HTTP response with a stable error code.
fn map_err(e: SuiteError) -> axum::response::Response {
    use backtest::experiment::ExperimentError;
    let (code, kind) = match &e {
        SuiteError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        SuiteError::AlreadyExists => (StatusCode::CONFLICT, "already_exists"),
        SuiteError::Experiment(ExperimentError::VaultSpent) => {
            (StatusCode::CONFLICT, "vault_spent")
        }
        SuiteError::Experiment(_) => (StatusCode::CONFLICT, "operation_not_allowed"),
        SuiteError::Gate(_) => (StatusCode::UNPROCESSABLE_ENTITY, "gate_error"),
        SuiteError::Study(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_study"),
        SuiteError::Null(_) => (StatusCode::UNPROCESSABLE_ENTITY, "null_error"),
        SuiteError::Invalid(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
    };
    (
        code,
        Json(json!({ "error": kind, "message": e.to_string() })),
    )
        .into_response()
}

// ── experiments ───────────────────────────────────────────────────────────────

/// POST /api/backtest/experiments — create a candidate Experiment.
pub async fn create_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Json(spec): Json<CreateExperimentSpec>,
) -> impl IntoResponse {
    match state.suite.create_experiment(token.user_id(), spec) {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(e) => map_err(e),
    }
}

/// GET /api/backtest/experiments — list this user's Experiments (newest first).
pub async fn list_experiments(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    let views = state.suite.list_experiments(token.user_id());
    Json(json!({ "experiments": views }))
}

/// GET /api/backtest/experiments/:id — one Experiment header.
pub async fn get_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.get_experiment(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/promote — validated → live.
pub async fn promote_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.promote_to_live(token.user_id(), id) {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

/// POST /api/backtest/experiments/:id/retire — terminal.
pub async fn retire_experiment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.retire(token.user_id(), id) {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

// ── studies ────────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/studies — sealed Study distributions.
pub async fn list_studies(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.list_studies(token.user_id(), id) {
        Some(studies) => Json(json!({ "studies": studies })).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/studies — run a research Study.
pub async fn run_study(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(spec): Json<RunStudySpec>,
) -> impl IntoResponse {
    // Real Runs take minutes: keep the async worker free.
    let suite = state.suite.clone();
    let user = token.user_id();
    match tokio::task::spawn_blocking(move || suite.run_study(user, id, spec)).await {
        Ok(Ok(view)) => (StatusCode::CREATED, Json(view)).into_response(),
        Ok(Err(e)) => map_err(e),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "study_panicked", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── nulls ──────────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/nulls — recommendation + catalog + choice.
pub async fn null_picker(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.null_picker(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

#[derive(Debug, Deserialize)]
pub struct ChooseNullBody {
    pub kind: NullKind,
    #[serde(default)]
    pub override_reason: Option<String>,
}

/// POST /api/backtest/experiments/:id/nulls — choose the significance null.
pub async fn choose_null(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<ChooseNullBody>,
) -> impl IntoResponse {
    match state
        .suite
        .choose_null(token.user_id(), id, body.kind, body.override_reason)
    {
        Ok(choice) => Json(choice).into_response(),
        Err(e) => map_err(e),
    }
}

// ── gate funnel ──────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/funnel — the gate-funnel board.
pub async fn get_funnel(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.funnel(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// Store one negative result per gate this funnel refused.
///
/// Idempotent by construction: the entry id is derived from the ticket and the
/// gate, so re-advancing a funnel — which returns the board as it stands without
/// re-running anything — rewrites the same rows rather than accumulating a
/// second account of the same refusal.
///
/// Every failure here is a warning. The funnel's verdict is already recorded and
/// already returned; what is lost is the durable note for the *next* run, and
/// trading a verdict for a note would be the wrong way round.
async fn record_refusals(state: &AppState, user_id: uuid::Uuid, id: Uuid) {
    let Some((verdicts, start, end, family)) = state.suite.funnel_record(user_id, id) else {
        return;
    };
    let window = harness::memory::DataWindow::new(
        start.date_naive().to_string(),
        end.date_naive().to_string(),
    );
    let store = crate::memory_store::MemoryStore::new(state.pg.clone());

    for v in verdicts.iter().filter(|v| !v.passed) {
        let Some(entry) = crate::negative_from_funnel::entry_for(
            v,
            &id.to_string(),
            &family,
            window.clone(),
            &id.to_string(),
        ) else {
            continue;
        };
        if let Err(e) = store.put(user_id, &entry).await {
            tracing::warn!(
                experiment = %id,
                gate = ?v.gate,
                error = %e,
                "the refusal did not persist as a negative result"
            );
        }
    }
}

/// POST /api/backtest/experiments/:id/funnel/advance — drive Gates 0→3.
pub async fn advance_funnel(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    // S-8.4: the funnel drives gates 0 through 3, so the tickets gate 2 and
    // gate 3 demand are demanded here — before a single Study runs. Checked
    // first because the alternative is spending minutes of CPCV to arrive at a
    // refusal that was knowable from a table lookup.
    //
    // The tickets come from `mlops.workflow_verdict`, which only the workflow
    // engine writes. Nothing in this request can put a row there, which is the
    // property that makes "a persuasive argument with no ticket is refused"
    // true rather than aspirational.
    let tickets =
        match ledger::tickets_for(&state.pg, &token.user_id().to_string(), &id.to_string()).await {
            Ok(t) => t,
            Err(e) => {
                // A lookup that failed is not a lookup that found tickets. The
                // refusal direction is the only safe one: a gate advanced because
                // the ticket table was unreachable is a gate advanced on no
                // evidence.
                return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "tickets_unreadable",
                    "message": e.to_string(),
                    "fix": "the funnel refuses rather than advancing on evidence it could not read"
                })),
            )
                .into_response();
            }
        };
    for checkpoint in [
        Checkpoint::Funnel(Gate::Robustness),
        Checkpoint::Funnel(Gate::Significance),
    ] {
        if let Err(missing) = check_tickets(checkpoint, &tickets) {
            return (
                StatusCode::PRECONDITION_FAILED,
                Json(json!({
                    "error": "missing_workflow_ticket",
                    "checkpoint": checkpoint,
                    "message": backtest::gates::tickets::refusal(checkpoint, &missing),
                    "missing": missing,
                })),
            )
                .into_response();
        }
    }

    // Gates 2–3 run CPCV / synthetic-path / permutation Studies — real Runs,
    // minutes of work — so keep the async worker free.
    let suite = state.suite.clone();
    let user = token.user_id();
    match tokio::task::spawn_blocking(move || suite.advance_funnel(user, id)).await {
        Ok(Ok(view)) => {
            // A refused gate is a negative result the **platform** writes
            // (AGENT-007 §3.2, S-4.4). Written here rather than inside the
            // funnel, which would give the gate runner a dependency on a memory
            // store it does not need to run gates.
            //
            // After the response is built and before it is returned, because a
            // funnel that refused a gate has refused it whether or not the note
            // stored — and failing the request would turn a bookkeeping problem
            // into a lost verdict.
            record_refusals(&state, token.user_id(), id).await;
            Json(view).into_response()
        }
        Ok(Err(e)) => map_err(e),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "funnel_panicked", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── vault ──────────────────────────────────────────────────────────────────

/// GET /api/backtest/experiments/:id/vault — one-shot state + access log.
pub async fn get_vault(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.vault(token.user_id(), id) {
        Some(view) => Json(view).into_response(),
        None => map_err(SuiteError::NotFound),
    }
}

/// POST /api/backtest/experiments/:id/vault — spend the holdout vault (once).
pub async fn run_vault(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    // The vault access log records who touched it; the bearer-derived id is the
    // stable per-user actor (M-17 placeholder auth).
    let by = token.user_id().to_string();
    let suite = state.suite.clone();
    let user = token.user_id();
    match tokio::task::spawn_blocking(move || suite.run_vault(user, id, by)).await {
        Ok(Ok(view)) => Json(view).into_response(),
        Ok(Err(e)) => map_err(e),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "vault_panicked", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── reconciliation ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ReconcileBody {
    /// Realized per-period performance series (live ledger input).
    pub realized: Vec<f64>,
    /// Share of periods below the planned worst-5% that flags decay (default 0.10).
    #[serde(default = "default_drift")]
    pub drift_threshold: f64,
}

fn default_drift() -> f64 {
    0.10
}

/// POST /api/backtest/experiments/:id/reconcile — live vs backtest distribution.
pub async fn reconcile(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<ReconcileBody>,
) -> impl IntoResponse {
    match state
        .suite
        .reconcile(token.user_id(), id, &body.realized, body.drift_threshold)
    {
        Ok(view) => Json(view).into_response(),
        Err(e) => map_err(e),
    }
}

/// GET /api/backtest/calibration — the suite-calibration meta-view.
pub async fn suite_calibration(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    Json(state.suite.suite_calibration(token.user_id()))
}
