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
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

use domain::model_def::validate::validate as validate_def;
use model_registry::{CreateModelRequest, TrainRequest};

use crate::{auth::BearerToken, state::AppState};

// In-memory rate limiter for test_inference: 20 calls per 60s per (user_id, model_id).
static TEST_RATE_LIMITER: Mutex<Option<HashMap<(String, String), VecDeque<Instant>>>> =
    Mutex::new(None);

fn check_test_rate_limit(user_id: &str, model_id: &str) -> bool {
    const MAX_CALLS: usize = 20;
    const WINDOW: Duration = Duration::from_secs(60);

    let mut guard = TEST_RATE_LIMITER.lock().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    let key = (user_id.to_string(), model_id.to_string());
    let now = Instant::now();
    let deque = map.entry(key).or_default();
    // Drop timestamps older than the window.
    while deque
        .front()
        .map(|t| now.duration_since(*t) > WINDOW)
        .unwrap_or(false)
    {
        deque.pop_front();
    }
    if deque.len() >= MAX_CALLS {
        return false;
    }
    deque.push_back(now);
    true
}

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

/// DELETE /api/models/:id/test-cases/:case_id
pub async fn delete_test_case(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, case_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    map_action(
        state
            .models
            .delete_test_case(&id, case_id, token.user_id())
            .await,
    )
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
    let uid = token.user_id().to_string();
    if !check_test_rate_limit(&uid, &id) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": "rate_limit_exceeded",
                "message": "maximum 20 test inference calls per minute per model",
                "retry_after_secs": 60,
            })),
        )
            .into_response();
    }
    map_result(
        state
            .models
            .test_inference(&id, version, token.user_id(), req.instances)
            .await,
    )
}

#[derive(Debug, Deserialize)]
pub struct FeatureVectorQuery {
    #[serde(default)]
    pub instrument_id: Option<String>,
    #[serde(default)]
    pub timeframe: Option<String>,
}

/// Lookback window (days) used to load enough warm-up bars for the timeframe.
fn lookback_days_for(tf_key: &str) -> i64 {
    match tf_key {
        "1m" => 2,
        "5m" => 10,
        "15m" | "30m" => 30,
        "1h" => 120,
        "4h" => 365,
        "1d" => 1095,
        _ => 30,
    }
}

/// GET /api/models/:id/feature-vector?instrument_id=&timeframe=
///
/// Returns the model's expected feature schema, computed from recent ClickHouse
/// bars when an instrument + timeframe are given (so the Test Lab can prefill a
/// realistic input), or zeros otherwise.
pub async fn feature_vector(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Query(q): Query<FeatureVectorQuery>,
) -> impl IntoResponse {
    use backtest::{BarStore, TimeframeExt};
    use domain::payloads::bar::Timeframe;

    let model = match state.models.get_model(&id, token.user_id()).await {
        Ok(Some(m)) => m,
        Ok(None) => return not_found(),
        Err(e) => return unprocessable(e),
    };

    let fs_ref = model
        .definition
        .feature_set_ref
        .clone()
        .unwrap_or_else(|| "fs_core_ohlcv_v3".to_string());
    let names: Vec<String> = features::resolve_feature_set(&fs_ref)
        .map(|s| s.features.clone())
        .unwrap_or_default();

    let instrument = q
        .instrument_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let tf_key = q
        .timeframe
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    if let (Some(inst), Some(tf_key)) = (instrument.as_deref(), tf_key.as_deref()) {
        if let Some(tf) = <Timeframe as TimeframeExt>::from_key(tf_key) {
            let store = BarStore::connect(&state.clickhouse_url);
            let to = chrono::Utc::now();
            let from = to - chrono::Duration::days(lookback_days_for(tf_key));
            match store.load_bars(inst, tf, from, to).await {
                Ok(bars) if !bars.is_empty() => {
                    let feats = crate::features_compute::latest_vector(&bars, &names);
                    let as_of_ms = bars.last().map_or(0, |b| b.ts_ns / 1_000_000);
                    return Json(json!({
                        "feature_set": fs_ref,
                        "feature_order": names,
                        "features": feats,
                        "source": "computed",
                        "instrument_id": inst,
                        "timeframe": tf_key,
                        "as_of_ms": as_of_ms,
                        "bars_used": bars.len(),
                    }))
                    .into_response();
                }
                Ok(_) => {} // no stored bars — fall through to schema zeros
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({ "error": e.to_string() })),
                    )
                        .into_response();
                }
            }
        }
    }

    // Schema fallback: expected feature names with placeholder zeros.
    let feats: serde_json::Map<String, serde_json::Value> =
        names.iter().map(|n| (n.clone(), json!(0.0))).collect();
    Json(json!({
        "feature_set": fs_ref,
        "feature_order": names,
        "features": feats,
        "source": "schema",
    }))
    .into_response()
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

#[derive(Debug, Deserialize)]
pub struct TracesParams {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/models/:id/traces
///
/// Returns recent inference traces from model_events (kind = 'inference_trace').
/// Each trace record includes version, instrument, latency_ms, result, and recorded_at.
pub async fn get_traces(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Query(params): Query<TracesParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(100).clamp(1, 1000);
    match state
        .models
        .get_traces_for_model(&id, token.user_id(), limit)
        .await
    {
        Ok(traces) => Json(json!({ "model_id": id, "traces": traces })).into_response(),
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

/// GET /api/models/:id/used-by
///
/// Returns a list of strategies that reference this model in their node graph.
pub async fn get_used_by(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.models.get_used_by(&id).await {
        Ok(strategies) => Json(json!({ "model_id": id, "strategies": strategies })).into_response(),
        Err(e) => unprocessable(e),
    }
}

// -- Data quality + walk-forward windows (I-0.7 / I-0.11) --

#[derive(Debug, Deserialize)]
pub struct DataQualityParams {
    pub instrument: String,
    pub timeframe: String,
    pub start: String,
    pub end: String,
    #[serde(default = "default_sigma")]
    pub sigma: f64,
}

fn default_sigma() -> f64 {
    5.0
}

/// GET /api/models/data/quality?instrument=&timeframe=&start=&end=&sigma=
///
/// Bar-level data-quality diagnostics (gaps, duplicates, outliers) for the
/// series a training run would train on. Read-only — never mutates state.
pub async fn data_quality(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(params): Query<DataQualityParams>,
) -> impl IntoResponse {
    use backtest::{BarStore, TimeframeExt};
    use chrono::DateTime;
    use domain::payloads::bar::Timeframe;
    use model_registry::data_view::{data_quality as dq_compute, AsOf};

    let tf = match <Timeframe as TimeframeExt>::from_key(&params.timeframe) {
        Some(t) => t,
        None => return unprocessable(format!("unknown timeframe: {}", params.timeframe)),
    };
    let start = match DateTime::parse_from_rfc3339(&params.start) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        Err(e) => return unprocessable(format!("invalid start: {e}")),
    };
    let end = match DateTime::parse_from_rfc3339(&params.end) {
        Ok(t) => t.with_timezone(&chrono::Utc),
        Err(e) => return unprocessable(format!("invalid end: {e}")),
    };

    let store = BarStore::connect(&state.clickhouse_url);
    let as_of = AsOf::from_datetime(end);
    let bars = match store.load_bars(&params.instrument, tf, start, end).await {
        Ok(b) => model_registry::data_view::filter_as_of(b, as_of),
        Err(e) => return unprocessable(format!("bar load failed: {e}")),
    };

    let step_secs = tf.seconds();
    let report = dq_compute(&bars, step_secs, params.sigma);
    Json(serde_json::json!({
        "instrument": params.instrument,
        "timeframe": params.timeframe,
        "start": params.start,
        "end": params.end,
        "bar_count": report.bar_count,
        "coverage_pct": report.coverage_pct,
        "gaps": report.gaps,
        "dupes": report.dupes,
        "outliers": report.outliers,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct DataWindowsRequest {
    /// Walk-forward CV spec to preview.
    pub spec: domain::model_def::cv::WalkForwardSpec,
    /// Total rows in the (already-materialized) dataset.
    pub row_count: u64,
    /// Label horizon token (e.g. `"1h"`) used to size purge.
    pub horizon_token: String,
    /// Bar timeframe key (e.g. `"1m"`) matching the dataset.
    pub timeframe: String,
}

/// POST /api/models/data/windows
///
/// Preview the walk-forward fold boundaries (train/cal/test index ranges) that
/// Rust would compute for the given spec over a dataset with `row_count` rows.
/// Lets the UI show fold geometry before committing to a full training run.
pub async fn data_windows(
    State(_state): State<AppState>,
    _token: BearerToken,
    Json(req): Json<DataWindowsRequest>,
) -> impl IntoResponse {
    use features::walk_forward_folds;

    let horizon_bars =
        features::label_horizon_bars(&req.horizon_token, &req.timeframe).unwrap_or(60);

    match walk_forward_folds(req.row_count as usize, &req.spec, horizon_bars) {
        Ok(folds) => {
            let windows: Vec<serde_json::Value> = folds
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "fold": f.index,
                        "train": { "start": f.train.start, "end": f.train.end, "len": f.train.len() },
                        "cal":   { "start": f.cal.start,   "end": f.cal.end,   "len": f.cal.len()   },
                        "test":  { "start": f.test.start,  "end": f.test.end,  "len": f.test.len()  },
                    })
                })
                .collect();
            Json(serde_json::json!({
                "row_count": req.row_count,
                "horizon_bars": horizon_bars,
                "folds": windows,
            }))
            .into_response()
        }
        Err(e) => unprocessable(format!("fold generation failed: {e}")),
    }
}

#[derive(Debug, Deserialize)]
pub struct ForNodeParams {
    pub kind: Option<String>,
    pub asset_class: Option<String>,
}

/// GET /api/models/for-node
///
/// Returns lightweight model records suitable for populating the strategy-builder
/// AIForecastNode dropdown.  Only non-archived models are returned; each entry
/// includes whether a `production` alias has been promoted so the UI can disable
/// non-ready models.
pub async fn for_node(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(params): Query<ForNodeParams>,
) -> impl IntoResponse {
    // Query models by kind and optional asset_class, excluding archived.
    let rows: Result<Vec<(String, String, String, String, bool)>, sqlx::Error> = sqlx::query_as(
        "SELECT m.model_id, m.slug, m.display_name, m.status, \
         EXISTS(SELECT 1 FROM model_aliases a WHERE a.model_id = m.model_id AND a.alias = 'production') AS has_production \
         FROM ai_models m \
         WHERE ($1::text IS NULL OR m.model_kind = $1) \
           AND ($2::text IS NULL OR m.asset_class = $2) \
           AND m.status != 'archived' \
         ORDER BY m.display_name",
    )
    .bind(params.kind.as_deref())
    .bind(params.asset_class.as_deref())
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rs) => {
            let models: Vec<serde_json::Value> = rs
                .into_iter()
                .map(|(id, slug, display_name, status, has_production)| {
                    json!({
                        "id": id,
                        "slug": slug,
                        "display_name": display_name,
                        "status": status,
                        "has_production": has_production,
                    })
                })
                .collect();
            Json(json!({ "models": models })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// -- Feature library (I-3.1, I-3.5) --

/// GET /api/models/feature-sets  — list all registered feature sets.
pub async fn list_feature_sets(
    State(state): State<AppState>,
    _token: BearerToken,
) -> impl IntoResponse {
    let sets = state.models.list_feature_sets();
    Json(json!({ "feature_sets": sets, "registry_version": features::REGISTRY_VERSION }))
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct FeaturePreviewRequest {
    pub feature_set_ref: String,
    pub features: Option<Vec<String>>,
}

/// POST /api/models/features/preview  — compute sample feature values + stats (I-3.5).
pub async fn feature_preview(
    State(_state): State<AppState>,
    _token: BearerToken,
    Json(req): Json<FeaturePreviewRequest>,
) -> impl IntoResponse {
    use features::{build_training_frame, resolve_feature_set, OhlcvRow};

    // Resolve feature set.
    let feature_names: Vec<String> = if let Some(names) = req.features {
        names
    } else if let Some(spec) = resolve_feature_set(&req.feature_set_ref) {
        spec.features.clone()
    } else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": format!("unknown feature_set_ref: {}", req.feature_set_ref) })),
        )
            .into_response();
    };

    // Validate feature names.
    let unknown = features::validate_features(&feature_names);
    if !unknown.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "unknown features", "unknown": unknown })),
        )
            .into_response();
    }

    // Generate a synthetic fixture for preview (100 bars of BTC-style prices).
    let n = 120usize;
    let bars: Vec<OhlcvRow> = (0..n)
        .map(|i| {
            let base = 50_000.0 + (i as f64) * 10.0;
            let ts_ns = (1_700_000_000i64 + i as i64 * 60) * 1_000_000_000;
            OhlcvRow {
                ts_ns,
                open: base,
                high: base * 1.002,
                low: base * 0.998,
                close: base + (i as f64 % 5.0 - 2.0) * 20.0,
                volume: 1_000_000.0 + (i as f64) * 100.0,
            }
        })
        .collect();

    let frame = build_training_frame(&bars, &feature_names, 1);

    // Per-feature stats.
    #[allow(clippy::cast_precision_loss)]
    let stats: Vec<serde_json::Value> = frame
        .feature_names
        .iter()
        .zip(frame.columns.iter())
        .map(|(name, col)| {
            let n = col.len() as f64;
            if n == 0.0 {
                return json!({ "name": name, "n": 0, "mean": null, "std": null, "nan_rate": 1.0 });
            }
            let mean = col.iter().sum::<f64>() / n;
            let std = if n > 1.0 {
                (col.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
            } else {
                0.0
            };
            json!({ "name": name, "n": col.len(), "mean": mean, "std": std, "nan_rate": 0.0 })
        })
        .collect();

    // Sample rows (up to 10).
    let sample_rows: Vec<serde_json::Value> = (0..frame.row_count().min(10))
        .map(|i| {
            let row: serde_json::Map<String, serde_json::Value> = frame
                .feature_names
                .iter()
                .zip(frame.columns.iter())
                .map(|(name, col)| (name.clone(), json!(col[i])))
                .collect();
            serde_json::Value::Object(row)
        })
        .collect();

    Json(json!({
        "feature_set_ref": req.feature_set_ref,
        "features": feature_names,
        "n_rows": frame.row_count(),
        "stats": stats,
        "sample": sample_rows,
        "note": "stats computed on a 120-bar synthetic fixture for preview only",
    }))
    .into_response()
}

// -- Reproducibility (I-3.9) --

#[derive(Debug, Deserialize)]
pub struct ReproduceRequest {
    pub run_id_or_hash: String,
}

/// POST /api/models/:id/runs/reproduce  — re-execute a run from its spec hash (I-3.9).
pub async fn reproduce_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Json(req): Json<ReproduceRequest>,
) -> impl IntoResponse {
    match state
        .models
        .reproduce_run(&req.run_id_or_hash, &id, token.user_id())
        .await
    {
        Ok(run_id) => (
            StatusCode::ACCEPTED,
            Json(json!({ "run_id": run_id, "reproduced_from": req.run_id_or_hash })),
        )
            .into_response(),
        Err(e) => map_result::<serde_json::Value>(Err(e)),
    }
}

// -- Run compare (I-3.10) --

#[derive(Debug, Deserialize)]
pub struct RunCompareParams {
    pub ids: String, // comma-separated run UUIDs
}

/// GET /api/models/:id/runs/compare?ids=run1,run2  — side-by-side run diff (I-3.10).
pub async fn compare_runs(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Query(params): Query<RunCompareParams>,
) -> impl IntoResponse {
    let run_ids: Vec<uuid::Uuid> = params
        .ids
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if run_ids.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "ids must be comma-separated UUIDs" })),
        )
            .into_response();
    }
    match state
        .models
        .compare_runs(&id, &run_ids, token.user_id())
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => map_result::<serde_json::Value>(Err(e)),
    }
}

// -- Leaderboard (I-2.11) --

#[derive(Debug, Deserialize)]
pub struct LeaderboardParams {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub asset_class: Option<String>,
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/models/leaderboard
pub async fn leaderboard(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<LeaderboardParams>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    match state
        .models
        .leaderboard(
            token.user_id(),
            params.kind.as_deref(),
            params.asset_class.as_deref(),
            params.metric.as_deref(),
            limit,
        )
        .await
    {
        Ok(entries) => {
            let total = entries.len();
            Json(json!({ "leaderboard": entries, "total": total })).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// -- Evaluation report (I-2.12) --

/// GET /api/models/:id/versions/:v/report
pub async fn get_report(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
) -> impl IntoResponse {
    match state.models.get_report(&id, version, token.user_id()).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("no completed eval") {
                not_found()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": msg })),
                )
                    .into_response()
            }
        }
    }
}

/// GET /api/models/:id/versions/:v/report/export  (JSON download)
pub async fn export_report(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
) -> impl IntoResponse {
    match state.models.export_report(&id, version, token.user_id()).await {
        Ok(bytes) => (
            StatusCode::OK,
            [
                ("Content-Type", "application/json"),
                (
                    "Content-Disposition",
                    "attachment; filename=\"eval_report.json\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("no completed eval") {
                not_found()
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": msg })),
                )
                    .into_response()
            }
        }
    }
}
