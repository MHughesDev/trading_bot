//! The monitor's HTTP surface (COMP-007 §6).
//!
//! Read-mostly, served by the API from the monitor's tables. The monitor itself is
//! a different process and does not run here.
//!
//! **Every mutating route in this file goes through [`require_control`] and nothing
//! else.** One gate, asked by name, so that adding a route without answering the
//! authority question is not something you can do by forgetting. The agent's
//! service token carries none of these scopes, and `crates/api/tests/
//! monitor_authority.rs` mints one and proves it per scope rather than per
//! endpoint.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use monitor::control::ResetRequest;
use monitor::drill::{DrillOutcome, DrillTrigger};
use monitor::events::{EventKind, MonitorEvent};
use monitor::family::Family;
use monitor::limits::{ControlLimit, LimitMethod, LimitSeverity};
use monitor::precision::PrecisionLabel;
use monitor::state::{HaltPolicy, StrategyState};
use monitor::store::MonitorStore;
use monitor::window::MaintenanceWindow;
use risk::Scope;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{auth::scopes, auth::BearerToken, state::AppState};

/// The single authority check for every monitor mutation.
///
/// Returns the `human:<user_id>` actor string on success. Returning the actor
/// rather than `()` is deliberate: it makes it awkward to write a handler that
/// checks authority and then records somebody else as the author.
fn require_control(token: &BearerToken) -> Result<String, Response> {
    if !scopes::permits_monitor_control(&token.scopes) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "monitor_control_required",
                "detail": "tripping, scaling, halting, resuming or resetting the monitor \
                           requires a human session; agent and service tokens never carry it",
            })),
        )
            .into_response());
    }
    Ok(format!("human:{}", token.user_id))
}

fn store(state: &AppState) -> MonitorStore {
    MonitorStore::new(state.pg.clone())
}

/// The same store, for the one caller outside this module.
///
/// `proposals::write_limit` writes a control limit as the human who approved it,
/// and it goes through this rather than building its own `MonitorStore` — a
/// second way to reach the limit table is a second place the store's own rules
/// could be bypassed, which is the argument `arm_by_id` already makes for
/// arming.
#[must_use]
pub fn limit_store(state: &AppState) -> MonitorStore {
    store(state)
}

fn bad_request(detail: impl std::fmt::Display) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "invalid_request", "detail": detail.to_string() })),
    )
        .into_response()
}

fn server_error(detail: impl std::fmt::Display) -> Response {
    tracing::error!(error = %detail, "monitor route failed");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "monitor_unavailable" })),
    )
        .into_response()
}

fn parse_scope(raw: &str) -> Result<Scope, Response> {
    raw.parse::<Scope>().map_err(bad_request)
}

// ── Reads ───────────────────────────────────────────────────────────────────

/// `GET /api/monitor/kill-switch`
///
/// The desk's banner reads this. It reports the durable table, not the in-process
/// mirror, because the question the operator is asking is "is this platform
/// stopped", and the answer that survives a restart is the one that means that.
pub async fn get_kill_switch(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let s = store(&state);
    let halted = s.halted_scopes().await.map_err(server_error)?;
    let lease = s.lease_status(chrono::Utc::now()).await;
    let windows = s.open_windows().await.map_err(server_error)?;

    Ok(Json(json!({
        "lease": format!("{lease:?}").to_lowercase(),
        "orders_permitted": lease.permits_orders() && !halted.contains(&Scope::Global),
        "halted_scopes": halted.iter().map(ToString::to_string).collect::<Vec<_>>(),
        // §4.3: an open window is as visible in the desk chrome as a trip, because
        // a window nobody can see is a hole nobody can audit.
        "maintenance_windows": windows.iter().map(|w| json!({
            "window_id": w.window_id,
            "opened_by": w.opened_by,
            "reason": w.reason,
            "expires_at": w.expires_at.to_rfc3339(),
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    limit: Option<i64>,
}

/// `GET /api/monitor/events`
pub async fn list_events(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let rows = store(&state)
        .recent_events(q.limit.unwrap_or(100).clamp(1, 1000))
        .await
        .map_err(server_error)?;
    Ok(Json(json!({
        "events": rows.into_iter().map(|(id, scope, family, kind)| json!({
            "event_id": id, "scope": scope, "family": family, "kind": kind,
        })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/monitor/strategies`
pub async fn list_strategies(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let rows = store(&state).strategy_states().await.map_err(server_error)?;
    Ok(Json(json!({
        "strategies": rows.into_iter().map(|(id, state, scale_pct)| json!({
            "strategy_id": id, "state": state, "scale_pct": scale_pct,
        })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/monitor/precision?month=YYYY-MM-DD`
pub async fn get_precision(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<PrecisionQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let month = q
        .month
        .unwrap_or_else(|| {
            // `Datelike::with_day` needs the trait in scope; spelled out here
            // instead, because the first of the current month is what is meant
            // and `from_ymd_opt` says so without an import.
            let today = chrono::Utc::now().date_naive();
            chrono::NaiveDate::from_ymd_opt(
                chrono::Datelike::year(&today),
                chrono::Datelike::month(&today),
                1,
            )
            .unwrap_or(today)
        });
    let rows = store(&state)
        .compute_precision(month)
        .await
        .map_err(server_error)?;
    Ok(Json(json!({
        "month": month.to_string(),
        "families": rows.into_iter().map(|r| json!({
            "family": r.family.as_str(),
            "true_positives": r.true_positives,
            "false_positives": r.false_positives,
            // Null, not zero and not one: a family nobody labelled is unmeasured.
            "precision": r.precision,
            "flagged_for_rederivation": r.flagged,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct PrecisionQuery {
    #[serde(default)]
    month: Option<chrono::NaiveDate>,
}

// ── Mutations ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TripBody {
    reason: String,
    #[serde(default)]
    metric: Option<String>,
}

/// `POST /api/monitor/kill-switch/{scope}/trip`
pub async fn trip(
    token: BearerToken,
    State(state): State<AppState>,
    Path(scope): Path<String>,
    Json(body): Json<TripBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    let scope = parse_scope(&scope)?;
    if body.reason.trim().is_empty() {
        return Err(bad_request("a trip requires a reason"));
    }

    let mut event = MonitorEvent::new(
        scope.clone(),
        Family::Platform,
        EventKind::Tripped,
        &actor,
    )
    .map_err(bad_request)?
    .with_reason(body.reason.clone());
    event.metric = body.metric;

    store(&state).trip(&event, &body.reason).await.map_err(server_error)?;
    // The mirror is refreshed from the table rather than set directly, so the
    // in-memory view can never say something the durable record does not.
    state.kill_switch.trip_scope(&scope);

    Ok(Json(json!({ "scope": scope.to_string(), "state": "halted" })))
}

#[derive(Debug, Deserialize)]
pub struct ResetBody {
    /// The scope name, typed out. Must match exactly.
    confirm_scope: String,
    reason: String,
}

/// `POST /api/monitor/kill-switch/{scope}/reset`
pub async fn reset(
    token: BearerToken,
    State(state): State<AppState>,
    Path(scope): Path<String>,
    Json(body): Json<ResetBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    let scope = parse_scope(&scope)?;
    let s = store(&state);

    // §5: a global reset requires every venue to reconcile clean first. "No venue
    // is halted" is the platform's own record of that, and it is checked here
    // rather than trusted from the request body — a precondition the caller
    // asserts is not a precondition.
    let halted = s.halted_scopes().await.map_err(server_error)?;
    let all_venues_reconciled = !halted.iter().any(|x| matches!(x, Scope::Venue(_)));

    let request = ResetRequest {
        scope: scope.clone(),
        typed_confirmation: body.confirm_scope,
        reason: body.reason,
        actor,
        all_venues_reconciled,
    };

    match s.reset(&request).await {
        Ok(()) => {
            state.kill_switch.reset_scope(&scope);
            Ok(Json(json!({ "scope": scope.to_string(), "state": "armed" })))
        }
        Err(monitor::store::StoreError::Reset(refusal)) => Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": "reset_refused", "detail": refusal.to_string() })),
        )
            .into_response()),
        Err(e) => Err(server_error(e)),
    }
}

#[derive(Debug, Deserialize)]
pub struct StrategyActionBody {
    #[serde(default)]
    reason: Option<String>,
    /// For `scale`: the percentage of full size.
    #[serde(default)]
    scale_pct: Option<u8>,
    /// For `resume`: the strategy id, typed out.
    #[serde(default)]
    confirm_scope: Option<String>,
    /// For `halt`: what to do with the open position.
    #[serde(default)]
    halt_policy: Option<HaltPolicy>,
}

/// `POST /api/monitor/strategies/{id}/{action}` where action is `scale|halt|resume`.
pub async fn strategy_action(
    token: BearerToken,
    State(state): State<AppState>,
    Path((strategy_id, action)): Path<(String, String)>,
    Json(body): Json<StrategyActionBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    let scope = Scope::strategy(&strategy_id);
    let s = store(&state);

    match action.as_str() {
        "scale" => {
            let pct = body.scale_pct.ok_or_else(|| bad_request("scale_pct is required"))?;
            if pct == 0 || pct > 100 {
                return Err(bad_request("scale_pct must be in 1..=100"));
            }
            let event = MonitorEvent::new(scope, Family::Behaviour, EventKind::ScaledDown, &actor)
                .map_err(bad_request)?
                .with_reason(body.reason.unwrap_or_else(|| "manual scale".to_owned()));
            s.record_event(&event).await.map_err(server_error)?;
            s.set_strategy_state(&strategy_id, StrategyState::ScaledDown.as_str(), i16::from(pct))
                .await
                .map_err(server_error)?;
            state.kill_switch.scale_strategy(&strategy_id, pct);
            Ok(Json(json!({ "strategy_id": strategy_id, "scale_pct": pct })))
        }
        "halt" => {
            let reason = body
                .reason
                .ok_or_else(|| bad_request("a halt requires a reason"))?;
            let policy = body.halt_policy.unwrap_or(HaltPolicy::Hold);
            let event = MonitorEvent::new(scope.clone(), Family::Pnl, EventKind::Halted, &actor)
                .map_err(bad_request)?
                .with_reason(reason.clone());
            s.trip(&event, &reason).await.map_err(server_error)?;
            s.set_strategy_state(&strategy_id, StrategyState::Halted.as_str(), 0)
                .await
                .map_err(server_error)?;
            state.kill_switch.trip_scope(&scope);
            Ok(Json(json!({
                "strategy_id": strategy_id,
                "state": "halted",
                "halt_policy": policy.as_str(),
            })))
        }
        // Resuming a halted strategy is a reset by another name, and takes the
        // same typed confirmation, reason and rate limit. Routing it through the
        // same code is the point: two paths to the same authority is one path plus
        // a hole.
        "resume" => {
            let request = ResetRequest {
                scope: scope.clone(),
                typed_confirmation: body
                    .confirm_scope
                    .unwrap_or_default(),
                reason: body.reason.unwrap_or_default(),
                actor,
                all_venues_reconciled: true,
            };
            match s.reset(&request).await {
                Ok(()) => {
                    state.kill_switch.reset_scope(&scope);
                    s.set_strategy_state(&strategy_id, StrategyState::Nominal.as_str(), 100)
                        .await
                        .map_err(server_error)?;
                    state.kill_switch.scale_strategy(&strategy_id, 100);
                    Ok(Json(json!({ "strategy_id": strategy_id, "state": "nominal" })))
                }
                Err(monitor::store::StoreError::Reset(refusal)) => Err((
                    StatusCode::CONFLICT,
                    Json(json!({ "error": "resume_refused", "detail": refusal.to_string() })),
                )
                    .into_response()),
                Err(e) => Err(server_error(e)),
            }
        }
        other => Err(bad_request(format!(
            "unknown action {other:?}; expected scale, halt or resume"
        ))),
    }
}

#[derive(Debug, Deserialize)]
pub struct DeclareLimitBody {
    metric: String,
    family: String,
    #[serde(default)]
    severity: Option<String>,
    version: i32,
    #[serde(default)]
    lower: Option<f64>,
    #[serde(default)]
    upper: Option<f64>,
    method: String,
    /// The backtest or paper run this baseline came from.
    declared_from: String,
}

/// `PUT /api/monitor/limits/{strategy_id}`
pub async fn declare_limit(
    token: BearerToken,
    State(state): State<AppState>,
    Path(strategy_id): Path<String>,
    Json(body): Json<DeclareLimitBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    if body.declared_from.trim().is_empty() {
        return Err(bad_request(
            "declared_from must name the backtest or paper run the baseline came from",
        ));
    }
    let family: Family = body.family.parse().map_err(bad_request)?;
    let severity: LimitSeverity = body
        .severity
        .as_deref()
        .unwrap_or("breach")
        .parse()
        .map_err(bad_request)?;
    let method: LimitMethod = match body.method.as_str() {
        "paper_ewma_3sigma" => LimitMethod::PaperEwma3Sigma,
        "backtest_3sigma" => LimitMethod::Backtest3Sigma,
        "venue_rate_ceiling" => LimitMethod::VenueRateCeiling,
        "risk_gate_mirror" => LimitMethod::RiskGateMirror,
        "declared_absolute" => LimitMethod::DeclaredAbsolute,
        "mc_p95_drawdown" => LimitMethod::McP95Drawdown,
        "cusum" => LimitMethod::Cusum,
        other => return Err(bad_request(format!("unknown method {other:?}"))),
    };

    let limit = ControlLimit {
        strategy_id: strategy_id.clone(),
        metric: body.metric,
        family,
        severity,
        version: body.version,
        lower: body.lower,
        upper: body.upper,
        method,
        declared_from: body.declared_from,
        declared_at: chrono::Utc::now(),
        declared_by: actor,
    };
    store(&state).declare_limit(&limit).await.map_err(server_error)?;
    Ok(Json(json!({ "strategy_id": strategy_id, "version": limit.version })))
}

/// `GET /api/monitor/limits/{strategy_id}/promotion-gate`
///
/// COMP-002 §3, amended: a strategy with no declared limits cannot be promoted to
/// `live`. Exposed as its own route so the promotion path has one thing to call and
/// cannot half-implement the check.
pub async fn promotion_gate(
    _token: BearerToken,
    State(state): State<AppState>,
    Path(strategy_id): Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    let book = store(&state).load_limits().await.map_err(server_error)?;
    match book.may_promote_to_live(&strategy_id) {
        Ok(()) => Ok(Json(json!({ "strategy_id": strategy_id, "may_promote": true }))),
        Err(refusal) => Ok(Json(json!({
            "strategy_id": strategy_id,
            "may_promote": false,
            "detail": refusal.to_string(),
        }))),
    }
}

#[derive(Debug, Deserialize)]
pub struct DrillBody {
    #[serde(default)]
    trigger: Option<String>,
    cadence_ms: i64,
    detection_ms: i64,
    propagation_ms: i64,
    notification_ms: i64,
    #[serde(default)]
    refused_real: bool,
}

/// `POST /api/monitor/drill` — record a rehearsal (COMP-007 §4.2).
pub async fn record_drill(
    token: BearerToken,
    State(state): State<AppState>,
    Json(body): Json<DrillBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    let trigger = match body.trigger.as_deref().unwrap_or("manual") {
        "deploy" => DrillTrigger::Deploy,
        "scheduled" => DrillTrigger::Scheduled,
        "manual" => DrillTrigger::Manual,
        other => return Err(bad_request(format!("unknown trigger {other:?}"))),
    };
    let outcome = DrillOutcome::record(
        Scope::drill("global"),
        trigger,
        chrono::Utc::now(),
        body.cadence_ms,
        body.detection_ms,
        body.propagation_ms,
        body.notification_ms,
        body.refused_real,
        &actor,
    )
    .map_err(bad_request)?;

    store(&state).record_drill(&outcome).await.map_err(server_error)?;
    Ok(Json(json!({
        "drill_id": outcome.drill_id,
        "passed": outcome.passed(),
        "blocks_deploy": outcome.blocks_deploy(),
        "propagation_ms": outcome.propagation_ms,
        "cadence_ms": outcome.cadence_ms,
    })))
}

#[derive(Debug, Deserialize)]
pub struct WindowBody {
    reason: String,
    #[serde(default)]
    minutes: Option<i64>,
}

/// `POST /api/monitor/windows` — open a maintenance window (COMP-007 §4.3).
pub async fn open_window(
    token: BearerToken,
    State(state): State<AppState>,
    Json(body): Json<WindowBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    let window = MaintenanceWindow::open(
        &actor,
        &body.reason,
        body.minutes.unwrap_or(monitor::window::DEFAULT_WINDOW_MINUTES),
        chrono::Utc::now(),
    )
    .map_err(bad_request)?;
    store(&state).open_window(&window).await.map_err(server_error)?;
    Ok(Json(json!({
        "window_id": window.window_id,
        "expires_at": window.expires_at.to_rfc3339(),
        "suppresses": ["platform"],
    })))
}

/// `DELETE /api/monitor/windows/{id}`
pub async fn close_window(
    token: BearerToken,
    State(state): State<AppState>,
    Path(window_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, Response> {
    require_control(&token)?;
    store(&state).close_window(window_id).await.map_err(server_error)?;
    Ok(Json(json!({ "window_id": window_id, "closed": true })))
}

#[derive(Debug, Deserialize)]
pub struct LabelBody {
    family: String,
    label: String,
    #[serde(default)]
    note: Option<String>,
}

/// `POST /api/monitor/events/{id}/label` — score the monitor (COMP-007 §4.1).
pub async fn label_breach(
    token: BearerToken,
    State(state): State<AppState>,
    Path(event_id): Path<Uuid>,
    Json(body): Json<LabelBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    let family: Family = body.family.parse().map_err(bad_request)?;
    let label = match body.label.as_str() {
        "true_positive" => PrecisionLabel::TruePositive,
        "false_positive" => PrecisionLabel::FalsePositive,
        other => return Err(bad_request(format!("unknown label {other:?}"))),
    };
    store(&state)
        .label_breach(event_id, family, label, &actor, body.note.as_deref())
        .await
        .map_err(server_error)?;
    Ok(Json(json!({ "event_id": event_id, "label": label.as_str() })))
}
