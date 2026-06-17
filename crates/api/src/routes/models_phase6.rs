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
use serde_json::json;

use domain::model::forecast::CalibratedForecast;
use model_registry::tags::CreateTemplateRequest;

use crate::{auth::BearerToken, state::AppState};

// ── I-6.1: Publish contract predict ──────────────────────────────────────────

#[derive(Deserialize)]
pub struct PredictQuery {
    pub alias: Option<String>,
    pub version: Option<u32>,
    pub timeframe: Option<String>,
    pub as_of: Option<String>,
}

/// GET /api/models/{id}/predict
pub async fn predict(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
    Query(params): Query<PredictQuery>,
) -> impl IntoResponse {
    // Convert all query params to owned values before any .await to avoid
    // self-referential borrows in the async future (makes it !Send).
    let alias_str: String = params.alias.unwrap_or_else(|| "production".to_string());
    let timeframe: String = params.timeframe.unwrap_or_else(|| "1h".to_string());
    let as_of = params
        .as_of
        .and_then(|s| s.parse::<chrono::DateTime<Utc>>().ok())
        .unwrap_or_else(Utc::now)
        .min(Utc::now());

    let version: i32 = if let Some(v) = params.version {
        v as i32
    } else {
        let resolved: Option<(i32,)> = sqlx::query_as(
            "SELECT version FROM ai_model_aliases WHERE model_id = $1 AND alias = $2",
        )
        .bind(&id)
        .bind(&alias_str)
        .fetch_optional(&state.pg)
        .await
        .ok()
        .flatten();

        match resolved {
            Some((v,)) => v,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "error": format!("alias '{alias_str}' not found for {id}") })),
                )
                    .into_response();
            }
        }
    };

    let features_map = std::collections::HashMap::<String, f64>::new();
    match state.inference.forecast(&id, &alias_str, &id, &features_map).await {
        Some(result) => {
            if let Some(dist) = result.distribution {
                match CalibratedForecast::from_distribution(
                    id.clone(),
                    version as u32,
                    id.clone(),
                    timeframe,
                    as_of,
                    dist,
                ) {
                    Ok(cf) => Json(serde_json::to_value(cf).unwrap()).into_response(),
                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
                }
            } else {
                Json(json!({
                    "model_id": id, "version": version, "as_of": as_of,
                    "direction": result.direction, "confidence": result.confidence,
                    "point_in_time": true,
                })).into_response()
            }
        }
        None => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": "no forecast available" }))).into_response(),
    }
}

// ── I-6.4: Tags ───────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TagBody { pub tag: String }

#[derive(Deserialize)]
pub struct TagSearchQuery { pub tag: String, pub kind: Option<String> }

pub async fn add_tag(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind)): Path<(String, String)>,
    Json(body): Json<TagBody>,
) -> impl IntoResponse {
    match state.tags.add_tag(&id, &kind, &body.tag).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn remove_tag(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind, tag)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state.tags.remove_tag(&id, &kind, &tag).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn list_tags(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.tags.list_tags(&id, &kind).await {
        Ok(tags) => Json(serde_json::to_value(tags).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
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
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
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
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn get_annotations(
    State(state): State<AppState>,
    _token: BearerToken,
    Path((id, kind)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.tags.get_annotations(&id, &kind).await {
        Ok(annots) => Json(serde_json::to_value(annots).unwrap()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn create_template(
    State(state): State<AppState>,
    token: BearerToken,
    Json(req): Json<CreateTemplateRequest>,
) -> impl IntoResponse {
    let uid = token.user_id().to_string();
    match state.tags.create_template(req, &uid).await {
        Ok(tmpl) => (StatusCode::CREATED, Json(serde_json::to_value(tmpl).unwrap())).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
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
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

pub async fn fork_template(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.tags.fork_template(&id).await {
        Ok(def) => Json(json!({ "definition": def })).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}
