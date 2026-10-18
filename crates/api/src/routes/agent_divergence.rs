//! `GET /api/agent/divergence` -- where a deployed strategy's return diverged
//! from its backtest (S-3.14, `attribute_divergence`).
//!
//! A thin reader: the forward record comes from the same loader the fleet
//! dashboard uses (`reconciliation::fleet_job::load_window`, tenant-scoped under
//! row-level security) and the arithmetic is
//! `backtest::diagnostics_divergence::decompose`, which says in its own docs what
//! it can and cannot attribute. Nothing here is computed twice.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
pub struct DivergenceQuery {
    pub strategy_id: String,
    /// Look-back in days, 1 to 365. Default 30.
    #[serde(default)]
    pub window_days: Option<i64>,
}

pub async fn divergence(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<DivergenceQuery>,
) -> impl IntoResponse {
    let days = q.window_days.unwrap_or(30);
    if !(1..=365).contains(&days) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "invalid_window_days", "got": days, "fix": "1 to 365"})),
        )
            .into_response();
    }
    let now = chrono::Utc::now();
    let from = now - chrono::Duration::days(days);
    let all = match reconciliation::fleet_job::load_window(
        &state.pg,
        &token.user_id().to_string(),
        from,
        now,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "query_failed", "message": e.to_string()})),
            )
                .into_response()
        }
    };
    let mine: Vec<_> = all
        .into_iter()
        .filter(|o| o.strategy_id == q.strategy_id)
        .collect();
    if mine.is_empty() {
        // Absent, not zero: a strategy with no forward record has no divergence
        // to attribute, and "no divergence" would be a claim about a record that
        // does not exist.
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "no_forward_record",
                "strategy_id": q.strategy_id,
                "window_days": days,
                "message": "no forward observations for this strategy in the window; \
                            nothing has been measured, which is not the same as no divergence",
            })),
        )
            .into_response();
    }
    match backtest::diagnostics_divergence::decompose(&mine) {
        Ok(r) => Json(json!({
            "strategy_id": q.strategy_id,
            "window_days": days,
            "report": r,
            "unmeasured": {
                "timing": "the forward record has no decision-to-fill latency field (Set T's ShadowBar adds none); `recording_lag_*` is how late the row was written, not how late the fill was",
                "data": "the forward record has no per-bar data-quality field",
            },
        }))
        .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "not_attributable", "message": e.to_string()})),
        )
            .into_response(),
    }
}
