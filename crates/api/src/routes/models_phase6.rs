//! Phase 6 additions — publish contract, tags, annotations, templates (I-6.1, I-6.4).
//!
//! Appended to models.rs functionality; registered in mod.rs.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use domain::payloads::bar::Timeframe;
use model_registry::tags::CreateTemplateRequest;

use crate::model_call::ModelCall;

use crate::{auth::BearerToken, state::AppState};

// ── I-6.1: Publish contract predict ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct PredictQuery {
    /// The instrument to predict for. **Required**, and deliberately not
    /// defaulted: this route used to pass the *model id* into the gateway's
    /// `instrument_id` slot, which corrupted the prediction cache key and the
    /// trace row alike. There is no sensible default for "which market", and a
    /// guess here is a wrong answer nobody can see.
    pub instrument_id: String,
    pub alias: Option<String>,
    pub timeframe: Option<String>,
    pub as_of: Option<String>,
}

/// GET /api/models/{id}/predict
///
/// Calls a trained model with the features its version was trained on.
///
/// The whole body of this handler is argument parsing: the call itself lives in
/// [`crate::model_call`], because the agent-facing `call_model` tool reaches this
/// same route over HTTP (ADR-0022) and two implementations of "ask a model" would
/// be two answers to the only question that matters here.
///
/// **What this replaced.** The previous version built an empty
/// `HashMap<String, f64>` and passed it as the feature vector, with the model id
/// in the instrument slot. It answered 200 with a direction and a confidence
/// computed from no inputs at all. Nothing called it, which is the only reason
/// that never reached an operator.
pub async fn predict(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
    Query(params): Query<PredictQuery>,
) -> impl IntoResponse {
    let timeframe = match params.timeframe.as_deref().unwrap_or("1h") {
        "1m" => Timeframe::Minutes1,
        "5m" => Timeframe::Minutes5,
        "15m" => Timeframe::Minutes15,
        "1h" => Timeframe::Hours1,
        "4h" => Timeframe::Hours4,
        "1d" => Timeframe::Daily,
        other => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "unsupported_timeframe",
                    "timeframe": other,
                    "supported": ["1m", "5m", "15m", "1h", "4h", "1d"],
                })),
            )
                .into_response();
        }
    };

    // An unparseable `as_of` is refused rather than silently becoming "now": a
    // caller who asked for a point in time and got the present would read the
    // answer as point-in-time when it is not.
    let as_of = match params.as_of.as_deref() {
        None => Utc::now(),
        Some(s) => match s.parse::<chrono::DateTime<Utc>>() {
            Ok(t) => t,
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "unparseable_as_of",
                        "as_of": s,
                        "expected": "an RFC-3339 timestamp",
                    })),
                )
                    .into_response();
            }
        },
    };

    let req = crate::model_call::CallRequest {
        model_ref: id,
        alias: params.alias.unwrap_or_else(|| "production".to_string()),
        instrument_id: params.instrument_id,
        timeframe,
        as_of,
    };

    match crate::model_call::call_model(&state.pg, &state.inference, &state.clickhouse_url, &req)
        .await
    {
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "database_unreachable", "detail": e.to_string() })),
        )
            .into_response(),
        Ok(outcome) => {
            // The status says what kind of answer this is; the body always
            // carries the typed outcome, so a caller reads one shape whichever
            // status arrived.
            let code = match &outcome {
                ModelCall::Predicted { .. } => StatusCode::OK,
                ModelCall::NoSuchModel { .. } | ModelCall::NoSuchAlias { .. } => {
                    StatusCode::NOT_FOUND
                }
                // 503: the model declined to answer. Retryable, and not the
                // caller's mistake.
                ModelCall::Abstained { .. } => StatusCode::SERVICE_UNAVAILABLE,
                // 422: understood, and cannot be served as asked. A missing
                // feature set or an uncomputable column is not a bad request —
                // the request was fine, the inputs are not there.
                ModelCall::NoFeatureSet { .. }
                | ModelCall::FeaturesUnavailable { .. }
                | ModelCall::NoBars { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            };
            (code, Json(serde_json::to_value(&outcome).unwrap_or(Value::Null))).into_response()
        }
    }
}


// ── I-6.4: Tags ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TagBody {
    pub tag: String,
}

#[derive(Deserialize)]
pub struct TagSearchQuery {
    pub tag: String,
    pub kind: Option<String>,
}

pub async fn add_tag(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind)): Path<(String, String)>,
    Json(body): Json<TagBody>,
) -> impl IntoResponse {
    match state.tags.add_tag(&id, &kind, &body.tag).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn remove_tag(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind, tag)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state.tags.remove_tag(&id, &kind, &tag).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn list_tags(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.tags.list_tags(&id, &kind).await {
        Ok(tags) => Json(serde_json::to_value(tags).unwrap()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn search_by_tag(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(params): Query<TagSearchQuery>,
) -> impl IntoResponse {
    let kind = params.kind.as_deref().unwrap_or("model");
    match state.tags.search_by_tag(kind, &params.tag).await {
        Ok(ids) => Json(json!({ "kind": kind, "tag": params.tag, "ids": ids })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn set_annotation(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind, key)): Path<(String, String, String)>,
    Json(value): Json<serde_json::Value>,
) -> impl IntoResponse {
    match state.tags.set_annotation(&id, &kind, &key, value).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn get_annotations(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.tags.get_annotations(&id, &kind).await {
        Ok(annots) => Json(serde_json::to_value(annots).unwrap()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn create_template(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateTemplateRequest>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.tags.create_template(req, &uid).await {
        Ok(tmpl) => (
            StatusCode::CREATED,
            Json(serde_json::to_value(tmpl).unwrap()),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn list_templates(
    State(state): State<AppState>,
    _token: BearerToken,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let kind = params.get("kind").map(|s| s.as_str());
    match state.tags.list_templates(kind).await {
        Ok(tmpls) => Json(serde_json::to_value(tmpls).unwrap()).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn fork_template(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.tags.fork_template(&id).await {
        Ok(def) => Json(json!({ "definition": def })).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
