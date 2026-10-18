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
    Json(mut body): Json<serde_json::Value>,
) -> impl IntoResponse {
    // Data admission (Set P P-2.3). The optional `qc_waiver_id` names an approved
    // `approval_requests.kind = 'qc_waiver'` row; it is taken off the body before
    // the spec is parsed because it is evidence about this Experiment, not a field
    // of it.
    let waiver_id = body
        .as_object_mut()
        .and_then(|o| o.remove("qc_waiver_id"))
        .and_then(|v| v.as_str().and_then(|s| s.parse::<Uuid>().ok()));
    // Evidence offered for the Experiment: a finding is a memory of a measurement, never one
    // (KM-03). Taken off the body like `qc_waiver_id`, so it is checked and not parsed as a
    // spec field.
    let offered: Vec<String> = body
        .as_object_mut()
        .and_then(|o| o.remove("evidence_ids"))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    if let Some(refusal) = crate::knowledge_routes::finding_evidence_refusal(&offered) {
        return refusal;
    }
    let spec: CreateExperimentSpec = match serde_json::from_value(body) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "invalid_request", "message": e.to_string() })),
            )
                .into_response()
        }
    };
    let (verdicts, skipped) = crate::data_admission::check_experiment_data(
        &state.pg,
        &spec.universe_ref,
        spec.eval_resolution.as_deref(),
        waiver_id,
    )
    .await;
    if verdicts.iter().any(|v| !v.admitted()) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(crate::data_admission::refusal_body(&verdicts)),
        )
            .into_response();
    }
    let experiment_id = spec.experiment_id.clone();
    match state.suite.create_experiment(token.user_id(), spec) {
        Ok(view) => {
            let lineage = crate::data_admission::record_admission(
                &state.pg,
                &experiment_id,
                token.user_id(),
                &verdicts,
            )
            .await;
            let mut out = serde_json::to_value(&view).unwrap_or_else(|_| json!({}));
            out["data_admission"] = json!({
                "series": verdicts,
                "ungraded": verdicts
                    .iter()
                    .filter(|v| v.grade.is_none())
                    .map(|v| &v.instrument)
                    .collect::<Vec<_>>(),
                "not_graded_here": skipped,
                "lineage_recorded": lineage.is_ok(),
            });
            (StatusCode::CREATED, Json(out)).into_response()
        }
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

/// A Study started with `detach: true`: the HTTP call returned at once and the
/// Study runs on a blocking task (AGENT-010 §8; S-2.11). Held in memory, which
/// is honest about what it is: a restart loses a running Study, and the status
/// read says so (`lost`) rather than leaving a run waiting on a job that is gone.
#[derive(Clone, Debug)]
pub enum DetachedStudy {
    Running,
    Failed(String),
}

type DetachedKey = (Uuid, String);

fn detached_studies(
) -> &'static std::sync::Mutex<std::collections::HashMap<DetachedKey, DetachedStudy>> {
    static M: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<DetachedKey, DetachedStudy>>,
    > = std::sync::OnceLock::new();
    M.get_or_init(Default::default)
}

/// Where a detached Study stands, for the list read.
fn detached_status(id: Uuid) -> serde_json::Value {
    let m = detached_studies().lock().expect("detached studies lock");
    let mut out = serde_json::Map::new();
    for ((exp, study), st) in m.iter().filter(|((e, _), _)| *e == id) {
        let _ = exp;
        out.insert(
            study.clone(),
            match st {
                DetachedStudy::Running => json!({ "status": "running" }),
                DetachedStudy::Failed(why) => json!({ "status": "failed", "error": why }),
            },
        );
    }
    serde_json::Value::Object(out)
}

/// GET /api/backtest/experiments/:id/studies — sealed Study distributions.
///
/// `detached` is additive: the state of any Study started with `detach: true`
/// that has not sealed (a sealed one is simply in `studies`).
pub async fn list_studies(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.suite.list_studies(token.user_id(), id) {
        Some(studies) => {
            Json(json!({ "studies": studies, "detached": detached_status(id) })).into_response()
        }
        None => map_err(SuiteError::NotFound),
    }
}

/// The body of `POST .../studies`: the Study itself plus whether to wait for it.
#[derive(Debug, Deserialize)]
pub struct StudyBody {
    #[serde(flatten)]
    pub spec: RunStudySpec,
    /// Return 202 at once and run the Study in the background. A caller that
    /// would otherwise hold a connection for minutes (the agent, which suspends
    /// on it) sets this.
    #[serde(default)]
    pub detach: bool,
}

/// POST /api/backtest/experiments/:id/studies — run a research Study.
pub async fn run_study(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<StudyBody>,
) -> impl IntoResponse {
    let StudyBody { spec, detach } = body;
    // Real Runs take minutes: keep the async worker free.
    let suite = state.suite.clone();
    let user = token.user_id();
    if detach {
        let study_id = spec.study_id.clone();
        let key = (id, study_id.clone());
        {
            let mut m = detached_studies().lock().expect("detached studies lock");
            if matches!(m.get(&key), Some(DetachedStudy::Running)) {
                return (
                    StatusCode::ACCEPTED,
                    Json(
                        json!({ "detached": true, "experiment_id": id, "study_id": study_id,
                                 "status": "running", "already_running": true }),
                    ),
                )
                    .into_response();
            }
            m.insert(key.clone(), DetachedStudy::Running);
        }
        tokio::spawn(async move {
            let done = tokio::task::spawn_blocking(move || suite.run_study(user, id, spec)).await;
            let mut m = detached_studies().lock().expect("detached studies lock");
            match done {
                // Sealed: the Study is in the suite's list, which is the answer.
                Ok(Ok(_)) => {
                    m.remove(&key);
                }
                Ok(Err(e)) => {
                    m.insert(key, DetachedStudy::Failed(format!("{e:?}")));
                }
                Err(e) => {
                    m.insert(
                        key,
                        DetachedStudy::Failed(format!("the study panicked: {e}")),
                    );
                }
            }
        });
        return (
            StatusCode::ACCEPTED,
            Json(
                json!({ "detached": true, "experiment_id": id, "study_id": study_id,
                         "status": "running" }),
            ),
        )
            .into_response();
    }
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
