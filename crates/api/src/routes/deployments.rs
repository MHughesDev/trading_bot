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
//!
//! A deployment's target is one instrument, or a selection (Set U U-6.4): the
//! trading strategy then trades whatever the selection picks, through the
//! platform's allocator, which starts and retires one member per instrument.

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
    /// The instrument it trades…
    #[serde(default)]
    pub instrument_id: Option<String>,
    /// …or a target: `{"kind":"instrument",…}` or `{"kind":"selection",…}`.
    #[serde(default)]
    pub target: Option<domain::strategy_graph::deployment::DeploymentTarget>,
    /// Decimal string — what the deployment's realised series is measured from.
    pub starting_capital: String,
    /// Record this deployment as forward evidence for a trial (Set T T-0.4):
    /// its realised series is compared with the backtest's, bar by bar.
    #[serde(default)]
    pub forward: Option<ForwardEvidence>,
}

/// What a deployment that is forward evidence declares before its first fill
/// (`mlops.paper_deployment`, migrations 0105 and 0141): the trial it is
/// compared against, the unit both series are counted in, its divergence
/// thresholds and its kill criteria. Never edited afterwards.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForwardEvidence {
    pub trial_id: Uuid,
    /// `usd`, `usdt`, `usdc`, `eur`, `btc` or `eth`.
    pub numeraire: String,
    pub max_abs_return_divergence: f64,
    pub max_cum_return_divergence: f64,
    /// Decimal string.
    pub max_position_divergence: String,
    pub min_signal_match: f64,
    pub kill_criteria: String,
}

/// Write the deployment's `paper_deployment` row, under the caller's tenant.
/// Its `strategy_id` is the deployment id — what the runner puts on every
/// order, and so the join between an order and its divergence halt.
async fn record_forward(
    pg: &sqlx::PgPool,
    tenant: &str,
    deployment_id: Uuid,
    bar_seconds: u32,
    f: &ForwardEvidence,
) -> Result<Uuid, axum::response::Response> {
    let bad = |m: String| refuse(StatusCode::UNPROCESSABLE_ENTITY, "invalid_forward", m);
    let max_position: Decimal = f
        .max_position_divergence
        .parse()
        .map_err(|_| bad("max_position_divergence must be a decimal".into()))?;
    let thresholds = reconciliation::forward::DivergenceThresholds {
        max_abs_return_divergence: f.max_abs_return_divergence,
        max_cum_return_divergence: f.max_cum_return_divergence,
        max_position_divergence: max_position,
        min_signal_match: f.min_signal_match,
    };
    thresholds.validate().map_err(|e| bad(e.to_string()))?;
    if !["usd", "usdt", "usdc", "eur", "btc", "eth"].contains(&f.numeraire.as_str()) {
        return Err(bad(format!("unknown numeraire '{}'", f.numeraire)));
    }
    if f.kill_criteria.trim().is_empty() {
        return Err(bad("kill criteria are declared before the first fill".into()));
    }
    let mut tx = ledger::pg::tenant_tx(pg, tenant)
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO mlops.paper_deployment (tenant_id, strategy_id, trial_id, venue, numeraire, \
           bar_seconds, max_abs_return_divergence, max_cum_return_divergence, \
           max_position_divergence, min_signal_match, kill_criteria, deployed_by) \
         VALUES ($1, $2, $3, 'platform_paper', $4, $5, $6, $7, $8, $9, $10, $1) \
         RETURNING deployment_id",
    )
    .bind(tenant)
    .bind(deployment_id.to_string())
    .bind(f.trial_id)
    .bind(&f.numeraire)
    .bind(i32::try_from(bar_seconds).unwrap_or(i32::MAX))
    .bind(f.max_abs_return_divergence)
    .bind(f.max_cum_return_divergence)
    .bind(max_position)
    .bind(f.min_signal_match)
    .bind(&f.kill_criteria)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        refuse(
            StatusCode::UNPROCESSABLE_ENTITY,
            "forward_not_recorded",
            format!("the forward-evidence record was refused (is `trial_id` a registered trial?): {e}"),
        )
    })?;
    tx.commit()
        .await
        .map_err(|e| refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    Ok(id)
}

/// A forward record whose deployment was then not recorded is retired, not
/// left claiming evidence nothing will produce. The table is append-only.
async fn retire_forward(pg: &sqlx::PgPool, tenant: &str, id: Uuid, reason: &str) {
    let Ok(mut tx) = ledger::pg::tenant_tx(pg, tenant).await else {
        return;
    };
    let _ = sqlx::query(
        "INSERT INTO mlops.paper_deployment_retirement \
           (deployment_id, tenant_id, retired_by, reason) \
         VALUES ($1, $2, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(tenant)
    .bind(reason)
    .execute(&mut *tx)
    .await;
    let _ = tx.commit().await;
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
    use domain::strategy_graph::deployment::DeploymentTarget;
    let instrument_id = match (&body.target, &body.instrument_id) {
        (Some(DeploymentTarget::Instrument { instrument }), None) | (None, Some(instrument)) => {
            instrument.clone()
        }
        (Some(DeploymentTarget::Selection { .. }), None) => String::new(),
        _ => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "missing_target",
                "give exactly one of `instrument_id` or `target`",
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
    let plan = match backtest::runner::compile_for_run(
        &graph,
        &backtest::run::ParamMap::new(),
        &backtest::models::RegistrySnapshot::load_or_empty(&state.pg).await,
    ) {
        Ok(p) => p,
        Err(message) => {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_strategy",
                message,
            )
        }
    };
    let model_pins = model_pins(&plan);
    let deployment_id = Uuid::new_v4();
    let user = token.user_id().to_string();
    let forward = body.forward;
    if let Some(DeploymentTarget::Selection {
        selection,
        top_n,
        rebalance,
        on_removal,
        weighting,
    }) = body.target
    {
        if forward.is_some() {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_forward",
                "forward evidence is recorded per instrument deployment; a selection's members \
                 trade capital the allocator moves, which no single backtest series describes",
            );
        }
        if graph.kind != domain::strategy_graph::StrategyKind::Trading {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "not_trading",
                "a deployment runs a trading strategy; the selection goes in `target`",
            );
        }
        if top_n == 0 {
            return refuse(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_target",
                "top_n must be at least 1",
            );
        }
        if let domain::strategy_graph::deployment::Rebalance::Schedule { cron } = &rebalance {
            if let Err(e) = strategy_engine::schedule::Cron::parse(cron) {
                return refuse(StatusCode::UNPROCESSABLE_ENTITY, "invalid_schedule", e);
            }
        }
        // Pin the selection: a strategy id resolves to its head revision now,
        // so the deployment cannot drift when the head moves.
        let pinned = match backtest::selection_run::load_selection(&state.pg, &selection).await {
            Ok((_, plan, _)) => plan.revision.clone(),
            Err(e) => {
                return refuse(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_selection",
                    e.to_string(),
                )
            }
        };
        let target = DeploymentTarget::Selection {
            selection: pinned.clone(),
            top_n,
            rebalance: rebalance.clone(),
            on_removal,
            weighting: weighting.clone(),
        };
        let record = storage::postgres::deployments::NewDeployment {
            deployment_id,
            user_id: user.clone(),
            strategy_name: graph.name.clone(),
            revision_hash: plan.revision.clone(),
            definition_json: serde_json::to_value(&graph).unwrap_or_default(),
            instrument_id: format!("selection:{pinned}"),
            starting_capital: capital,
            model_pins: json!(model_pins),
            started_by: user.clone(),
            forward_deployment_id: None,
            target: serde_json::to_value(&target).ok(),
            parent_deployment_id: None,
        };
        if let Err(e) = storage::postgres::deployments::start(&state.pg, &record).await {
            return refuse(
                StatusCode::CONFLICT,
                "not_recorded",
                format!("the deployment could not be recorded: {e}"),
            );
        }
        let declared = strategy_runtime::live::global().register_selection(
            strategy_runtime::live::SelectionBinding {
                deployment_id,
                user_id: user,
                trading: graph,
                trading_revision: plan.revision.clone(),
                selection: pinned.clone(),
                top_n,
                rebalance,
                on_removal,
                weighting,
                capital,
            },
        );
        if let Err(e) = declared {
            let _ = storage::postgres::deployments::stop(
                &state.pg,
                deployment_id,
                &format!("not started: {e}"),
            )
            .await;
            return refuse(StatusCode::CONFLICT, "already_deployed", e.to_string());
        }
        return (
            StatusCode::CREATED,
            Json(json!({
                "deployment_id": deployment_id,
                "revision": plan.revision,
                "target": target,
                "environment": "paper",
                "venue": "platform_paper",
                "starting_capital": capital.to_string(),
                "model_pins": model_pins,
            })),
        )
            .into_response();
    }
    // Forward evidence first (Set T T-0.4), so the deployment record names it.
    let forward_deployment_id = match &forward {
        None => None,
        Some(f) => {
            match record_forward(&state.pg, &user, deployment_id, plan.timeframe.seconds(), f)
                .await
            {
                Ok(id) => Some(id),
                Err(r) => return r,
            }
        }
    };
    let record = storage::postgres::deployments::NewDeployment {
        deployment_id,
        user_id: user.clone(),
        strategy_name: graph.name.clone(),
        revision_hash: plan.revision.clone(),
        definition_json: serde_json::to_value(&graph).unwrap_or_default(),
        instrument_id: instrument_id.clone(),
        starting_capital: capital,
        model_pins: json!(model_pins),
        started_by: user.clone(),
        forward_deployment_id,
        target: None,
        parent_deployment_id: None,
    };
    // The record first: a deployment that runs without one is the defect this
    // route exists to end (FEAT-007 §1 defect 10).
    match record_and_declare(&state.pg, &record, graph).await {
        Ok(()) => {}
        Err(DeclareError::NotRecorded(e)) => {
            if let Some(id) = forward_deployment_id {
                retire_forward(&state.pg, &user, id, "its deployment was not recorded").await;
            }
            return refuse(
                StatusCode::CONFLICT,
                "not_recorded",
                format!("the deployment could not be recorded: {e}"),
            );
        }
        Err(DeclareError::AlreadyDeployed(e)) => {
            return refuse(StatusCode::CONFLICT, "already_deployed", e)
        }
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

/// The model versions a compiled plan pins, as a deployment records them.
#[must_use]
pub fn model_pins(plan: &strategy_engine::Plan) -> Vec<serde_json::Value> {
    plan.nodes
        .iter()
        .filter_map(|n| match &n.kind {
            strategy_engine::plan::NodeKind::Model(c) => {
                Some(json!({ "node": n.id, "model": c.model, "version": c.version }))
            }
            _ => None,
        })
        .collect()
}

/// Why a deployment did not start.
#[derive(Debug)]
pub enum DeclareError {
    /// The record was refused; nothing runs.
    NotRecorded(String),
    /// Recorded, but the registry already runs it; the record was closed.
    AlreadyDeployed(String),
}

/// Record an instrument deployment, then declare it in the one live registry,
/// where the live-runner supervisor picks it up — the one way a v3 strategy
/// starts on paper, from this route and from an armed automation alike. The
/// record comes first: a deployment that runs without one is the defect this
/// path exists to end (FEAT-007 §1 defect 10). Recorded but refused by the
/// registry, the record is closed rather than left claiming a deployment
/// nothing runs.
pub async fn record_and_declare(
    pg: &sqlx::PgPool,
    record: &storage::postgres::deployments::NewDeployment,
    graph: domain::strategy_graph::StrategyGraph,
) -> Result<(), DeclareError> {
    storage::postgres::deployments::start(pg, record)
        .await
        .map_err(|e| DeclareError::NotRecorded(e.to_string()))?;
    let declared = strategy_runtime::live::global().register_v3(V3Deployment {
        deployment_id: record.deployment_id,
        user_id: record.user_id.clone(),
        instrument_id: record.instrument_id.clone(),
        revision: record.revision_hash.clone(),
        graph,
        starting_capital: record.starting_capital,
        sleeve: None,
    });
    if let Err(e) = declared {
        let _ = storage::postgres::deployments::stop(
            pg,
            record.deployment_id,
            &format!("not started: {e}"),
        )
        .await;
        return Err(DeclareError::AlreadyDeployed(e.to_string()));
    }
    Ok(())
}

/// Stop every running deployment `started_by` names — withdrawn from the
/// registry first, so its runner stops at its next tick, then closed. Returns
/// how many stopped.
pub async fn stop_started_by(pg: &sqlx::PgPool, started_by: &str, reason: &str) -> usize {
    let Ok(rows) = storage::postgres::deployments::running(pg).await else {
        return 0;
    };
    let registry = strategy_runtime::live::global();
    let mut stopped = 0;
    for row in rows.into_iter().filter(|r| r.started_by == started_by) {
        registry.deregister_v3(row.deployment_id);
        if storage::postgres::deployments::stop(pg, row.deployment_id, reason)
            .await
            .unwrap_or(false)
        {
            stopped += 1;
        }
    }
    stopped
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
    // A selection-bound deployment's allocator stops its members (U-6.4).
    let registry = strategy_runtime::live::global();
    let withdrawn = registry.deregister_v3(id) | registry.deregister_selection(id);
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
                "target": r.target,
                "parent_deployment_id": r.parent_deployment_id,
                "running": registry.is_v3_declared(r.deployment_id)
                    || registry.is_selection_declared(r.deployment_id),
            }))
            .collect::<Vec<_>>()))
        .into_response(),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()),
    }
}
