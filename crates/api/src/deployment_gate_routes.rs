//! HTTP for the deployment path (Set T T-1.1, T-1.3, T-2.1). Thin: every rule
//! lives in [`crate::deployment_gate`] and `ledger::deployment_path`, and this
//! file only reads the token and renders the answer.
//!
//! | route | who |
//! |---|---|
//! | `POST /api/deployment-proposals` | anyone with a session — asking costs a row |
//! | `GET  /api/deployment-proposals` | the owner |
//! | `POST /api/deployment-proposals/{id}/decision` | a person (`web:full`) |
//! | `GET  /api/deployment-overview` | the owner |
//! | `POST /api/deployments/{id}/revoke` | the owner |
//! | `POST /api/deployment-stop-all` | a person holding `monitor.control` |
//! | `GET/PUT /api/deployment-ceilings` | read: anyone; declare: a person |

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use ledger::deployment_path::ProposalDraft;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::deployment_gate::{self as gate, GateError, ProposerKind};
use crate::{auth::scopes, auth::BearerToken, state::AppState};

pub fn render(e: &GateError) -> Response {
    let (status, code) = e.http();
    if status == 500 {
        tracing::error!(error = %e, "deployment gate failed");
    }
    let mut body = json!({ "error": code, "message": e.to_string() });
    if let GateError::Ceiling(c) = e {
        body["binding"] = json!(c.binding().as_str());
    }
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(body),
    )
        .into_response()
}

/// The revision the strategy's head is at, when it is a stored v3 strategy.
/// `None` for one that is not stored (an inline graph has no head to be stale
/// against).
async fn head_revision(pg: &sqlx::PgPool, strategy_name: &str) -> Option<String> {
    let json: Option<(Value,)> =
        sqlx::query_as("SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1")
            .bind(strategy_name)
            .fetch_optional(pg)
            .await
            .ok()
            .flatten();
    let (json,) = json?;
    if !backtest::runner::is_v3(&json) {
        return None;
    }
    let graph: domain::strategy_graph::StrategyGraph = serde_json::from_value(json).ok()?;
    graph.revision().ok().map(|r| r.as_str().to_string())
}

/// `POST /api/deployment-proposals`
pub async fn create_proposal(
    State(state): State<AppState>,
    token: BearerToken,
    Json(draft): Json<ProposalDraft>,
) -> Response {
    let head = head_revision(&state.pg, &draft.strategy_name).await;
    let kind = ProposerKind::from_scopes(&token.scopes);
    let owner = token.user_id.to_string();
    match gate::create_proposal(&state.pg, &owner, &owner, kind, draft, head.as_deref()).await {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({ "proposal_id": id, "state": "pending", "performed": false })),
        )
            .into_response(),
        Err(e) => render(&e),
    }
}

/// `GET /api/deployment-proposals`
pub async fn list_proposals(State(state): State<AppState>, token: BearerToken) -> Response {
    let rows = sqlx::query(
        "SELECT p.proposal_id, p.strategy_name, p.revision_hash, p.instrument_id, p.profile_id,
                p.gates_passed, p.starting_capital::text AS capital, p.review_period_days,
                p.proposer_kind, p.created_at, d.decision, d.decided_by, d.decided_at, d.envelope,
                (a.deployment_id) AS deployment_id
           FROM deployment_proposal p
           LEFT JOIN deployment_decision d USING (proposal_id)
           LEFT JOIN deployment_authorisation a USING (proposal_id)
          WHERE p.owner_user_id = $1
          ORDER BY p.created_at DESC LIMIT 200",
    )
    .bind(token.user_id.to_string())
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => Json(
            rows.iter()
                .map(|r| {
                    json!({
                        "proposal_id": r.get::<Uuid, _>("proposal_id"),
                        "strategy_name": r.get::<String, _>("strategy_name"),
                        "revision": r.get::<String, _>("revision_hash"),
                        "instrument_id": r.get::<String, _>("instrument_id"),
                        "profile_id": r.get::<String, _>("profile_id"),
                        "gates_passed": r.get::<i32, _>("gates_passed"),
                        "starting_capital": r.get::<String, _>("capital"),
                        "review_period_days": r.get::<i32, _>("review_period_days"),
                        "proposed_by_kind": r.get::<String, _>("proposer_kind"),
                        "created_at": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                        "decision": r.get::<Option<String>, _>("decision").unwrap_or_else(|| "pending".into()),
                        "decided_by": r.get::<Option<String>, _>("decided_by"),
                        "decided_at": r.get::<Option<chrono::DateTime<chrono::Utc>>, _>("decided_at"),
                        "envelope": r.get::<Option<Value>, _>("envelope"),
                        "deployment_id": r.get::<Option<Uuid>, _>("deployment_id"),
                    })
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => render(&GateError::Db(e)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionBody {
    /// `approved` or `rejected`. No default.
    decision: String,
    #[serde(default)]
    note: String,
}

/// `POST /api/deployment-proposals/{id}/decision`
pub async fn decide(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<DecisionBody>,
) -> Response {
    let approve = match body.decision.as_str() {
        "approved" => true,
        "rejected" => false,
        other => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "unknown_decision",
                             "message": format!("`{other}` is not a decision; `approved` or `rejected`") })),
            )
                .into_response()
        }
    };
    // The head is read before the decision so a stale verdict is refused.
    let head = match gate::load_proposal(&state.pg, id).await {
        Ok((_, d)) => head_revision(&state.pg, &d.strategy_name).await,
        Err(e) => return render(&e),
    };
    let actor = format!("human:{}", token.user_id);
    match gate::decide_proposal(
        &state.pg,
        &token.scopes,
        &actor,
        id,
        approve,
        &body.note,
        head.as_deref(),
    )
    .await
    {
        Ok(d) => Json(json!({
            "proposal_id": d.proposal_id,
            "decision": if d.approved { "approved" } else { "rejected" },
            "envelope": d.envelope,
        }))
        .into_response(),
        Err(e) => render(&e),
    }
}

/// `GET /api/deployment-overview`
pub async fn overview(State(state): State<AppState>, token: BearerToken) -> Response {
    match gate::overview(&state.pg, &token.user_id.to_string()).await {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => render(&e),
    }
}

/// `POST /api/deployments/{id}/revoke`
pub async fn revoke(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> Response {
    let owner: Result<Option<(String,)>, _> =
        sqlx::query_as("SELECT user_id FROM strategy_deployment WHERE deployment_id = $1")
            .bind(id)
            .fetch_optional(&state.pg)
            .await;
    match owner {
        Ok(Some((u,))) if u == token.user_id.to_string() => {}
        Ok(_) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
        Err(e) => return render(&GateError::Db(e)),
    }
    let actor = format!("human:{}", token.user_id);
    match gate::revoke_one(
        &state.pg,
        &state.kill_switch,
        id,
        &actor,
        "revoked by its owner",
    )
    .await
    {
        Ok(r) => Json(json!({ "deployment_id": id, "withdrawn": r.withdrawn, "closed": r.closed }))
            .into_response(),
        Err(e) => render(&e),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StopAllBody {
    reason: String,
}

/// `POST /api/deployment-stop-all` — the kill switch's action, then every
/// deployment withdrawn. Needs what tripping the kill switch needs.
pub async fn stop_all(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<StopAllBody>,
) -> Response {
    if !scopes::permits_monitor_control(&token.scopes) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "monitor_control_required",
                         "message": "stopping every deployment trips the kill switch, which a person does" })),
        )
            .into_response();
    }
    if body.reason.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_request", "message": "a stop-all requires a reason" })),
        )
            .into_response();
    }
    let actor = format!("human:{}", token.user_id);
    match gate::revoke_all(&state.pg, &state.kill_switch, &actor, &body.reason).await {
        Ok(ids) => {
            Json(json!({ "scope": "global", "state": "halted", "stopped": ids })).into_response()
        }
        Err(e) => render(&e),
    }
}

/// `GET /api/deployment-ceilings`
pub async fn get_ceilings(State(state): State<AppState>, _token: BearerToken) -> Response {
    match sqlx::query("SELECT name, value::text AS value, declared_by, declared_at FROM current_deployment_ceiling")
        .fetch_all(&state.pg)
        .await
    {
        Ok(rows) => Json(json!({
            "declared": rows.iter().map(|r| json!({
                "name": r.get::<String, _>("name"),
                "value": r.get::<String, _>("value"),
                "declared_by": r.get::<String, _>("declared_by"),
                "declared_at": r.get::<chrono::DateTime<chrono::Utc>, _>("declared_at"),
            })).collect::<Vec<_>>(),
            "undeclared_refuses_every_deployment": true,
        }))
        .into_response(),
        Err(e) => render(&GateError::Db(e)),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CeilingBody {
    name: String,
    value: String,
    #[serde(default)]
    note: String,
}

/// `PUT /api/deployment-ceilings`
pub async fn declare_ceiling(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<CeilingBody>,
) -> Response {
    let actor = format!("human:{}", token.user_id);
    match gate::declare_ceiling(
        &state.pg,
        &token.scopes,
        &actor,
        &body.name,
        &body.value,
        &body.note,
    )
    .await
    {
        Ok(()) => (StatusCode::CREATED, Json(json!({ "declared": body.name }))).into_response(),
        Err(e) => render(&e),
    }
}
