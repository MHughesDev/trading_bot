//! AI Model Studio REST API handlers.
//!
//! All endpoints follow the house pattern from `routes/backtests.rs`:
//! `State(AppState)` + `BearerToken` + typed extractors -> `impl IntoResponse`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use domain::model_def::validate::validate as validate_def;
use model_registry::{CreateModelRequest, TrainRequest};

use crate::{auth::BearerToken, state::AppState};

fn not_found() -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
}

fn unprocessable(msg: impl std::fmt::Display) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": "invalid_request", "message": msg.to_string() })),
    )
        .into_response()
}

fn map_result<T: serde::Serialize>(result: anyhow::Result<T>) -> axum::response::Response {
    match result {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") {
                not_found()
            } else {
                unprocessable(msg)
            }
        }
    }
}

fn map_action(result: anyhow::Result<()>) -> axum::response::Response {
    match result {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => {
            let msg = e.to_string();
            let code = if msg.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (
                code,
                Json(json!({ "error": "action_failed", "message": msg })),
            )
                .into_response()
        }
    }
}

// -- List / Create / Detail / Rename / Delete --

#[derive(Debug, Deserialize)]
pub struct ModelListParams {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub asset_class: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// GET /api/models
pub async fn list_models(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<ModelListParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    match state
        .models
        .list_models(
            token.user_id(),
            params.kind.as_deref(),
            params.status.as_deref(),
            params.asset_class.as_deref(),
        )
        .await
    {
        Ok(models) => {
            let total = models.len();
            let page: Vec<_> = models.into_iter().take(limit).collect();
            Json(json!({ "models": page, "total": total })).into_response()
        }
        Err(e) => unprocessable(e),
    }
}

/// POST /api/models
pub async fn create_model(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateModelRequest>,
) -> impl IntoResponse {
    if let Err(errors) = validate_def(&req.definition) {
        let formatted: Vec<_> = errors
            .iter()
            .map(|e| json!({ "path": e.path, "message": e.message }))
            .collect();
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "invalid_definition", "errors": formatted })),
        )
            .into_response();
    }
    match state.models.create_model(token.user_id(), req).await {
        Ok(model_id) => {
            (StatusCode::CREATED, Json(json!({ "model_id": model_id }))).into_response()
        }
        Err(e) => unprocessable(e),
    }
}

/// GET /api/models/:id
pub async fn get_model(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.models.get_model(&id, token.user_id()).await {
        Ok(Some(m)) => Json(m).into_response(),
        Ok(None) => not_found(),
        Err(e) => unprocessable(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PatchModelRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
}

/// PATCH /api/models/:id
pub async fn patch_model(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<PatchModelRequest>,
) -> impl IntoResponse {
    let display_name = match req.display_name {
        Some(ref n) => n.as_str(),
        None => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "display_name required" })),
            )
                .into_response()
        }
    };
    map_action(
        state
            .models
            .rename_model(
                &id,
                token.user_id(),
                display_name,
                req.description.as_deref(),
            )
            .await,
    )
}

/// DELETE /api/models/:id
pub async fn delete_model(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_action(state.models.delete_model(&id, token.user_id()).await)
}

/// POST /api/models/:id/archive
pub async fn archive_model(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_action(state.models.archive_model(&id, token.user_id()).await)
}

// -- Training runs --

/// POST /api/models/:id/train
pub async fn start_train(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<TrainRequest>,
) -> impl IntoResponse {
    match state.models.start_train(&id, token.user_id(), req).await {
        Ok(run_id) => (StatusCode::CREATED, Json(json!({ "run_id": run_id }))).into_response(),
        Err(e) => unprocessable(e),
    }
}

/// GET /api/models/:id/runs
pub async fn list_runs(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .list_runs(&id, token.user_id())
            .await
            .map(|runs| json!({ "runs": runs })),
    )
}

/// GET /api/models/:id/runs/:run_id
pub async fn get_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path((_id, run_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    match state.models.get_run(run_id, token.user_id()).await {
        Some(snap) => Json(snap).into_response(),
        None => not_found(),
    }
}

/// POST /api/models/:id/runs/:run_id/cancel
pub async fn cancel_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path((_id, run_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    map_action(state.models.cancel_run(run_id, token.user_id()).await)
}

// -- Versions --

/// GET /api/models/:id/versions
pub async fn list_versions(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .list_versions(&id, token.user_id())
            .await
            .map(|v| json!({ "versions": v })),
    )
}

#[derive(Debug, Deserialize)]
pub struct RegisterVersionRequest {
    pub artifact_uri: String,
    pub artifact_hash: String,
    pub notes: Option<String>,
}

/// POST /api/models/:id/versions
pub async fn register_version(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<RegisterVersionRequest>,
) -> impl IntoResponse {
    match state
        .models
        .register_version(
            &id,
            token.user_id(),
            &req.artifact_uri,
            &req.artifact_hash,
            req.notes.as_deref(),
        )
        .await
    {
        Ok(v) => (StatusCode::CREATED, Json(json!({ "version": v }))).into_response(),
        Err(e) => unprocessable(e),
    }
}

// -- Evaluation --

/// POST /api/models/:id/versions/:v/evaluate
pub async fn start_eval(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
) -> impl IntoResponse {
    match state.models.start_eval(&id, version, token.user_id()).await {
        Ok(eval_id) => (StatusCode::CREATED, Json(json!({ "eval_id": eval_id }))).into_response(),
        Err(e) => unprocessable(e),
    }
}

/// GET /api/models/:id/evaluations
pub async fn list_evals(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .list_evals(&id, token.user_id())
            .await
            .map(|e| json!({ "evaluations": e })),
    )
}

/// GET /api/models/:id/evaluations/:eval_id
pub async fn get_eval(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, eval_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    map_result(state.models.get_eval(&id, eval_id, token.user_id()).await)
}

#[derive(Debug, Deserialize)]
pub struct CompareParams {
    pub versions: String,
}

/// GET /api/models/:id/evaluations/compare?versions=a,b
pub async fn compare_evals(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Query(params): Query<CompareParams>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .compare_evals(&id, &params.versions, token.user_id())
            .await,
    )
}

// -- Promote / rollback / aliases --

#[derive(Debug, Deserialize)]
pub struct PromoteRequest {
    pub environment: Option<String>,
    pub override_reason: Option<String>,
}

/// POST /api/models/:id/versions/:v/promote
pub async fn promote(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
    Json(req): Json<PromoteRequest>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .promote_gated(
                &id,
                token.user_id(),
                version,
                req.environment.as_deref().unwrap_or("paper"),
                req.override_reason.as_deref(),
            )
            .await,
    )
}

/// POST /api/models/:id/aliases/:alias/rollback
pub async fn rollback(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, alias)): Path<(String, String)>,
) -> impl IntoResponse {
    map_action(state.models.rollback(&id, &alias, token.user_id()).await)
}

/// GET /api/models/:id/aliases
pub async fn get_aliases(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(state.models.get_aliases(&id, token.user_id()).await)
}

// -- Deployments --

/// GET /api/models/:id/deployments
pub async fn list_deployments(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .list_deployments(&id, token.user_id())
            .await
            .map(|d| json!({ "deployments": d })),
    )
}

#[derive(Debug, Deserialize)]
pub struct CreateDeploymentRequest {
    pub version: i32,
    pub environment: String,
    #[serde(default)]
    pub traffic_pct: Option<i32>,
}

/// POST /api/models/:id/deployments
pub async fn create_deployment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<CreateDeploymentRequest>,
) -> impl IntoResponse {
    match state
        .models
        .create_deployment(
            &id,
            req.version,
            &req.environment,
            req.traffic_pct.unwrap_or(100),
            token.user_id(),
        )
        .await
    {
        Ok(did) => (StatusCode::CREATED, Json(json!({ "deployment_id": did }))).into_response(),
        Err(e) => unprocessable(e),
    }
}

// -- Test Lab --

/// GET /api/models/:id/test-cases
pub async fn list_test_cases(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .list_test_cases(&id, token.user_id())
            .await
            .map(|c| json!({ "test_cases": c })),
    )
}

#[derive(Debug, Deserialize)]
pub struct AddTestCaseRequest {
    pub name: String,
    pub input: serde_json::Value,
    pub expected: Option<serde_json::Value>,
}

/// POST /api/models/:id/test-cases
pub async fn add_test_case(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<AddTestCaseRequest>,
) -> impl IntoResponse {
    match state
        .models
        .add_test_case(&id, token.user_id(), &req.name, req.input, req.expected)
        .await
    {
        Ok(case_id) => (StatusCode::CREATED, Json(json!({ "case_id": case_id }))).into_response(),
        Err(e) => unprocessable(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct TestInferenceRequest {
    #[serde(default)]
    pub instances: Vec<serde_json::Value>,
}

/// POST /api/models/:id/versions/:v/test
pub async fn test_inference(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
    Json(req): Json<TestInferenceRequest>,
) -> impl IntoResponse {
    map_result(
        state
            .models
            .test_inference(&id, version, token.user_id(), req.instances)
            .await,
    )
}

// -- Lineage / traces / used-by --

/// GET /api/models/:id/lineage
pub async fn get_lineage(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    map_result(state.models.get_lineage(&id, token.user_id()).await)
}

/// GET /api/models/:id/traces  (stub -- real data in Phase 4 ClickHouse query)
pub async fn get_traces(
    State(_state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    Json(json!({ "model_id": id, "traces": [], "note": "inference traces available in Phase 4" }))
}

/// GET /api/models/:id/used-by
pub async fn get_used_by(
    State(_state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    Json(
        json!({ "model_id": id, "strategies": [], "note": "cross-reference available in Phase 4" }),
    )
}
