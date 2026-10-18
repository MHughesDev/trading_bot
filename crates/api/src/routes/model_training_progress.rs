//! What a model's Studio reads that no other route serves (Set H H-5.4, H-5.5).
//!
//! * `GET /api/models/{id}/runs/{run_id}/curve` — the recorded per-epoch loss
//!   curve of a training run. The run snapshot holds only the latest epoch (each
//!   progress frame is merged over the last); the history is `mlops.training_metric`
//!   (migration 0201), appended by `ModelManager::apply_nats_progress`.
//! * `GET /api/models/{id}/events` — the model's activity: created, promoted,
//!   rolled back, archived, renamed. These are `model_events` rows, which the
//!   cockpit's activity feed could not read, so "rename shows in activity" did
//!   not hold. Inference traces are excluded: they have their own route.
//!
//! Both are the caller's own: a model that is not theirs is a 404, the same as a
//! model that does not exist.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "not_found", "message": "no such model or run" })),
    )
        .into_response()
}

fn failed(e: impl std::fmt::Display) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": "invalid_request", "message": e.to_string() })),
    )
        .into_response()
}

/// `GET /api/models/{id}/runs/{run_id}/curve`
pub async fn curve(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, run_id)): Path<(String, Uuid)>,
) -> Response {
    match state
        .models
        .training_curve(&id, run_id, token.user_id())
        .await
    {
        Ok(points) => Json(json!({
            "run_id": run_id,
            "points": points.iter().map(|p| json!({
                "epoch": p.epoch,
                // null is "not measured", never 0.
                "train_loss": p.train_loss,
                "val_loss": p.val_loss,
            })).collect::<Vec<Value>>(),
        }))
        .into_response(),
        Err(e) if e.to_string().contains("not found") => not_found(),
        Err(e) => failed(e),
    }
}

#[derive(Debug, Deserialize)]
pub struct EventsParams {
    pub limit: Option<i64>,
}

/// `GET /api/models/{id}/events?limit=`
pub async fn events(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
    Query(params): Query<EventsParams>,
) -> Response {
    let limit = params.limit.unwrap_or(100).clamp(1, 500);
    let owned: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT model_id FROM ai_models WHERE model_id = $1 AND created_by = $2")
            .bind(&id)
            .bind(token.user_id())
            .fetch_optional(&state.pg)
            .await;
    match owned {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(),
        Err(e) => return failed(e),
    }
    let rows: Result<Vec<(String, Value, Option<String>, chrono::DateTime<chrono::Utc>)>, _> =
        sqlx::query_as(
            "SELECT kind, payload, actor::text, created_at FROM model_events \
             WHERE model_id = $1 AND kind <> 'inference_trace' \
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(&id)
        .bind(limit)
        .fetch_all(&state.pg)
        .await;
    match rows {
        Ok(rows) => Json(json!({
            "model_id": id,
            "events": rows.into_iter().map(|(kind, payload, actor, at)| json!({
                "kind": kind, "payload": payload, "actor": actor, "created_at": at,
            })).collect::<Vec<Value>>(),
        }))
        .into_response(),
        Err(e) => failed(e),
    }
}

#[cfg(test)]
mod tests {
    /// Inference traces are not activity; they have their own route and would
    /// bury the promotions in a busy model's feed.
    #[test]
    fn the_activity_feed_leaves_out_inference_traces() {
        let src = include_str!("model_training_progress.rs");
        let body = &src[..src.find("#[cfg(test)]").expect("tests module")];
        assert!(body.contains("kind <> 'inference_trace'"));
    }
}
