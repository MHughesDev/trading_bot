//! The Model Studio's own routes (Set N, Phases 1, 3, 4 and 5).
//!
//! Everything here is a thin edge over `model_registry`: the declaration check
//! (`dataset_declaration`), the family registry (`families`), the usage-rule verdict
//! (`verdict`), the promotion proposals (`promotions`) and the fleet read. The decisions
//! live in those modules; a handler parses, authorises and serialises.
//!
//! # Authority
//!
//! Moving the `production` alias is ring 1. The `promote` and `rollback` handlers in
//! `models.rs` call [`require_human`] first, so an agent session (a service token holding
//! research scopes, never `web:full`) is refused with a pointer to the proposal route, and a
//! proposal can be *made* by anyone who can read the model but only *decided* by a person.
//! The one place that asks "is this a person" is [`require_human`].

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Duration, Utc};
use model_registry::dataset_declaration::{DatasetDeclaration, LoweringContext};
use model_registry::{families, importance, promotions, scoring_v2::ForecastUse, verdict};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    auth::{scopes::WEB_FULL, BearerToken},
    state::AppState,
};

/// A model that has not been retrained in this many days is stale.
const STALE_AFTER_DAYS: i64 = 30;

fn err(code: StatusCode, error: &str, message: impl std::fmt::Display, fix: &str) -> Response {
    (
        code,
        Json(json!({ "error": error, "message": message.to_string(), "fix": fix })),
    )
        .into_response()
}

/// Refuse anything but a person's session.
///
/// # Errors
/// A 403 naming the proposal route, for an agent or service session.
pub fn require_human(token: &BearerToken) -> Result<(), Response> {
    if token.permits(WEB_FULL) {
        return Ok(());
    }
    Err(err(
        StatusCode::FORBIDDEN,
        "not_a_human_session",
        "moving a model alias is decided by a person, from the desk",
        "propose it instead: POST /api/models/{id}/versions/{v}/promotion-proposals. \
         An alias an agent could move is a write the agent adds to its own proposals, \
         not a decision somebody else made.",
    ))
}

async fn owned(state: &AppState, id: &str, token: &BearerToken) -> Result<(), Response> {
    match state.models.get_model(id, token.user_id()).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(err(StatusCode::NOT_FOUND, "not_found", "no such model", "list models first")),
        Err(e) => Err(err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry")),
    }
}

// -- Families ----------------------------------------------------------------

/// `GET /api/models/studio/families` -- the registry the trainer also reads.
pub async fn list_families(_token: BearerToken) -> impl IntoResponse {
    Json(json!({ "families": families::all() }))
}

// -- Declarations ------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ValidateBody {
    pub declaration: Value,
    #[serde(default)]
    pub cutoff: Option<DateTime<Utc>>,
}

/// `POST /api/models/declarations/validate`
///
/// Validates, derives purge and embargo, and says what lowering would do, without writing
/// anything. The embargo in the answer is the only embargo there is: the declaration has no
/// field for one, and a body that sends one is refused by `deny_unknown_fields`.
pub async fn validate_declaration(
    _token: BearerToken,
    Json(body): Json<ValidateBody>,
) -> Response {
    let decl: DatasetDeclaration = match serde_json::from_value(body.declaration) {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "ok": false, "errors": [{ "field": "declaration", "message": e.to_string() }] })),
            )
                .into_response()
        }
    };
    let cutoff = body.cutoff.unwrap_or_else(Utc::now);
    let validated = match decl.validate(Some(cutoff)) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "ok": false, "errors": [{ "field": e.field, "message": e.reason }] })),
            )
                .into_response()
        }
    };
    let ctx = LoweringContext {
        universe_spec_id: "validate".into(),
        output_prefix: "validate".into(),
    };
    let lowering = match validated.lower_for_trainer(&ctx) {
        Ok(l) => json!({ "ok": true, "unsupported": [], "trainer_fields": l.trainer_fields }),
        Err(un) => json!({
            "ok": false,
            "unsupported": un.iter().map(|u| json!({ "field": u.field, "why": u.why })).collect::<Vec<_>>(),
            "trainer_fields": [],
        }),
    };
    let d = &validated.derived;
    Json(json!({
        "ok": true,
        "declaration_hash": validated.declaration_hash,
        "derived": {
            "purge_bars": d.purge_bars,
            "embargo_bars": d.embargo_bars,
            "horizon_bars": d.horizon_bars,
            "lookback_bars": d.max_lookback_bars,
            "sample_estimate": d.expected_bars,
            "floor": d.floor,
        },
        "clip": validated.clipped_to_cutoff.then(|| json!({
            "requested_end": decl.window.end.to_rfc3339(),
            "clipped_end": validated.declaration.window.end.to_rfc3339(),
        })),
        "lowering": lowering,
    }))
    .into_response()
}

// -- Fleet -------------------------------------------------------------------

/// `GET /api/models/studio/fleet` -- what is training, stale and serving.
pub async fn fleet(State(state): State<AppState>, token: BearerToken) -> Response {
    let user = token.user_id();
    let rows: Result<Vec<FleetRow>, _> = sqlx::query_as(
        "SELECT m.model_id, m.display_name, m.model_kind, m.status, \
                lv.family, lv.version AS latest_version, \
                pa.version AS production_version, \
                (SELECT max(r.finished_at) FROM training_runs r \
                  WHERE r.model_id = m.model_id AND r.status = 'succeeded') AS last_trained_at, \
                (SELECT bool_or(v.requires_retrain) FROM model_versions v WHERE v.model_id = m.model_id) AS requires_retrain, \
                ar.run_id::text AS run_id, ar.status AS run_status, ar.progress AS run_progress, ar.phase AS run_phase, \
                'train' AS run_kind \
           FROM ai_models m \
           LEFT JOIN LATERAL (SELECT v.family, v.version FROM model_versions v \
                               WHERE v.model_id = m.model_id ORDER BY v.version DESC LIMIT 1) lv ON TRUE \
           LEFT JOIN model_aliases pa ON pa.model_id = m.model_id AND pa.alias = 'production' \
           LEFT JOIN LATERAL (SELECT r.run_id, r.status, r.progress, r.phase FROM training_runs r \
                               WHERE r.model_id = m.model_id AND r.status IN ('queued', 'running') \
                               ORDER BY r.created_at DESC LIMIT 1) ar ON TRUE \
          WHERE m.created_by = $1 AND m.status <> 'archived' \
          ORDER BY m.created_at DESC LIMIT 500",
    )
    .bind(user)
    .fetch_all(&state.pg)
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry"),
    };

    let now = Utc::now();
    let mut models = Vec::with_capacity(rows.len());
    let (mut training, mut evaluating, mut stale, mut serving) = (0, 0, 0, 0);
    for r in rows {
        let (is_stale, why) = if r.requires_retrain.unwrap_or(false) {
            (true, Some("a version is marked requires_retrain".to_string()))
        } else {
            match r.last_trained_at {
                Some(t) if now - t > Duration::days(STALE_AFTER_DAYS) => (
                    true,
                    Some(format!("last trained {} days ago", (now - t).num_days())),
                ),
                None if r.latest_version.is_some() => (true, Some("no successful training run is recorded".into())),
                _ => (false, None),
            }
        };
        let is_serving = r.production_version.is_some();
        if r.run_id.is_some() {
            training += 1;
        }
        if r.status == "evaluating" {
            evaluating += 1;
        }
        if is_stale {
            stale += 1;
        }
        if is_serving {
            serving += 1;
        }
        models.push(json!({
            "model_id": r.model_id,
            "display_name": r.display_name,
            "model_kind": r.model_kind,
            "status": r.status,
            "family": r.family,
            "production_version": r.production_version,
            "latest_version": r.latest_version,
            "active_run": r.run_id.as_ref().map(|id| json!({
                "run_id": id, "kind": r.run_kind, "status": r.run_status,
                "progress": r.run_progress, "phase": r.run_phase,
            })),
            "last_trained_at": r.last_trained_at.map(|t| t.to_rfc3339()),
            "stale": is_stale,
            "stale_reason": why,
            "serving": is_serving,
        }));
    }
    Json(json!({
        "generated_at": now.to_rfc3339(),
        "counts": {
            "models": models.len(), "training": training, "evaluating": evaluating,
            "stale": stale, "serving": serving,
        },
        "models": models,
    }))
    .into_response()
}

#[derive(sqlx::FromRow)]
struct FleetRow {
    model_id: String,
    display_name: String,
    model_kind: String,
    status: String,
    family: Option<String>,
    latest_version: Option<i32>,
    production_version: Option<i32>,
    last_trained_at: Option<DateTime<Utc>>,
    requires_retrain: Option<bool>,
    run_id: Option<String>,
    run_status: Option<String>,
    run_progress: Option<f32>,
    run_phase: Option<String>,
    run_kind: String,
}

// -- Verdict and evidence ----------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct VerdictQuery {
    /// `return,vol,tail_risk`; absent means what the model's definition implies.
    #[serde(default)]
    pub uses: Option<String>,
}

async fn verdict_for(
    state: &AppState,
    id: &str,
    version: i32,
    uses: Option<&str>,
) -> Result<verdict::VerdictReport, Response> {
    match uses {
        Some(u) => {
            let parsed: Vec<ForecastUse> = verdict::parse_uses(u)
                .map_err(|m| err(StatusCode::UNPROCESSABLE_ENTITY, "unknown_use", m, "return, vol or tail_risk"))?;
            Ok(state.models.usage_verdict_for(id, version, &parsed).await)
        }
        None => Ok(state.models.usage_verdict(id, version).await),
    }
}

/// `GET /api/models/{id}/versions/{v}/verdict`
pub async fn get_verdict(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
    Query(q): Query<VerdictQuery>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    match verdict_for(&state, &id, version, q.uses.as_deref()).await {
        Ok(v) => {
            let mut j = v.to_json();
            j["model_id"] = json!(id);
            j["version"] = json!(version);
            Json(j).into_response()
        }
        Err(r) => r,
    }
}

/// `GET /api/models/{id}/versions/{v}/promotion-evidence` -- what a decider needs.
pub async fn promotion_evidence(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    let verdict = state.models.usage_verdict(&id, version).await;
    let (checks, gate_passed) = match state.models.gate_checks(&id, version, "paper").await {
        Ok(c) => c,
        Err(e) => return err(StatusCode::UNPROCESSABLE_ENTITY, "no_such_version", e, "check the version"),
    };
    let aliases = state
        .models
        .get_aliases(&id, token.user_id())
        .await
        .unwrap_or_else(|_| json!({}));
    let pending = promotions::pending_for(&state.pg, &id, version)
        .await
        .unwrap_or_default();
    let mut v = verdict.to_json();
    v["model_id"] = json!(id);
    v["version"] = json!(version);
    Json(json!({
        "model_id": id,
        "version": version,
        "environment": "paper",
        "gate_checks": checks,
        "gate_passed": gate_passed,
        "verdict": v,
        "aliases": aliases,
        "pending_proposals": pending,
    }))
    .into_response()
}

// -- Promotion proposals -----------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ProposeBody {
    #[serde(default)]
    pub environment: Option<String>,
    pub rationale: String,
}

/// `POST /api/models/{id}/versions/{v}/promotion-proposals` -- anyone who can see the model.
pub async fn propose_promotion(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
    Json(body): Json<ProposeBody>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    let env = body.environment.as_deref().unwrap_or("paper");
    // The evidence is frozen onto the proposal: what the decider reads is what the
    // proposer saw, not whatever the evaluation says later.
    let verdict = state.models.usage_verdict(&id, version).await.to_json();
    let evidence = json!({ "verdict": verdict, "proposed_by_scope": if token.permits(WEB_FULL) { "human" } else { "service" } });
    match promotions::propose(&state.pg, &id, version, env, &body.rationale, token.user_id(), &evidence).await {
        Ok(pid) => (
            StatusCode::CREATED,
            Json(json!({ "proposal_id": pid.to_string(), "status": "pending" })),
        )
            .into_response(),
        Err(e @ promotions::PromotionError::AlreadyPending(_)) => {
            err(StatusCode::CONFLICT, "already_pending", e, "decide the pending proposal first")
        }
        Err(e @ promotions::PromotionError::NoSuchVersion(_)) => {
            err(StatusCode::NOT_FOUND, "no_such_version", e, "list versions first")
        }
        Err(e @ promotions::PromotionError::NoRationale) => {
            err(StatusCode::UNPROCESSABLE_ENTITY, "no_rationale", e, "say why")
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry"),
    }
}

/// `GET /api/models/{id}/promotion-proposals`
pub async fn list_promotions(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    match promotions::list(&state.pg, &id).await {
        Ok(p) => Json(json!({ "proposals": p })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry"),
    }
}

#[derive(Debug, Deserialize)]
pub struct DecisionBody {
    /// `approved` or `rejected`; no default.
    pub decision: String,
    #[serde(default)]
    pub note: String,
}

/// `POST /api/models/promotion-proposals/{proposal_id}/decision` -- a person decides.
pub async fn decide_promotion(
    State(state): State<AppState>,
    token: BearerToken,
    Path(proposal_id): Path<Uuid>,
    Json(body): Json<DecisionBody>,
) -> Response {
    if let Err(r) = require_human(&token) {
        return r;
    }
    match promotions::decide(&state.models, &state.pg, proposal_id, token.user_id(), &body.decision, &body.note).await {
        Ok(row) => Json(row).into_response(),
        Err(e @ promotions::PromotionError::NotFound) => err(StatusCode::NOT_FOUND, "not_found", e, "list proposals"),
        Err(e @ promotions::PromotionError::AlreadyDecided) => {
            err(StatusCode::CONFLICT, "already_decided", e, "a decided proposal is final")
        }
        Err(e @ promotions::PromotionError::UnknownDecision(_)) => {
            err(StatusCode::UNPROCESSABLE_ENTITY, "unknown_decision", e, "approved or rejected")
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry"),
    }
}

// -- Importance --------------------------------------------------------------

/// `GET /api/models/{id}/versions/{v}/importance`
///
/// The trainer records per-fold, per-cluster loss increases from its own fitted model
/// (`metrics.importance_inputs`); the platform summarises them with
/// `importance::summarize_fold_importances`, so the rule (error across folds, nothing claimed
/// from one fold) lives in one place.
pub async fn get_importance(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    let metrics: Option<Value> = sqlx::query_scalar(
        "SELECT metrics_json FROM evaluation_runs WHERE model_id = $1 AND version = $2 \
         AND status = 'succeeded' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&id)
    .bind(version)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten()
    .flatten();
    let Some(inputs) = metrics.as_ref().and_then(|m| m.get("importance_inputs")) else {
        return Json(json!({ "available": false, "reason": "no successful evaluation recorded importance inputs" }))
            .into_response();
    };
    if let Some(why) = inputs.get("skipped").and_then(Value::as_str) {
        return Json(json!({ "available": false, "reason": why })).into_response();
    }
    let clusters: Vec<Vec<usize>> = serde_json::from_value(inputs["clusters"].clone()).unwrap_or_default();
    let per_fold: Vec<Vec<f64>> = serde_json::from_value(inputs["per_fold"].clone()).unwrap_or_default();
    let names: Vec<String> = serde_json::from_value(inputs["features"].clone()).unwrap_or_default();
    match importance::summarize_fold_importances(&clusters, &per_fold) {
        Ok(rows) => {
            let folds = per_fold.first().map_or(0, Vec::len);
            let out: Vec<Value> = rows
                .iter()
                .map(|c| {
                    json!({
                        "members": c.features.iter().map(|&i| names.get(i).cloned().unwrap_or_else(|| i.to_string())).collect::<Vec<_>>(),
                        "mean": c.mean_increase,
                        "std_err": c.se,
                        "interval": c.se.map(|s| [c.mean_increase - 2.0 * s, c.mean_increase + 2.0 * s]),
                        "distinguishable_from_zero": c.distinguishable_from_zero,
                    })
                })
                .collect();
            Json(json!({ "available": true, "importance": { "folds": folds, "clusters": out } })).into_response()
        }
        Err(e) => Json(json!({ "available": false, "reason": format!("importance inputs are unusable: {e}") })).into_response(),
    }
}

// -- Calibration (N-4.1) -----------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CalibrateBody {
    /// `aci` or `aci_cusum`.
    pub method: String,
    #[serde(default)]
    pub gamma: Option<f64>,
}

/// `POST /api/models/{id}/versions/{v}/calibrate`
///
/// Runs the trainer's adaptive conformal wrapper over the version's realised forecasts and
/// records the result as a **wrapper version** (`family = calibration`) that names what it
/// wraps; the wrapped version is not changed.
pub async fn calibrate(
    State(state): State<AppState>,
    token: BearerToken,
    Path((id, version)): Path<(String, i32)>,
    Json(body): Json<CalibrateBody>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    if !matches!(body.method.as_str(), "aci" | "aci_cusum") {
        return err(StatusCode::UNPROCESSABLE_ENTITY, "unknown_method", body.method, "aci or aci_cusum");
    }
    let rows: Vec<(f64, f64)> = sqlx::query_as(
        "SELECT predicted_q50, realized FROM model_forecasts \
         WHERE model_id = $1 AND version = $2 AND realized IS NOT NULL AND predicted_q50 IS NOT NULL \
         ORDER BY created_at ASC LIMIT 5000",
    )
    .bind(&id)
    .bind(version)
    .fetch_all(&state.pg)
    .await
    .unwrap_or_default();
    if rows.len() < 50 {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "too_few_realised_forecasts",
            format!("{} realised forecasts for this version; calibration needs at least 50", rows.len()),
            "serve and realise more forecasts first",
        );
    }
    let payload = json!({
        "method": body.method,
        "scores": rows.iter().map(|r| r.0).collect::<Vec<_>>(),
        "realized": rows.iter().map(|r| r.1).collect::<Vec<_>>(),
        "alpha": 0.10,
    });
    let mut payload = payload;
    if let (Some(g), Some(o)) = (body.gamma, payload.as_object_mut()) {
        o.insert("gamma".into(), json!(g));
    }
    let sidecar = model_registry::sidecar::SidecarClient::from_env();
    let result = match sidecar.calibrate(&payload).await {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_GATEWAY, "trainer_unavailable", e, "check the trainer sidecar"),
    };
    let (next,): (i64,) = sqlx::query_as("SELECT COALESCE(MAX(version), 0) + 1 FROM model_versions WHERE model_id = $1")
        .bind(&id)
        .fetch_one(&state.pg)
        .await
        .unwrap_or((1,));
    let config = json!({ "wraps": { "model_id": id, "version": version }, "method": body.method, "calibration": result });
    let inserted = sqlx::query(
        "INSERT INTO model_versions (model_id, version, status, config_json, created_by, family, family_variant, notes) \
         VALUES ($1, $2, 'candidate', $3, $4, 'calibration', $5, $6)",
    )
    .bind(&id)
    .bind(next as i32)
    .bind(&config)
    .bind(token.user_id())
    .bind(&body.method)
    .bind(format!("{} calibration of version {version}", body.method))
    .execute(&state.pg)
    .await;
    match inserted {
        Ok(_) => Json(json!({ "wrapper_version": next, "wraps": version, "calibration": result })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry"),
    }
}

// -- Trace rollups (H-4.6) ---------------------------------------------------

/// `GET /api/models/{id}/traces/rollup` -- inference count and latency by outcome.
pub async fn trace_rollup(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = owned(&state, &id, &token).await {
        return r;
    }
    match state.inference.trace_rollup(&id).await {
        Ok(r) => Json(json!({ "window_days": 7, "by_result": r })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, "db_error", e, "retry"),
    }
}

#[cfg(test)]
mod human_gate_tests {
    use super::*;

    fn token(scopes: &[&str]) -> BearerToken {
        BearerToken {
            token: "k".into(),
            user_id: Uuid::nil(),
            scopes: scopes.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// N-4.3: a service (agent) token is refused with a 403 that names the proposal route; a web
    /// session passes.
    #[test]
    fn a_service_token_cannot_decide_a_promotion_and_a_web_session_can() {
        let agent = token(&["data.read", "models.read"]);
        let refused = require_human(&agent).expect_err("an agent session is not a person");
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        assert!(require_human(&token(&["web:full"])).is_ok());
        assert!(require_human(&token(&[])).is_err(), "no scopes is not a person either");
    }

    /// N-4.3: every handler that moves an alias calls `require_human` first. A new alias-moving
    /// route that forgets it fails here.
    #[test]
    fn every_handler_that_moves_an_alias_checks_for_a_person_first() {
        let src = include_str!("models.rs");
        let mut checked = 0;
        for chunk in src.split("pub async fn ").skip(1) {
            let moves_alias = chunk.contains(".promote_gated(")
                || chunk.contains(".models.rollback(")
                || chunk.contains(".set_alias(");
            if moves_alias {
                checked += 1;
                let name = chunk.split('(').next().unwrap_or("?");
                let at_guard = chunk.find("require_human(");
                let at_move = chunk
                    .find(".promote_gated(")
                    .or_else(|| chunk.find(".models.rollback("))
                    .or_else(|| chunk.find(".set_alias("))
                    .unwrap();
                assert!(
                    at_guard.is_some_and(|g| g < at_move),
                    "`{name}` moves an alias without calling require_human first"
                );
            }
        }
        assert!(checked >= 2, "expected promote and rollback to be found, found {checked}");
    }
}
