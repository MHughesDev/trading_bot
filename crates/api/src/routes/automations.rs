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

// Every automation belongs to the user whose session created it, and every
// route below is scoped to the caller (Set E 1.4). There is no shared operator
// identity: `Uuid::nil()` was one, and it let any authenticated caller list,
// arm, disarm or delete every user's automations.

// ── List ──────────────────────────────────────────────────────────────────────

/// GET /api/automations — the caller's automations, paper and live, newest first.
pub async fn list_automations(
    State(state): State<AppState>,
    token: BearerToken,
) -> impl IntoResponse {
    match automation::list_automations_for(&state.pg, token.user_id()).await {
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
    token: BearerToken,
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
        user_id: token.user_id(),
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

/// Arming and disarming.
///
/// **Arming is registration in the one live registry, not a second evaluator**
/// (ADR-0038).  `armed_automations` — the query that was meant to resume armed
/// plans at startup — had no caller anywhere in the repository, so arming set a
/// boolean, returned success, and nothing ever evaluated the spec.  The fix is
/// not to write the missing evaluator: a second evaluation path is how the
/// platform came to have two doors that both reported success and both did
/// nothing.  Arming now declares the automation's execution strategy into
/// [`strategy_runtime::live`], which is what the in-process pipeline reads.
///
/// `pipeline` automations (multi-stage discovery over a universe) are a
/// genuinely different shape and have no executable path yet — the universe
/// feed is Set E 4.3.  Arming one is **refused** with a reason rather than
/// accepted and ignored.
async fn set_armed(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    armed: bool,
) -> axum::response::Response {
    // Ownership first, for both directions: a stranger's id is a 404, exactly
    // like a missing one, and nothing is deregistered on its behalf.
    match automation::get_automation_owned(&state.pg, token.user_id(), id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(id),
        Err(e) => return db_error(e),
    }
    if !armed {
        return match automation::disarm_automation_owned(&state.pg, token.user_id(), id).await {
            Ok(true) => {
                let removed =
                    strategy_runtime::live::global().deregister_automation(&id.to_string());
                let stopped = super::deployments::stop_started_by(
                    &state.pg,
                    &format!("automation:{id}"),
                    "automation disarmed",
                )
                .await;
                Json(json!({ "id": id, "armed": false, "deregistered": removed + stopped }))
                    .into_response()
            }
            Ok(false) => not_found(id),
            Err(e) => db_error(e),
        };
    }
    match arm_by_id(&state, id).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => e.into_response(),
    }
}

/// Why an automation could not be armed.
///
/// A typed refusal rather than a `Response`, so the **one** arming path can be
/// called by the route and by a human's approval of a `deploy_paper` proposal
/// (S-10.9) without either building its own. `set_armed`'s own note is the
/// reason this is not two functions: a second evaluation path is how the
/// platform came to have two doors that both reported success and both did
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmError {
    NotFound,
    /// A multi-stage discovery pipeline needs the shared universe feed
    /// (Set E 4.3). Refused rather than armed and never evaluated.
    NotExecutable,
    /// The automation's spec or its strategy will not load.
    Unusable(String),
    AlreadyRunning(String),
    Storage(String),
}

impl ArmError {
    /// The short reason, for a caller that records rather than renders.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::NotFound => "no such automation".into(),
            Self::NotExecutable => "pipeline automations are not executable yet (Set E 4.3)".into(),
            Self::Unusable(d) => format!("the automation cannot be armed: {d}"),
            Self::AlreadyRunning(d) => format!("already running: {d}"),
            Self::Storage(d) => format!("storage: {d}"),
        }
    }
}

impl IntoResponse for ArmError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "not_found", "detail": self.reason() })),
            )
                .into_response(),
            Self::NotExecutable => (
                StatusCode::NOT_IMPLEMENTED,
                Json(json!({
                    "error": "pipeline automations are not executable yet",
                    "detail": "a multi-stage discovery pipeline needs the shared universe feed \
                               (Set E 4.3).  Refusing to arm rather than reporting success and \
                               evaluating nothing.",
                })),
            )
                .into_response(),
            Self::AlreadyRunning(d) => (
                StatusCode::CONFLICT,
                Json(json!({ "error": "already_running", "detail": d })),
            )
                .into_response(),
            Self::Unusable(d) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "unusable_automation", "detail": d })),
            )
                .into_response(),
            Self::Storage(d) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "storage", "detail": d })),
            )
                .into_response(),
        }
    }
}

/// Arm one automation. **The only arming path.**
///
/// Not owner-scoped itself: the HTTP route checks ownership before it calls
/// this (Set E 1.4), and the proposal-approval caller acts on a proposal's own
/// subject. Use [`arm_owned`] from anything that acts for a caller.
///
/// Arming is registration in the one live registry, not a second evaluator
/// (ADR-0038). `armed_automations` — the query meant to resume armed plans at
/// startup — once had no caller anywhere in the repository, so arming set a
/// boolean, returned success, and nothing ever evaluated the spec.
///
/// # Errors
/// [`ArmError`], which renders itself for a route and reports itself for an
/// approval.
pub async fn arm_by_id(state: &AppState, id: Uuid) -> Result<serde_json::Value, ArmError> {
    let Ok(Some(row)) = automation::get_automation(&state.pg, id).await else {
        return Err(ArmError::NotFound);
    };
    if row.kind.as_str() != "single_instrument" {
        return Err(ArmError::NotExecutable);
    }

    let spec = &row.spec;
    let asset_class = spec
        .get("asset_class")
        .and_then(|v| v.as_str())
        .unwrap_or("crypto_spot_cex");
    let Some(instrument_id) = spec.get("instrument_id").and_then(|v| v.as_str()) else {
        return Err(ArmError::Unusable("spec is missing instrument_id".into()));
    };
    let Some(execution_strategy_id) = spec.get("execution_strategy_id").and_then(|v| v.as_str())
    else {
        return Err(ArmError::Unusable(
            "spec is missing execution_strategy_id".into(),
        ));
    };

    // Load the execution strategy's definition.  An automation that names a
    // strategy the platform does not have must not arm.
    let def_row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1")
            .bind(execution_strategy_id)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();

    let Some((def_json,)) = def_row else {
        return Err(ArmError::Unusable(format!(
            "execution strategy {execution_strategy_id} not found"
        )));
    };

    // A strategy on the one engine arms as a recorded deployment (Set U
    // U-8.3), never on hot-path stage 3.
    if backtest::runner::is_v3(&def_json) {
        return arm_v3(state, &row, id, instrument_id, asset_class, execution_strategy_id, def_json)
            .await;
    }

    let definition: domain::strategy_def::StrategyDefinition = serde_json::from_value(def_json)
        .map_err(|e| ArmError::Unusable(format!("invalid strategy definition: {e}")))?;
    let definition = domain::strategy_def::params::materialize(
        &definition,
        &domain::strategy_def::ParamValues::new(),
    )
    .map_err(|e| ArmError::Unusable(format!("invalid parameters: {e}")))?;

    // Data first, then evaluation: the pipeline that supplies marks and bars
    // must exist before anything is declared live against it.
    state.ensure_pipeline(instrument_id, asset_class);

    let live = strategy_runtime::live::global();
    live.register(strategy_runtime::live::LiveStrategySpec {
        user_id: row.user_id.to_string(),
        instrument_id: instrument_id.to_owned(),
        definition,
        source: strategy_runtime::live::LiveSource::Automation {
            automation_id: id.to_string(),
        },
    })
    .map_err(|e| ArmError::AlreadyRunning(e.to_string()))?;

    match automation::set_automation_armed(&state.pg, id, true).await {
        Ok(true) => Ok(json!({
            "id": id,
            "armed": true,
            "instrument_id": instrument_id,
            "execution_strategy_id": execution_strategy_id,
            "live_generation": live.generation(),
        })),
        // The registry declaration and the durable flag are two writes, and a
        // failure after the first leaves the platform evaluating something it
        // does not believe is armed. Deregistered before returning, so the two
        // cannot disagree.
        Ok(false) => {
            live.deregister_automation(&id.to_string());
            Err(ArmError::NotFound)
        }
        Err(e) => {
            live.deregister_automation(&id.to_string());
            Err(ArmError::Storage(e.to_string()))
        }
    }
}

/// Arm an automation whose execution strategy is a `tbot.strategy/3` graph:
/// record and declare it the way `POST /api/deployments` does
/// ([`super::deployments::record_and_declare`]), `started_by` naming the
/// automation so disarming or deleting it stops exactly this deployment. Its
/// realised series needs a base, so the spec names a `starting_capital`.
async fn arm_v3(
    state: &AppState,
    row: &automation::AutomationRow,
    id: Uuid,
    instrument_id: &str,
    asset_class: &str,
    execution_strategy_id: &str,
    def_json: serde_json::Value,
) -> Result<serde_json::Value, ArmError> {
    let capital: rust_decimal::Decimal = match row.spec.get("starting_capital") {
        Some(serde_json::Value::String(s)) => s.parse().ok(),
        Some(serde_json::Value::Number(n)) => n.to_string().parse().ok(),
        _ => None,
    }
    .filter(|c: &rust_decimal::Decimal| c.is_sign_positive() && !c.is_zero())
    .ok_or_else(|| {
        ArmError::Unusable(
            "a strategy on the one engine runs as a recorded deployment, which is measured from \
             a starting capital: set a positive `starting_capital` in the automation's spec"
                .into(),
        )
    })?;
    let graph: domain::strategy_graph::StrategyGraph = serde_json::from_value(def_json)
        .map_err(|e| ArmError::Unusable(format!("invalid strategy graph: {e}")))?;
    if graph.kind != domain::strategy_graph::StrategyKind::Trading {
        return Err(ArmError::Unusable(
            "an automation trades one instrument; a selection strategy deploys on a selection target"
                .into(),
        ));
    }
    let plan = backtest::runner::compile_for_run(
        &graph,
        &backtest::run::ParamMap::new(),
        &backtest::models::RegistrySnapshot::load_or_empty(&state.pg).await,
    )
    .map_err(|e| ArmError::Unusable(format!("the strategy does not compile: {e}")))?;
    let started_by = format!("automation:{id}");
    let running = storage::postgres::deployments::running(&state.pg)
        .await
        .map_err(|e| ArmError::Storage(e.to_string()))?;
    if running.iter().any(|r| r.started_by == started_by) {
        return Err(ArmError::AlreadyRunning(format!(
            "automation {id} already runs as a deployment"
        )));
    }
    let deployment_id = Uuid::new_v4();
    let record = storage::postgres::deployments::NewDeployment {
        deployment_id,
        user_id: row.user_id.to_string(),
        strategy_name: graph.name.clone(),
        revision_hash: plan.revision.clone(),
        definition_json: serde_json::to_value(&graph).unwrap_or_default(),
        instrument_id: instrument_id.to_owned(),
        starting_capital: capital,
        model_pins: json!(super::deployments::model_pins(&plan)),
        started_by: started_by.clone(),
        forward_deployment_id: None,
        target: None,
        parent_deployment_id: None,
    };
    state.ensure_pipeline(instrument_id, asset_class);
    super::deployments::record_and_declare(&state.pg, &record, graph)
        .await
        .map_err(|e| match e {
            super::deployments::DeclareError::NotRecorded(m) => ArmError::Storage(m),
            super::deployments::DeclareError::AlreadyDeployed(m) => ArmError::AlreadyRunning(m),
        })?;
    match automation::set_automation_armed(&state.pg, id, true).await {
        Ok(true) => Ok(json!({
            "id": id,
            "armed": true,
            "instrument_id": instrument_id,
            "execution_strategy_id": execution_strategy_id,
            "deployment_id": deployment_id,
            "revision": plan.revision,
        })),
        // Running but not believed armed: stopped, so the two cannot disagree.
        Ok(false) => {
            super::deployments::stop_started_by(&state.pg, &started_by, "automation vanished while arming").await;
            Err(ArmError::NotFound)
        }
        Err(e) => {
            super::deployments::stop_started_by(&state.pg, &started_by, "automation not marked armed").await;
            Err(ArmError::Storage(e.to_string()))
        }
    }
}

/// Re-declare every armed `single_instrument` automation into the live
/// registry, and start the data pipeline each one needs.
///
/// Called once at startup.  `armed` is durable, so an automation armed before a
/// restart must evaluate after one; before ADR-0038 the resume query existed
/// and had no caller, which meant a restart silently stopped every automation
/// while the UI went on showing them as armed.
///
/// Returns how many were resumed.  A `pipeline` automation is skipped with a
/// warning — it has no executable path (Set E 4.3) and arming one is refused,
/// but rows predating that refusal may still be armed in the database.
pub async fn resume_armed(
    pg: &sqlx::PgPool,
    stream_tx: Option<&tokio::sync::mpsc::UnboundedSender<crate::state::StreamRequest>>,
) -> Result<usize, sqlx::Error> {
    let rows = automation::armed_automations(pg).await?;
    let live = strategy_runtime::live::global();
    let mut resumed = 0;

    for row in rows {
        if row.kind.as_str() != "single_instrument" {
            tracing::warn!(
                automation_id = %row.id,
                kind = %row.kind,
                "armed automation of a kind that has no executable path — not resumed"
            );
            continue;
        }
        let Some(instrument_id) = row.spec.get("instrument_id").and_then(|v| v.as_str()) else {
            tracing::warn!(automation_id = %row.id, "armed automation has no instrument_id");
            continue;
        };
        let Some(strategy_id) = row
            .spec
            .get("execution_strategy_id")
            .and_then(|v| v.as_str())
        else {
            tracing::warn!(
                automation_id = %row.id,
                "armed automation has no execution_strategy_id"
            );
            continue;
        };
        let asset_class = row
            .spec
            .get("asset_class")
            .and_then(|v| v.as_str())
            .unwrap_or("crypto_spot_cex");

        let def_row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT definition_json FROM strategy_definitions WHERE strategy_id = $1",
        )
        .bind(strategy_id)
        .fetch_optional(pg)
        .await?;

        let Some((def_json,)) = def_row else {
            tracing::error!(
                automation_id = %row.id,
                strategy_id,
                "armed automation names a strategy that no longer exists — not resumed"
            );
            continue;
        };
        // A v3 automation runs as a recorded deployment, which the live-runner
        // supervisor resumes from its record; declaring it here too would run
        // it twice.
        if backtest::runner::is_v3(&def_json) {
            tracing::info!(automation_id = %row.id, "v3 automation resumes as its recorded deployment");
            continue;
        }
        let definition: domain::strategy_def::StrategyDefinition =
            match serde_json::from_value(def_json)
                .map_err(|e| e.to_string())
                .and_then(|d: domain::strategy_def::StrategyDefinition| {
                    domain::strategy_def::params::materialize(
                        &d,
                        &domain::strategy_def::ParamValues::new(),
                    )
                    .map_err(|e| e.to_string())
                }) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(
                        automation_id = %row.id,
                        strategy_id,
                        error = %e,
                        "armed automation's strategy no longer materializes — not resumed"
                    );
                    continue;
                }
            };

        if let Some(tx) = stream_tx {
            let _ = tx.send(crate::state::StreamRequest {
                instrument_id: instrument_id.to_owned(),
                asset_class: asset_class.to_owned(),
            });
        }

        match live.register(strategy_runtime::live::LiveStrategySpec {
            user_id: row.user_id.to_string(),
            instrument_id: instrument_id.to_owned(),
            definition,
            source: strategy_runtime::live::LiveSource::Automation {
                automation_id: row.id.to_string(),
            },
        }) {
            Ok(()) => resumed += 1,
            Err(e) => tracing::warn!(
                automation_id = %row.id,
                error = %e,
                "armed automation was already live — not resumed twice"
            ),
        }
    }
    Ok(resumed)
}

/// DELETE /api/automations/:id — the caller's own automation only.
///
/// An armed automation is deregistered from the live registry as it is
/// deleted. Before, deleting an armed row left its strategy evaluating with no
/// row anyone could see or disarm.
pub async fn delete_automation(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match automation::delete_automation_owned(&state.pg, token.user_id(), id).await {
        Ok(true) => {
            let deregistered =
                strategy_runtime::live::global().deregister_automation(&id.to_string());
            let stopped = super::deployments::stop_started_by(
                &state.pg,
                &format!("automation:{id}"),
                "automation deleted",
            )
            .await;
            Json(json!({ "id": id, "deleted": true, "deregistered": deregistered + stopped }))
                .into_response()
        }
        Ok(false) => not_found(id),
        Err(e) => db_error(e),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn bad_request(message: &str) -> axum::response::Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message }))).into_response()
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

/// [`arm_by_id`] for one user's own automation: `NotFound` when `id` is missing
/// **or** belongs to someone else, so a caller cannot arm (and so cannot learn
/// the existence of) another user's automation.
///
/// # Errors
/// [`ArmError`].
pub async fn arm_owned(
    state: &AppState,
    owner: Uuid,
    id: Uuid,
) -> Result<serde_json::Value, ArmError> {
    match automation::get_automation_owned(&state.pg, owner, id).await {
        Ok(Some(_)) => arm_by_id(state, id).await,
        Ok(None) => Err(ArmError::NotFound),
        Err(e) => Err(ArmError::Storage(e.to_string())),
    }
}
