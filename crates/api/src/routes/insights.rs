//! The insight store's HTTP surface (SPEC §5.6 and §15's `search_insights` /
//! `write_insight`, checklist 3.10, AGENT-007 §4).
//!
//! Before this module the store had no reader at all. Four handlers:
//! retrieval, write, contradict, reconfirm.
//!
//! **Retrieval is a POST.** Not because the request mutates anything it should
//! not — it appends to `knowledge.memory_retrieval`, which is the audit trail —
//! but because the query carries a structured scope filter *and* a research
//! question, and a research question does not belong in a URL, a proxy log or a
//! browser history.
//!
//! **No embedding key, no client, no URL anywhere in this file.** The embedder
//! is [`insight_memory::LocalHashEmbedder`], which runs in-process;
//! `crates/api/tests/insight_egress.rs` asserts that against this source.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use uuid::Uuid;

use crate::insight_memory::{self, InsightQuery, LocalHashEmbedder, NewInsight};
use crate::knowledge_jobs::InsightStore;
use crate::{auth::BearerToken, state::AppState};

fn backend(e: &sqlx::Error) -> axum::response::Response {
    // A CHECK rejection is the caller's fault and reads as one — notably the
    // evidence requirement, which is the store's oldest rule: no evidence, no
    // insight.
    let msg = e.to_string();
    let is_check = msg.contains("violates check constraint");
    let (code, kind) = if is_check {
        (StatusCode::UNPROCESSABLE_ENTITY, "rejected_by_constraint")
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, "backend_error")
    };
    (code, Json(json!({ "error": kind, "message": msg }))).into_response()
}

/// `POST /api/knowledge/insights/search` — metadata-filtered, capped retrieval.
///
/// The response always restates what retrieved memory is for. An agent reading
/// this is reading claims to weigh, not instructions to follow, and none of it
/// is evidence for a new Experiment.
pub async fn search(
    State(state): State<AppState>,
    token: BearerToken,
    Json(query): Json<InsightQuery>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let store = InsightStore::new(state.pg.clone());
    match store.retrieve(&tenant, &query, &LocalHashEmbedder).await {
        Ok(result) => Json(result).into_response(),
        Err(e) => backend(&e),
    }
}

/// `POST /api/knowledge/insights` — record a claim, embedded locally.
pub async fn write(
    State(state): State<AppState>,
    token: BearerToken,
    Json(new): Json<NewInsight>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let store = InsightStore::new(state.pg.clone());
    match store.write(&tenant, &new, &LocalHashEmbedder).await {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({
                "insight_id": id,
                "embedding_version": insight_memory::LOCAL_EMBEDDING_VERSION,
            })),
        )
            .into_response(),
        Err(e) => backend(&e),
    }
}

/// `POST /api/knowledge/insights/{id}/contradict` — new evidence disagrees.
///
/// Decays; never deletes. The response says so explicitly, including the score
/// and whether the claim is still within retrieval's reach, because "it is gone
/// from the answers" and "it is gone" are different facts and a caller that
/// confuses them will eventually write a cleanup job.
pub async fn contradict(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let store = InsightStore::new(state.pg.clone());
    match store.contradict(&tenant, id).await {
        Ok(score) => Json(json!({
            "insight_id": id,
            "decay_score": score,
            "retrievable": score >= insight_memory::RETRIEVAL_FLOOR,
            "retrieval_floor": insight_memory::RETRIEVAL_FLOOR,
            "row_retained": true,
            "note": "Contradiction decays a claim out of retrieval. The row is never deleted: \
                     that a belief was held and then contradicted is itself knowledge, and a \
                     claim that vanishes can be rediscovered and re-believed indefinitely.",
        }))
        .into_response(),
        Err(e) => backend(&e),
    }
}

/// `POST /api/knowledge/insights/{id}/reconfirm` — a later run agreed.
pub async fn reconfirm(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let store = InsightStore::new(state.pg.clone());
    match store.reconfirm(&tenant, id).await {
        Ok(score) => Json(json!({
            "insight_id": id,
            "decay_score": score,
            "retrievable": score >= insight_memory::RETRIEVAL_FLOOR,
        }))
        .into_response(),
        Err(e) => backend(&e),
    }
}
