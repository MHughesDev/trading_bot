//! The pre-COMP-007 kill-switch routes, brought under the monitor's rules.
//!
//! These three endpoints predate COMP-007 and had two defects that the spec exists
//! to remove:
//!
//! 1. **Any bearer token could call them.** A service token minted for an agent
//!    would have satisfied `BearerToken` and been able to halt — or, worse, resume —
//!    the platform. They now require `monitor.control`, which no agent session can
//!    hold (migration 0075 refuses it in the database as well).
//! 2. **The trip was not durable.** `trip()` flipped an in-process `AtomicBool` and
//!    never wrote `global_risk_config.trading_enabled`, which is the column
//!    `apps/platform` reads at boot. So a switch tripped on Monday was silently
//!    cleared by the next restart, and nothing anywhere logged that the platform
//!    had decided to resume. They now write the durable `monitor.kill_switch` row
//!    and the legacy column in one transaction.
//!
//! They are kept rather than removed because the frontend and existing scripts call
//! them; they now delegate to exactly the same code as `/api/monitor/*`.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use monitor::control::ResetRequest;
use monitor::events::{EventKind, MonitorEvent};
use monitor::family::Family;
use monitor::store::MonitorStore;
use risk::Scope;
use serde::Deserialize;
use serde_json::json;

use crate::{auth::scopes, auth::BearerToken, state::AppState};

/// GET /api/trading/status — current kill switch / trading state.
pub async fn trading_status(
    _token: BearerToken,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let halted = state.kill_switch.is_active();
    let monitor_available = state.kill_switch.monitor_available();
    Json(json!({
        // Trading is enabled only when nothing is halted *and* the monitor is
        // alive. Reporting `!halted` alone would tell an operator the platform is
        // trading while every order is being refused with monitor_unavailable.
        "trading_enabled": !halted && monitor_available,
        "kill_switch_active": halted,
        "monitor_available": monitor_available,
    }))
}

fn forbidden() -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "monitor_control_required",
            "detail": "halting or resuming trading requires a human session",
        })),
    )
        .into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct KillBody {
    #[serde(default)]
    reason: Option<String>,
}

/// POST /api/trading/kill — trip the global scope, durably.
pub async fn trip_kill_switch(
    token: BearerToken,
    State(state): State<AppState>,
    body: Option<Json<KillBody>>,
) -> axum::response::Response {
    if !scopes::permits_monitor_control(&token.scopes) {
        return forbidden();
    }
    let actor = format!("human:{}", token.user_id);
    let reason = body
        .and_then(|Json(b)| b.reason)
        .unwrap_or_else(|| "manual kill from /api/trading/kill".to_owned());

    let event = match MonitorEvent::new(Scope::Global, Family::Platform, EventKind::Tripped, &actor)
    {
        Ok(e) => e.with_reason(reason.clone()),
        Err(e) => return (StatusCode::FORBIDDEN, Json(json!({ "error": e.to_string() }))).into_response(),
    };

    // Durable first, mirror second. If the write fails the switch is not tripped
    // and the caller is told — which is better than an in-memory halt that the
    // next restart forgets and nobody is told about.
    if let Err(e) = MonitorStore::new(state.pg.clone()).trip(&event, &reason).await {
        tracing::error!(error = %e, "could not persist the kill switch");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "kill_switch_not_persisted" })),
        )
            .into_response();
    }
    state.kill_switch.trip();

    (
        StatusCode::OK,
        Json(json!({
            "message": "kill switch tripped — all new orders halted",
            "durable": true,
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct ResumeBody {
    /// The scope name, typed out: `global`.
    confirm_scope: String,
    reason: String,
}

/// POST /api/trading/resume — release the global halt.
///
/// Typed, reasoned and rate-limited like every other reset (COMP-007 §5). The old
/// version took no body at all, which made resuming the whole platform a single
/// unauthenticated-in-practice click.
pub async fn reset_kill_switch(
    token: BearerToken,
    State(state): State<AppState>,
    Json(body): Json<ResumeBody>,
) -> axum::response::Response {
    if !scopes::permits_monitor_control(&token.scopes) {
        return forbidden();
    }
    let store = MonitorStore::new(state.pg.clone());
    let halted = store.halted_scopes().await.unwrap_or_else(|e| {
        tracing::error!(error = %e, "could not read halted scopes; treating a venue as unreconciled");
        vec![Scope::venue("unreadable")]
    });
    let all_venues_reconciled = !halted.iter().any(|x| matches!(x, Scope::Venue(_)));

    let request = ResetRequest {
        scope: Scope::Global,
        typed_confirmation: body.confirm_scope,
        reason: body.reason,
        actor: format!("human:{}", token.user_id),
        all_venues_reconciled,
    };

    match store.reset(&request).await {
        Ok(()) => {
            state.kill_switch.reset();
            (
                StatusCode::OK,
                Json(json!({ "message": "kill switch reset — order flow resumed" })),
            )
                .into_response()
        }
        Err(monitor::store::StoreError::Reset(refusal)) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "reset_refused", "detail": refusal.to_string() })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "reset failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "reset_failed" })),
            )
                .into_response()
        }
    }
}
