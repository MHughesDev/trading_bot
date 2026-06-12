//! Automations API — list, create, arm/disarm, delete.
//!
//! Automations are persisted in Postgres (`automations` table, migration
//! 0010) and are **server-side state**: an armed automation stays armed —
//! paper or live — regardless of which mode any UI tab is currently
//! displaying, and across UI sessions.  Paper and live automations coexist;
//! the `account_mode` column decides which execution path each one routes to.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use storage::automation::{self, AutomationRow};
use strategy_runtime::automation::plan::AutomationSpec;

use crate::{auth::BearerToken, state::AppState};

/// Placeholder user until Phase 2 session auth lands (M-17): all automations
/// belong to the single local operator.
const DEV_USER: Uuid = Uuid::nil();

// ── List ──────────────────────────────────────────────────────────────────────

/// GET /api/automations — every automation, paper and live, newest first.
pub async fn list_automations(
    State(state): State<AppState>,
    _token: BearerToken,
) -> impl IntoResponse {
    match automation::list_automations(&state.pg).await {
        Ok(rows) => {
            let automations: Vec<serde_json::Value> = rows
                .iter()
                .map(|r| {
                    json!({
                        "id": r.id,
                        "kind": r.kind,
                        "account_mode": r.account_mode,
                        "spec": r.spec,
                        "armed": r.armed,
                        // Armed automations run server-side while the platform
                        // is up — independent of any UI session or mode toggle.
                        "active": r.armed,
                        "created_at": r.created_at,
                    })
                })
                .collect();
            Json(json!({ "automations": automations })).into_response()
        }
        Err(e) => db_error(e),
    }
}

// ── Create ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAutomationRequest {
    /// `"single_instrument"` | `"pipeline"`.
    pub kind: String,
    /// `"paper"` | `"live"`.
    pub account_mode: String,
    /// Untagged spec payload (the `kind` field above selects the variant).
    pub spec: serde_json::Value,
    #[serde(default)]
    pub armed: bool,
}

/// POST /api/automations — validate and persist a new automation plan.
pub async fn create_automation(
    State(state): State<AppState>,
    _token: BearerToken,
    Json(req): Json<CreateAutomationRequest>,
) -> impl IntoResponse {
    if !matches!(req.kind.as_str(), "single_instrument" | "pipeline") {
        return bad_request("kind must be 'single_instrument' or 'pipeline'");
    }
    if !matches!(req.account_mode.as_str(), "paper" | "live") {
        return bad_request("account_mode must be 'paper' or 'live'");
    }

    // Validate the spec by parsing it into the typed AutomationSpec model.
    // The spec arrives untagged; inject the request kind as the serde tag.
    let mut tagged = req.spec.clone();
    match tagged.as_object_mut() {
        Some(obj) => {
            obj.insert("kind".to_owned(), json!(req.kind));
        }
        None => return bad_request("spec must be a JSON object"),
    }
    if let Err(e) = serde_json::from_value::<AutomationSpec>(tagged) {
        return bad_request(&format!("invalid {} spec: {e}", req.kind));
    }

    let row = AutomationRow {
        id: Uuid::new_v4(),
        user_id: DEV_USER,
        kind: req.kind,
        account_mode: req.account_mode,
        spec: req.spec,
        armed: req.armed,
        created_at: Utc::now(),
    };

    match automation::insert_automation(&state.pg, &row).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({
                "id": row.id,
                "kind": row.kind,
                "account_mode": row.account_mode,
                "armed": row.armed,
            })),
        )
            .into_response(),
        Err(e) => db_error(e),
    }
}

// ── Arm / disarm / delete ─────────────────────────────────────────────────────

/// POST /api/automations/:id/arm
pub async fn arm_automation(
    state: State<AppState>,
    token: BearerToken,
    id: Path<Uuid>,
) -> impl IntoResponse {
    set_armed(state, token, id, true).await
}

/// POST /api/automations/:id/disarm
pub async fn disarm_automation(
    state: State<AppState>,
    token: BearerToken,
    id: Path<Uuid>,
) -> impl IntoResponse {
    set_armed(state, token, id, false).await
}

async fn set_armed(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
    armed: bool,
) -> axum::response::Response {
    match automation::set_automation_armed(&state.pg, id, armed).await {
        Ok(true) => Json(json!({ "id": id, "armed": armed })).into_response(),
        Ok(false) => not_found(id),
        Err(e) => db_error(e),
    }
}

/// DELETE /api/automations/:id
pub async fn delete_automation(
    State(state): State<AppState>,
    _token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match automation::delete_automation(&state.pg, id).await {
        Ok(true) => Json(json!({ "id": id, "deleted": true })).into_response(),
        Ok(false) => not_found(id),
        Err(e) => db_error(e),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn bad_request(message: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": message })),
    )
        .into_response()
}

fn not_found(id: Uuid) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "not_found", "id": id })),
    )
        .into_response()
}

fn db_error(e: sqlx::Error) -> axum::response::Response {
    tracing::error!(error = %e, "automations database error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "database error" })),
    )
        .into_response()
}
