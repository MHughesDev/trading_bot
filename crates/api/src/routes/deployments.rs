//! v3 deployments to paper (Set U U-4.2, U-4.6).
//!
//! `POST /api/deployments` compiles a `tbot.strategy/3` graph, **records** the
//! deployment (migration 0142 — the writer `mlops.paper_deployment` never had)
//! and declares it in the one live registry, where the platform's live-runner
//! supervisor picks it up. `DELETE /api/deployments/{id}` withdraws it — its
//! runner stops at its next decision tick — and closes the record.
//!
//! Paper only, on the platform's own paper venue (ADR-0048). There is no live
//! environment to ask for; the table refuses one.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::json;
use strategy_runtime::live::V3Deployment;
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployBody {
    /// An inline graph…
    #[serde(default)]
    pub graph: Option<domain::strategy_graph::StrategyGraph>,
    /// …or the slug of a stored v3 graph.
    #[serde(default)]
    pub strategy_ref: Option<String>,
    pub instrument_id: String,
    /// Decimal string — what the deployment's realised series is measured from.
    pub starting_capital: String,
}

fn refuse(status: StatusCode, error: &str, message: impl Into<String>) -> axum::response::Response {
    (
        status,
        Json(json!({ "error": error, "message": message.into() })),
    )
        .into_response()
}

/// POST /api/deployments
pub async fn create_deployment(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<DeployBody>,
) -> impl IntoResponse {
    let graph = match (body.graph, &body.strategy_ref) {
        (Some(g), _) => g,
        (None, Some(slug)) => {
            let row: Option<(serde_json::Value,)> = sqlx::query_as(
                "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
            )
            .bind(slug)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();
            match row {
                Some((json,)) if backtest::runner::is_v3(&json) => {
                    match serde_json::from_value(json) {
                        Ok(g) => g,
                        Err(e) => {
                            return refuse(
                                StatusCode::UNPROCESSABLE_ENTITY,
                                "invalid_definition",
                                e.to_string(),
                            )
                        }
                    }
                }
                Some(_) => return refuse(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "not_v3",
                    "this endpoint deploys tbot.strategy/3 graphs; v1 strategies start through \
                         /api/strategies/{id}/start until Phase 8",
                ),
                None => return refuse(StatusCode::NOT_FOUND, "strategy_not_found", slug.clone()),
            }
        }
        (None, None) => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "missing_strategy",
                "provide either `graph` or `strategy_ref`",
            )
        }
    };
    let capital: Decimal = match body.starting_capital.parse() {
        Ok(c) if c > Decimal::ZERO => c,
        _ => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_capital",
                "starting_capital must be a positive decimal",
            )
        }
    };
    let plan = match backtest::runner::compile_for_run(&graph, &backtest::run::ParamMap::new()) {
        Ok(p) => p,
        Err(message) => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_strategy",
                message,
            )
        }
    };
    let model_pins: Vec<serde_json::Value> = plan
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            strategy_engine::plan::NodeKind::Model(c) => {
                Some(json!({ "node": n.id, "model": c.model, "version": c.version }))
            }
            _ => None,
        })
        .collect();
    let deployment_id = Uuid::new_v4();
    let user = token.user_id().to_string();
    let record = storage::postgres::deployments::NewDeployment {
        deployment_id,
        user_id: user.clone(),
        strategy_name: graph.name.clone(),
        revision_hash: plan.revision.clone(),
        definition_json: serde_json::to_value(&graph).unwrap_or_default(),
        instrument_id: body.instrument_id.clone(),
        starting_capital: capital,
        model_pins: json!(model_pins),
        started_by: user.clone(),
        forward_deployment_id: None,
    };
    // The record first: a deployment that runs without one is the defect this
    // route exists to end (FEAT-007 §1 defect 10).
    if let Err(e) = storage::postgres::deployments::start(&state.pg, &record).await {
        return refuse(
            StatusCode::CONFLICT,
            "not_recorded",
            format!("the deployment could not be recorded: {e}"),
        );
    }
    let declared = strategy_runtime::live::global().register_v3(V3Deployment {
        deployment_id,
        user_id: user,
        instrument_id: body.instrument_id.clone(),
        revision: plan.revision.clone(),
        graph,
        starting_capital: capital,
    });
    if let Err(e) = declared {
        // Recorded but not running: close the record rather than leave a row
        // that claims a deployment nothing runs.
        let _ = storage::postgres::deployments::stop(
            &state.pg,
            deployment_id,
            &format!("not started: {e}"),
        )
        .await;
        return refuse(StatusCode::CONFLICT, "already_deployed", e.to_string());
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "deployment_id": deployment_id,
            "revision": plan.revision,
            "environment": "paper",
            "venue": "platform_paper",
            "starting_capital": capital.to_string(),
            "model_pins": model_pins,
        })),
    )
        .into_response()
}

/// DELETE /api/deployments/{id}
pub async fn stop_deployment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match storage::postgres::deployments::get(&state.pg, id).await {
        Ok(Some(row)) if row.user_id == token.user_id().to_string() => {}
        Ok(_) => return refuse(StatusCode::NOT_FOUND, "not_found", id.to_string()),
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()),
    }
    // Withdraw first: the runner checks the registry before every decision, so
    // it stops at its next tick whatever the database does next.
    let withdrawn = strategy_runtime::live::global().deregister_v3(id);
    match storage::postgres::deployments::stop(&state.pg, id, "stopped by user").await {
        Ok(closed) => {
            Json(json!({ "deployment_id": id, "withdrawn": withdrawn, "closed": closed }))
                .into_response()
        }
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()),
    }
}

/// GET /api/deployments
pub async fn list_deployments(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    let registry = strategy_runtime::live::global();
    match storage::postgres::deployments::for_user(&state.pg, &token.user_id().to_string()).await {
        Ok(rows) => Json(json!(rows
            .iter()
            .map(|r| json!({
                "deployment_id": r.deployment_id,
                "strategy_name": r.strategy_name,
                "revision": r.revision_hash,
                "instrument_id": r.instrument_id,
                "environment": r.environment,
                "venue": r.venue,
                "starting_capital": r.starting_capital.to_string(),
                "model_pins": r.model_pins,
                "started_at": r.started_at,
                "stopped_at": r.stopped_at,
                "stop_reason": r.stop_reason,
                "running": registry.is_v3_declared(r.deployment_id),
            }))
            .collect::<Vec<_>>()))
        .into_response(),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()),
    }
}
