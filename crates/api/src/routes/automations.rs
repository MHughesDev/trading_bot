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
    _token: BearerToken,
    Path(id): Path<Uuid>,
    armed: bool,
) -> axum::response::Response {
    if !armed {
        let removed = strategy_runtime::live::global().deregister_automation(&id.to_string());
        return match automation::set_automation_armed(&state.pg, id, false).await {
            Ok(true) => {
                Json(json!({ "id": id, "armed": false, "deregistered": removed })).into_response()
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
