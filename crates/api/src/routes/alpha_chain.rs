//! `GET /api/ledger/experiments/{id}/alpha_chain`: the five-stage alpha-chain audit
//! (Set Q Q-1.7).
//!
//! A read over `mlops.gate_verdict`, the tenant-scoped append-only record every gate
//! evaluation writes. It dispatches nothing and spends no trial. The arithmetic is
//! `backtest::gates::alpha_chain::audit`; this is only the read, scoped to the session's
//! tenant through the same transaction-local `app.tenant_id` every ledger read uses, so a
//! caller cannot audit another tenant's experiment by naming it.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use backtest::gates::alpha_chain::{audit, GateRow};
use chrono::{DateTime, Utc};
use serde_json::json;
use sqlx::Row;

use crate::{auth::BearerToken, state::AppState};

/// Every gate verdict recorded for an experiment, oldest first.
async fn rows(
    state: &AppState,
    tenant: &str,
    experiment_id: &str,
) -> Result<Vec<GateRow>, sqlx::Error> {
    let mut tx = ledger::pg::tenant_tx(&state.pg, tenant).await?;
    let rows = sqlx::query(
        "SELECT gate_no, gate_name, profile_id, passed, statistic, detail, decided_at \
         FROM mlops.gate_verdict WHERE tenant_id = $1 AND experiment_id = $2 \
         ORDER BY decided_at, gate_no",
    )
    .bind(tenant)
    .bind(experiment_id)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    rows.iter()
        .map(|r| {
            Ok(GateRow {
                gate_no: r.try_get("gate_no")?,
                gate_name: r.try_get("gate_name")?,
                profile_id: r.try_get("profile_id")?,
                passed: r.try_get("passed")?,
                statistic: r.try_get("statistic")?,
                detail: r.try_get("detail")?,
                decided_at: r.try_get::<DateTime<Utc>, _>("decided_at")?,
            })
        })
        .collect()
}

pub async fn alpha_chain(
    State(state): State<AppState>,
    token: BearerToken,
    Path(experiment_id): Path<String>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    match rows(&state, &tenant, &experiment_id).await {
        Ok(found) => {
            let chain = audit(&found);
            Json(json!({
                "experiment_id": experiment_id,
                "gate_rows_read": found.len(),
                "complete": chain.complete,
                "first_unresolved": chain.first_unresolved,
                "stages": chain.stages,
                "reading": "a stage is `pass` only when every gate row it reads exists and passed; a missing row, or one that could not run, is `unknown`, never `pass`",
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "internal", "message": e.to_string() })),
        )
            .into_response(),
    }
}
