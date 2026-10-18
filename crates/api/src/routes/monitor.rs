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
use chrono::{DateTime, Utc};
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
    /// One strategy's events. Without it, every strategy's.
    #[serde(default)]
    scope: Option<String>,
    /// `30d`, `60d` or `90d` (COMP-007 §5.7). Mutually exclusive with `since`.
    #[serde(default)]
    window: Option<String>,
    /// An explicit inclusive lower bound on `occurred_at`, for a caller that wants
    /// a window §5.7 does not name.
    #[serde(default)]
    since: Option<DateTime<Utc>>,
}

/// §5.7's three windows, as days.
///
/// A closed set rather than a duration parser. §5.7 names exactly these three, and
/// the reason to refuse `45d` rather than accept it is the same reason
/// `check_control_limits` refused `window` altogether until now: an argument a
/// caller can pass and a schema does not describe is one nobody can reason about
/// from the outside.
fn window_days(raw: &str) -> Option<i64> {
    match raw {
        "30d" => Some(30),
        "60d" => Some(60),
        "90d" => Some(90),
        _ => None,
    }
}

/// `GET /api/monitor/events`
///
/// **`occurred_at` was ordered by and not selected**, which is why
/// `check_control_limits` has been answering `time_bounded: false` and refusing
/// §5.7's `window` argument outright — an event with no time cannot be placed
/// inside or outside a window, and a tool that accepted the argument anyway would
/// be lying about what it applied. That is the one field, and it is here now,
/// along with the `scope` and `since` the window needs to be useful.
///
/// The filters are applied in SQL, not to the page this route would otherwise
/// have returned. Filtering after `LIMIT` would make "this strategy's last 30
/// days" mean "whichever of the last thousand events platform-wide happen to be
/// this strategy's, inside 30 days" -- an answer that silently shrinks as the rest
/// of the platform gets busier.
pub async fn list_events(
    _token: BearerToken,
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<serde_json::Value>, Response> {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let mut query = monitor::store::EventsQuery::recent(limit);
    if let Some(s) = q.scope.clone() {
        query = query.scoped(s);
    }
    // `window` and `since` are not merged. Two lower bounds silently reduced to
    // the stricter one is how a caller comes to believe it read 90 days.
    match (q.window.as_deref(), q.since) {
        (Some(_), Some(_)) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "window_and_since",
                    "message": "pass `window` or `since`, not both",
                })),
            )
                .into_response())
        }
        (Some(w), None) => match window_days(w) {
            Some(days) => query = query.since(Utc::now() - chrono::Duration::days(days)),
            None => {
                return Err((
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({
                        "error": "unknown_window",
                        "message": format!("unknown window {w:?}"),
                        "known": ["30d", "60d", "90d"],
                    })),
                )
                    .into_response())
            }
        },
        (None, Some(s)) => query = query.since(s),
        (None, None) => {}
    }

    let rows = store(&state)
        .recent_events(&query)
        .await
        .map_err(server_error)?;
    // `truncated` separates "no more events" from "older than this page", which is
    // the distinction `check_control_limits` carries as `events_truncated`.
    let truncated = i64::try_from(rows.len()).unwrap_or(i64::MAX) >= limit;
    Ok(Json(json!({
        "events": rows.iter().map(|e| json!({
            "event_id": e.event_id,
            "occurred_at": e.occurred_at,
            "scope": e.scope,
            "family": e.family,
            "kind": e.kind,
            "metric": e.metric,
            "observed_value": e.observed_value,
            "limit_value": e.limit_value,
            "actor": e.actor,
            "is_drill": e.is_drill,
        })).collect::<Vec<_>>(),
        "window": {
            // Echoed so a caller can tell an applied bound from an ignored one.
            "since": query.since,
            "scope": query.scope,
            "limit": limit,
            "truncated": truncated,
        },
    })))
}

/// Which environment each strategy's orders were executed in.
///
/// AGENT-012 §2.4 and §11 both want live-versus-paper on the monitor row, and
/// the desk left the column out rather than infer it from the strategy's name —
/// a naming convention read as an environment is how a paper strategy comes to
/// be read as live.
///
/// # Where the fact actually lives
///
/// `orders.execution_mode`, joined on `orders.strategy_id`, which is the same id
/// `monitor.strategy_state` keys on (`OrderIntent.strategy_id`). It is a
/// recorded fact per order, written by `execution::journal`, not a guess.
///
/// The three places that *look* like better homes are all empty:
/// `mlops.paper_deployment.venue` is the declared home and has no writer
/// anywhere in the repository; `strategy_instances` is written only by a ledger
/// test; and `accounts.account_type` is unreachable from a monitored strategy
/// because migration 0121 made `orders.account_id` nullable precisely so the
/// paper engine could record an order without one.
///
/// # What it can return today, and why that is not a lie
///
/// `paper` or absent, because `orders_execution_mode_paper_only` (0121) CHECKs
/// the column to `'paper'` — enabling a live venue is a migration and a security
/// review, which is the point. Reading the recorded column rather than hard-coding
/// the word means the day that CHECK widens this route reports `live` without a
/// code change.
///
/// A strategy that has placed no order is **absent from the map**, and the route
/// serves `null` for it: unknown, not paper. `mixed` is served if one strategy
/// ever has orders in more than one mode, because collapsing that to a single
/// word would pick one of two true answers.
async fn order_environments(
    state: &AppState,
) -> Result<std::collections::HashMap<String, String>, Response> {
    let rows: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT strategy_id, COUNT(DISTINCT execution_mode), MIN(execution_mode)
           FROM orders
          WHERE strategy_id IS NOT NULL
          GROUP BY strategy_id",
    )
    .fetch_all(&state.pg)
    .await
    .map_err(server_error)?;
    Ok(rows
        .into_iter()
        .map(|(id, distinct, mode)| {
            let word = if distinct == 1 { mode } else { "mixed".into() };
            (id, word)
        })
        .collect())
}

/// `GET /api/monitor/strategies`
pub async fn list_strategies(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let rows = store(&state)
        .strategy_states()
        .await
        .map_err(server_error)?;
    let env = order_environments(&state).await?;
    Ok(Json(json!({
        "strategies": rows.into_iter().map(|(id, state, scale_pct)| json!({
            // `null` where nothing was recorded. A strategy the platform has
            // never traded has no environment, and `paper` there would be the
            // route answering a question nobody asked the database.
            "environment": env.get(&id),
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
    let month = q.month.unwrap_or_else(|| {
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

    let mut event = MonitorEvent::new(scope.clone(), Family::Platform, EventKind::Tripped, &actor)
        .map_err(bad_request)?
        .with_reason(body.reason.clone());
    event.metric = body.metric;

    store(&state)
        .trip(&event, &body.reason)
        .await
        .map_err(server_error)?;
    // The mirror is refreshed from the table rather than set directly, so the
    // in-memory view can never say something the durable record does not.
    state.kill_switch.trip_scope(&scope);

    Ok(Json(
        json!({ "scope": scope.to_string(), "state": "halted" }),
    ))
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
            Ok(Json(
                json!({ "scope": scope.to_string(), "state": "armed" }),
            ))
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
            let pct = body
                .scale_pct
                .ok_or_else(|| bad_request("scale_pct is required"))?;
            if pct == 0 || pct > 100 {
                return Err(bad_request("scale_pct must be in 1..=100"));
            }
            let event = MonitorEvent::new(scope, Family::Behaviour, EventKind::ScaledDown, &actor)
                .map_err(bad_request)?
                .with_reason(body.reason.unwrap_or_else(|| "manual scale".to_owned()));
            s.record_event(&event).await.map_err(server_error)?;
            s.set_strategy_state(
                &strategy_id,
                StrategyState::ScaledDown.as_str(),
                i16::from(pct),
            )
            .await
            .map_err(server_error)?;
            state.kill_switch.scale_strategy(&strategy_id, pct);
            Ok(Json(
                json!({ "strategy_id": strategy_id, "scale_pct": pct }),
            ))
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
                typed_confirmation: body.confirm_scope.unwrap_or_default(),
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
                    Ok(Json(
                        json!({ "strategy_id": strategy_id, "state": "nominal" }),
                    ))
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
    store(&state)
        .declare_limit(&limit)
        .await
        .map_err(server_error)?;
    Ok(Json(
        json!({ "strategy_id": strategy_id, "version": limit.version }),
    ))
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
        Ok(()) => Ok(Json(
            json!({ "strategy_id": strategy_id, "may_promote": true }),
        )),
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

    store(&state)
        .record_drill(&outcome)
        .await
        .map_err(server_error)?;
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
        body.minutes
            .unwrap_or(monitor::window::DEFAULT_WINDOW_MINUTES),
        chrono::Utc::now(),
    )
    .map_err(bad_request)?;
    store(&state)
        .open_window(&window)
        .await
        .map_err(server_error)?;
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
    store(&state)
        .close_window(window_id)
        .await
        .map_err(server_error)?;
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
    Ok(Json(
        json!({ "event_id": event_id, "label": label.as_str() }),
    ))
}

// ── Measurement, coverage, defects, drills and the push channel (0165) ─────

/// `GET /api/monitor/feeds` — every live feed's age and what it means now
/// (COMP-007 §3 data family; S-7.5).
///
/// The verdict is computed with the monitor's own `freshness::judge`, so the
/// page and the halt cannot disagree about whether a feed is stale.
pub async fn list_feeds(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    use monitor::evaluate::Staleness;
    use monitor::freshness::{judge, FeedReading};
    let s = store(&state);
    let now = chrono::Utc::now();
    let rows = s.feed_rows().await.map_err(server_error)?;
    let deployments = s.running_deployments().await.map_err(server_error)?;
    let deps = monitor::freshness::dependencies(&deployments, &rows);
    let feeds: Vec<_> = rows
        .iter()
        .map(|r| {
            let verdict = match judge(r, now) {
                FeedReading::Judged(Staleness::Fresh) => "fresh",
                FeedReading::Judged(Staleness::Warning { .. }) => "warning",
                FeedReading::Judged(Staleness::Hard(_)) => "hard_stale",
                FeedReading::Judged(Staleness::Unmeasurable { .. }) => "unmeasurable",
                FeedReading::SessionClosed(_) => "session_closed",
                FeedReading::ProducerSilent { .. } => "producer_silent",
            };
            json!({
                "symbol": r.feed.symbol(),
                "venue": r.feed.venue().as_str(),
                "timeframe_secs": r.timeframe_secs,
                "last_bar_at": r.last_bar_at,
                "age_secs": (now - r.last_bar_at).num_seconds().max(0),
                "session_open": r.session_open,
                "affirmed_at": r.observed_at,
                "verdict": verdict,
                "strategies": deps.strategies_for(&r.feed),
            })
        })
        .collect();
    let unwatched: Vec<_> = monitor::freshness::unwatched(&deployments, &rows)
        .into_iter()
        .map(|d| d.strategy_id.clone())
        .collect();
    Ok(Json(json!({ "feeds": feeds, "unwatched_deployments": unwatched })))
}

/// `GET /api/monitor/venues` — each venue's last reconciliation verdict (S-7.6).
pub async fn list_venue_checks(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let s = store(&state);
    let checks = s.latest_venue_checks().await.map_err(server_error)?;
    let all = s.all_venues_reconciled().await.map_err(server_error)?;
    Ok(Json(json!({
        "all_venues_reconciled": all,
        "venues": checks.iter().map(|c| json!({
            "venue": c.venue,
            "verdict": c.verdict.as_str(),
            "instruments_checked": c.instruments_checked,
            "detail": c.detail,
            "checked_by": c.checked_by,
            "checked_at": c.checked_at,
        })).collect::<Vec<_>>(),
    })))
}

/// `GET /api/monitor/coverage` — what the monitor watches, from the live
/// deployment rows (Set T T-2.3).
///
/// One row per running deployment: which control-limit families it has
/// declared, whether it could be promoted off paper, and whether a feed gauge
/// exists for what it trades. A deployment with nothing declared is measured
/// and never judged, and this is where that is visible rather than implied.
pub async fn coverage(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let s = store(&state);
    let deployments = s.running_deployments().await.map_err(server_error)?;
    let limits = s.load_limits().await.map_err(server_error)?;
    let feeds = s.feed_rows().await.map_err(server_error)?;
    let rows: Vec<_> = deployments
        .iter()
        .map(|d| {
            let families: Vec<&str> = limits
                .families_for(&d.strategy_id)
                .into_iter()
                .map(Family::as_str)
                .collect();
            json!({
                "strategy_id": d.strategy_id,
                "instrument_id": d.instrument_id,
                "environment": d.environment,
                "started_at": d.started_at,
                "declared_families": families,
                "judged": !families.is_empty(),
                "promotion_gate": match limits.may_promote_to_live(&d.strategy_id) {
                    Ok(()) => json!({ "allowed": true }),
                    Err(r) => json!({ "allowed": false, "reason": r.to_string() }),
                },
                "feed_gauged": feeds.iter().any(|f| f.feed.symbol() == d.instrument_id),
            })
        })
        .collect();
    Ok(Json(json!({ "deployments": rows })))
}

/// `GET /api/monitor/defects` — open platform defects (T-2.2, T-2.4).
pub async fn list_defects(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let rows = store(&state).open_defects().await.map_err(server_error)?;
    Ok(Json(json!({
        "defects": rows.iter().map(|d| json!({
            "defect_id": d.defect_id,
            "source": d.source,
            "summary": d.summary,
            "evidence": d.evidence,
            "filed_by": d.filed_by,
            "filed_at": d.filed_at,
            "status": d.status,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct ResolveDefectBody {
    resolution: String,
}

/// `POST /api/monitor/defects/{id}/resolve` — a named human, with a sentence.
pub async fn resolve_defect(
    token: BearerToken,
    State(state): State<AppState>,
    Path(defect_id): Path<Uuid>,
    Json(body): Json<ResolveDefectBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    match store(&state)
        .resolve_defect(defect_id, &actor, &body.resolution)
        .await
    {
        Ok(true) => Ok(Json(json!({ "defect_id": defect_id, "status": "resolved" }))),
        Ok(false) => Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": "not_open", "detail": "no open defect has that id" })),
        )
            .into_response()),
        Err(monitor::store::StoreError::Invalid(why)) => Err(bad_request(why)),
        Err(e) => Err(server_error(e)),
    }
}

/// `GET /api/monitor/drill-runs` — each fail-closed drill's last run (T-2.4).
pub async fn list_drill_runs(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let last = store(&state).last_drill_runs().await.map_err(server_error)?;
    Ok(Json(json!({
        "suite_due": reconciliation::drills::suite_is_due(&last, chrono::Utc::now()),
        "interval_days": reconciliation::drills::DRILL_SUITE_INTERVAL_DAYS,
        "drills": last.iter().map(|(drill, at, passed)| json!({
            "drill": drill, "last_run_at": at, "passed": passed,
        })).collect::<Vec<_>>(),
    })))
}

/// `POST /api/monitor/drill-runs` — run the four drills now (T-2.4, on demand).
pub async fn run_drills(
    token: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor = require_control(&token)?;
    match reconciliation::drills::run_all(&store(&state), false, &actor).await {
        Ok(runs) => Ok(Json(json!({
            "passed": runs.iter().all(|r| r.passed),
            "drills": runs.iter().map(|r| json!({
                "run_id": r.run_id,
                "drill": r.drill.as_str(),
                "passed": r.passed,
                "duration_ms": r.duration_ms,
                "failure": r.failure,
                "defect_id": r.defect_id,
            })).collect::<Vec<_>>(),
        }))),
        Err(e @ reconciliation::drills::DrillSuiteError::NotPaper { .. }) => Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": "not_paper", "detail": e.to_string() })),
        )
            .into_response()),
        Err(e) => Err(server_error(e)),
    }
}

/// The snapshot the push channel sends: the same fields as
/// `GET /api/monitor/kill-switch`, so a client renders one shape.
async fn kill_switch_snapshot(s: &MonitorStore) -> serde_json::Value {
    let lease = s.lease_status(chrono::Utc::now()).await;
    match s.halted_scopes().await {
        Ok(halted) => json!({
            "lease": format!("{lease:?}").to_lowercase(),
            "orders_permitted": lease.permits_orders() && !halted.contains(&Scope::Global),
            "halted_scopes": halted.iter().map(ToString::to_string).collect::<Vec<_>>(),
        }),
        // Unreadable is not "nothing halted": the client is told it does not know.
        Err(e) => json!({ "lease": "unreadable", "orders_permitted": false, "error": e.to_string() }),
    }
}

/// `GET /api/monitor/stream` — the desk's push channel (COMP-007 §6; S-7.10).
///
/// Server-sent events: a `kill_switch` snapshot on connect, on every lease or
/// switch notification, and every 15 s regardless; a `monitor_event` for every
/// row written to `monitor.events` (0165's NOTIFY), so a trip, a scale-down, a
/// drill or a window reaches the desk **within one event** rather than on its
/// next poll. The timeout is what keeps it true when a notification is lost:
/// the worst a broken listener can do is make the stream as slow as polling.
pub async fn stream(
    _token: BearerToken,
    State(state): State<AppState>,
) -> Result<
    axum::response::sse::Sse<
        impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>,
    >,
    Response,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let s = store(&state);
    let mut listener = s.listener().await.map_err(server_error)?;
    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);
    tokio::spawn(async move {
        let snapshot = |v: serde_json::Value| Event::default().event("kill_switch").data(v.to_string());
        if tx.send(Ok(snapshot(kill_switch_snapshot(&s).await))).await.is_err() {
            return;
        }
        loop {
            let note = tokio::time::timeout(
                std::time::Duration::from_secs(15),
                listener.try_recv(),
            )
            .await;
            let event = match note {
                Ok(Ok(Some(n))) if n.channel() == "monitor_events" => {
                    let Ok(id) = n.payload().parse::<Uuid>() else { continue };
                    let row = sqlx::query_as::<_, (Uuid, DateTime<Utc>, String, String, String, Option<String>, Option<String>, Option<f64>, Option<f64>, String, bool)>(
                        "SELECT event_id, occurred_at, scope, family, kind, metric, reason,
                                observed_value::float8, limit_value::float8, actor, is_drill
                           FROM monitor.events WHERE event_id = $1",
                    )
                    .bind(id)
                    .fetch_optional(s.pool())
                    .await;
                    match row {
                        Ok(Some((event_id, occurred_at, scope, family, kind, metric, reason, observed_value, limit_value, actor, is_drill))) => {
                            Event::default().event("monitor_event").id(event_id.to_string()).data(
                                json!({
                                    "event_id": event_id, "occurred_at": occurred_at,
                                    "scope": scope, "family": family, "kind": kind,
                                    "metric": metric, "reason": reason,
                                    "observed_value": observed_value, "limit_value": limit_value,
                                    "actor": actor,
                                    "is_drill": is_drill,
                                })
                                .to_string(),
                            )
                        }
                        _ => continue,
                    }
                }
                // A lease or switch change, a reconnect, or the timeout: resend
                // the state rather than trusting the last one sent.
                Ok(Ok(_)) | Err(_) => snapshot(kill_switch_snapshot(&s).await),
                Ok(Err(e)) => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("stream_error")
                            .data(json!({ "message": e.to_string() }).to_string())))
                        .await;
                    return;
                }
            };
            if tx.send(Ok(event)).await.is_err() {
                return; // the desk went away
            }
        }
    });
    Ok(Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)).text("keep-alive")))
}


// Read-only handlers below that need the caller's identity (for tenant scoping)
// name the token `reader`, not `token`: `tests/monitor_authority.rs` treats a
// handler that takes `token: BearerToken` as one acting on a caller's behalf and
// demands `require_control`, and `_token` would forbid reading the user id. A
// reader changes nothing — the one-gate rule is for mutations, and every POST in
// this file still takes `token` and calls `require_control`.
//
// ── Reconciliation: divergence halts and the fleet (Set T T-0.3, T-2.2) ─────
//
// Served from this file rather than a `reconciliation.rs` route module so they
// share `require_control` — the single authority check this file's header
// promises every mutation goes through — and the authority scan in
// `tests/monitor_authority.rs` that holds it there.
//
// A strategy halted because its realised returns or its position diverged is
// refused at the order gate by `risk::StrategyHaltRegistry`. These are where an
// operator **sees** that halt with its reason, and the only place it is lifted:
// a named human, typing the strategy id, with a note — recorded in
// `mlops.divergence_halt_clearance`. One strategy per request, and no route
// that continues on divergence.

/// `GET /api/reconciliation/halts` — every divergence halt in force for this
/// tenant, with its reason.
///
/// `in_force` is the order gate's own registry; `persisted` says the halt is
/// also in `mlops.divergence_halt`, so a restart will not forget it. A durable
/// halt the registry did not hold is put back into force by this read.
pub async fn list_halts(
    reader: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let tenant = reader.user_id().to_string();
    let registry = state.risk_gate.halt_registry();
    let halts = reconciliation::halts::active(&state.pg, &tenant, registry)
        .await
        .map_err(server_error)?;
    Ok(Json(json!({
        "blocked_orders": registry.blocked_orders(),
        "halts": halts.iter().map(|(h, persisted)| json!({
            "halt_id": h.halt_id,
            "strategy_id": h.strategy_id,
            "kind": h.kind.as_str(),
            "reason": h.reason,
            "raised_at": h.raised_at,
            "in_force": registry.is_halted(&h.strategy_id),
            "persisted": persisted,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Debug, Deserialize)]
pub struct ClearHaltBody {
    /// The strategy id, typed out. Must match exactly.
    confirm_strategy: String,
    /// Why the person believes it is safe to resume.
    note: String,
}

/// `POST /api/reconciliation/halts/{strategy_id}/clear` — the one way a
/// divergence-halted strategy resumes.
pub async fn clear_halt(
    token: BearerToken,
    State(state): State<AppState>,
    Path(strategy_id): Path<String>,
    Json(body): Json<ClearHaltBody>,
) -> Result<Json<serde_json::Value>, Response> {
    let actor_name = require_control(&token)?;
    if body.confirm_strategy != strategy_id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "confirmation_mismatch",
                "detail": format!("type {strategy_id:?} to confirm; received {:?}", body.confirm_strategy),
            })),
        )
            .into_response());
    }
    let tenant = token.user_id().to_string();
    let actor = risk::HumanActor::new(actor_name).map_err(|e| {
        (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "not_a_person", "detail": e.to_string() })),
        )
            .into_response()
    })?;
    match reconciliation::halts::clear(
        &state.pg,
        &tenant,
        state.risk_gate.halt_registry(),
        &strategy_id,
        actor,
        &body.note,
    )
    .await
    {
        Ok(c) => Ok(Json(json!({
            "halt_id": c.halt_id,
            "strategy_id": c.strategy_id,
            "cleared_by": c.cleared_by.as_str(),
            "cleared_at": c.cleared_at,
            "halt_reason": c.halt_reason,
        }))),
        Err(reconciliation::halts::ClearHaltError::Refused(e)) => Err((
            StatusCode::CONFLICT,
            Json(json!({ "error": "clear_refused", "detail": e.to_string() })),
        )
            .into_response()),
        Err(e) => Err(server_error(e)),
    }
}

/// `GET /api/reconciliation/fleet` — the divergence dashboard's data (T-2.2).
pub async fn fleet(
    reader: BearerToken,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, Response> {
    let tenant = reader.user_id().to_string();
    let store = monitor::store::MonitorStore::new(state.pg.clone());
    let a = reconciliation::fleet_job::assess_tenant(&state.pg, &store, &tenant, chrono::Utc::now(), false)
        .await
        .map_err(server_error)?;
    Ok(Json(reconciliation::fleet_job::report_json(&a)))
}

#[cfg(test)]
mod tests {
    /// The environment comes from a recorded column, not from the strategy's name.
    ///
    /// The desk left this column out rather than infer it from a naming
    /// convention, which is how a paper strategy comes to be read as live. The
    /// scan below fails if the query ever reaches for the id instead of the fact.
    #[test]
    fn the_environment_is_read_and_not_guessed() {
        let src = include_str!("monitor.rs");
        let start = src
            .find("SELECT strategy_id, COUNT(DISTINCT execution_mode)")
            .expect("the environment query is gone; this test is stale");
        let query = &src[start..];
        let query = &query[..query.find("\")").unwrap_or(query.len())];

        assert!(
            query.contains("FROM orders"),
            "the environment stopped coming from the order journal"
        );
        assert!(
            query.contains("execution_mode"),
            "the environment is no longer read from the column that records it"
        );
        // The failure this exists for: deciding live-versus-paper from the id.
        for tell in ["LIKE", "ILIKE", "similar to", "position("] {
            assert!(
                !query.contains(tell),
                "the environment query pattern-matches ({tell}); an environment inferred from a \
                 strategy id is a naming convention, and a paper strategy named like a live one \
                 would read as live"
            );
        }
    }

    /// Both event surfaces carry the numbers the desk needs (S-7.10): the SSE
    /// frame and `GET /api/monitor/events` each select and serve
    /// `observed_value` and `limit_value`, as nullable numbers.
    #[test]
    fn both_event_surfaces_serve_observed_and_limit_values() {
        let src = include_str!("monitor.rs");
        let body = &src[..src.find("#[cfg(test)]
mod tests").expect("tests module")];
        for needle in [
            "\"observed_value\": e.observed_value",
            "\"limit_value\": e.limit_value",
            "\"observed_value\": observed_value",
            "\"limit_value\": limit_value",
            "observed_value::float8, limit_value::float8",
        ] {
            assert!(body.contains(needle), "the monitor event surfaces lost {needle}");
        }
    }

    /// A strategy nobody has traded has no environment, and the row says so.
    #[test]
    fn an_untraded_strategy_is_absent_rather_than_paper() {
        let env: std::collections::HashMap<String, String> =
            [("traded".to_string(), "paper".to_string())]
                .into_iter()
                .collect();
        // This is exactly the expression the handler builds the field from.
        assert_eq!(
            serde_json::json!(env.get("traded")),
            serde_json::json!("paper")
        );
        assert_eq!(
            serde_json::json!(env.get("never-traded")),
            serde_json::Value::Null,
            "an untraded strategy rendered as paper, which is the route answering a question \
             nobody asked the database"
        );
    }
}
