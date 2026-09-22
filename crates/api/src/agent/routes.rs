//! HTTP surface for agent runs: start, list, detail, transcript, cancel.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use uuid::Uuid;

use crate::credentials::LlmCredentialStore;
use crate::{auth::BearerToken, state::AppState};

use super::conversations;
use super::manager::{PauseOutcome, ResumeOutcome, StartError};

#[derive(Debug, Deserialize)]
pub struct NewConversationBody {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageBody {
    pub text: String,
}

/// The default provider and model for a new conversation.
///
/// The user picks a prompt, not a model. A default that can be overridden per
/// conversation keeps the choice available without putting it in the way.
/// The provider a conversation opens under when nobody names one.
///
/// Public because the session door opens conversations too (S-11.14), and two
/// places deciding this separately is two defaults for one desk.
#[must_use]
pub fn default_provider() -> String {
    std::env::var("TBOT_AGENT_PROVIDER").unwrap_or_else(|_| "anthropic".into())
}

fn default_model(state: &AppState) -> (String, String) {
    let model = std::env::var("TBOT_AGENT_MODEL").unwrap_or_else(|_| state.default_profile.clone());
    (default_provider(), model)
}

fn start_error(e: &StartError) -> (StatusCode, &'static str) {
    match e {
        StartError::InvalidRequest(_) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"),
        StartError::NotConfigured(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "provider_not_configured")
        }
        StartError::Busy => (StatusCode::CONFLICT, "agent_busy"),
        // 503 with a duration, not 409. The desk shows it as a wait rather than
        // failing the click, and the two are different instructions: `agent_busy`
        // means try again when something finishes, this means try again in about a
        // minute (S-2.15).
        StartError::Draining { .. } => (StatusCode::SERVICE_UNAVAILABLE, "platform_draining"),
        StartError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
    }
}

/// GET /api/agent/profiles — what this platform can actually be asked to run.
///
/// The conversation API has always accepted a `provider`/`model` override, but with
/// nothing enumerating the loaded profiles the only way to reach a local tier was to
/// know a model id by heart or to set `TBOT_AGENT_MODEL` and restart. That made the
/// local tier real in the backend and invisible in the product, which is the same as
/// not having it.
///
/// The tier and tool-calling fields travel with each entry because they are what the
/// choice is actually between — a picker that shows only names asks the operator to
/// remember which of them costs money and which of them can hold a chain.
///
/// # Why this probes the backend instead of reading the config
///
/// A profile describes a model; it does not mean this machine has one. The first
/// version grouped the picker on `tier.is_local()`, which put the *deployment target*
/// profile (`qwen3.6-35b-a3b`, sized for a 3090 nobody here owns) under "on this
/// machine" beside a model that genuinely was installed. That is a config file
/// describing itself, and an operator who picked it would have got a model-not-found
/// error from a list that had just told them it was there.
///
/// So local providers are asked what they actually hold: `/api/tags` for what is
/// installed, `/api/ps` for what is resident right now. Those are different answers to
/// different questions — installed means it will run, resident means it will run
/// *without paying the cold load*, which is ~170 s on the dev box and the difference
/// between a slow answer and an apparent hang.
///
/// A backend that cannot be reached yields `null` rather than `false`, because "I
/// could not ask" and "it is not there" must not render as the same claim.
pub async fn list_profiles(State(state): State<AppState>, token: BearerToken) -> Response {
    let (default_provider, default_model) = default_model(&state);
    let creds = LlmCredentialStore::new(state.pg.clone(), state.cred_crypto.clone());

    // Probe each distinct local provider once, not once per profile.
    let mut local_providers: Vec<String> = state
        .profiles
        .ids()
        .into_iter()
        .filter_map(|id| state.profiles.get(id))
        .filter(|p| p.tier.is_local())
        .map(|p| p.provider.clone())
        .collect();
    local_providers.sort();
    local_providers.dedup();

    let mut inventory: BTreeMap<String, Inventory> = BTreeMap::new();
    for provider in local_providers {
        let inv = probe_local_provider(&creds, token.user_id(), &provider).await;
        inventory.insert(provider, inv);
    }

    let profiled: BTreeSet<String> = state
        .profiles
        .ids()
        .into_iter()
        .filter_map(|id| state.profiles.get(id))
        .map(|p| p.model_id.clone())
        .collect();

    let items: Vec<Value> = state
        .profiles
        .ids()
        .into_iter()
        .filter_map(|id| state.profiles.get(id))
        .map(|p| {
            let inv = inventory.get(&p.provider);
            // `None` at any level means "not asked" or "could not ask", and stays
            // null the whole way to the UI.
            let installed =
                inv.and_then(|i| i.installed.as_ref().map(|m| m.contains(&p.model_id)));
            let resident = inv.and_then(|i| i.resident.as_ref().map(|m| m.contains(&p.model_id)));
            json!({
                "model_id": p.model_id,
                "provider": p.provider,
                "tier": p.tier.as_str(),
                "is_local": p.tier.is_local(),
                "tool_calling": p.tool_calling.as_str(),
                "max_steps": p.orchestration.max_steps,
                // Surfaced so the UI can say *why* a local tier is allowed to hold a
                // chain, rather than presenting it as a bare claim.
                "multi_step_evidence": p.multi_step_evidence.as_ref().map(|e| json!({
                    "eval": e.eval,
                    "device": e.device,
                    "trials": e.trials,
                    "clean": e.clean,
                    "recorded": e.recorded,
                })),
                "requires_api_key": p.provider != "ollama" && p.provider != "vllm",
                "is_default": p.model_id == default_model,
                // Measured, not declared. Null when the backend could not be reached.
                "installed": installed,
                "resident": resident,
                "backend_reachable": inv.map(|i| i.reachable),
            })
        })
        .collect();

    // Models this machine holds that no profile covers. Reported rather than hidden:
    // the platform refuses to run an unprofiled model on purpose (ADR-0031 §1.2), and
    // "you have this but it cannot be used yet" is more useful to an operator than
    // silence, which reads as the model not having been detected at all.
    let unprofiled: Vec<Value> = inventory
        .iter()
        .flat_map(|(provider, inv)| {
            inv.installed
                .iter()
                .flatten()
                .filter(|m| !profiled.contains(*m))
                .map(move |m| {
                    json!({
                        "model_id": m,
                        "provider": provider,
                        "resident": inv.resident.as_ref().map(|r| r.contains(m)),
                    })
                })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "profiles": items,
            "unprofiled_local_models": unprofiled,
            "default_provider": default_provider,
            "default_model": default_model,
        })),
    )
        .into_response()
}

/// What one local backend actually holds.
#[derive(Default)]
struct Inventory {
    reachable: bool,
    /// Installed and runnable. `None` when the backend could not be asked.
    installed: Option<BTreeSet<String>>,
    /// Loaded in memory right now, so no cold load is owed.
    resident: Option<BTreeSet<String>>,
}

/// How long to wait on a local backend before reporting "unknown".
///
/// The shared client carries a 600 s timeout because a local model can take minutes
/// to generate. That is right for inference and catastrophic for a page load: a host
/// that black-holes packets rather than refusing them would hang the picker for ten
/// minutes. Listing what is installed is a cheap call or it does not happen.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

async fn probe_local_provider(
    creds: &LlmCredentialStore,
    user_id: Uuid,
    provider: &str,
) -> Inventory {
    let Ok(parsed) = provider.parse::<llm::Provider>() else {
        return Inventory::default();
    };
    let (api_key, base_url) = match creds.load(user_id, provider).await {
        Ok(Some(c)) => ((!c.api_key.is_empty()).then_some(c.api_key), c.base_url),
        _ => (None, None),
    };
    let client = llm::LlmClient::new(parsed, api_key, base_url);

    let installed: Option<BTreeSet<String>> =
        match tokio::time::timeout(PROBE_TIMEOUT, client.list_models()).await {
            Ok(Ok(models)) => Some(models.into_iter().map(|m| m.id).collect()),
            _ => None,
        };
    if installed.is_none() {
        // Unreachable: report nothing rather than an empty machine, so the UI can
        // say "could not ask" instead of "you have no models".
        return Inventory::default();
    }
    let resident = tokio::time::timeout(PROBE_TIMEOUT, client.resident_models())
        .await
        .ok()
        .map(|list| list.into_iter().collect());
    Inventory {
        reachable: true,
        installed,
        resident,
    }
}

/// POST /api/agent/conversations — the "New chat" button.
pub async fn create_conversation(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<NewConversationBody>,
) -> Response {
    let (dp, dm) = default_model(&state);
    let provider = body.provider.unwrap_or(dp);
    let model = body.model.unwrap_or(dm);
    match conversations::create(&state.pg, token.user_id(), &provider, &model).await {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({ "conversation_id": id, "provider": provider, "model": model })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/agent/conversations — the sidebar.
pub async fn list_conversations(State(state): State<AppState>, token: BearerToken) -> Response {
    match conversations::list(&state.pg, token.user_id(), 200).await {
        Ok(items) => (StatusCode::OK, Json(json!({ "conversations": items }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// GET /api/agent/conversations/{id} — header plus every turn.
pub async fn get_conversation(
    State(state): State<AppState>,
    token: BearerToken,
    Path(conversation_id): Path<Uuid>,
) -> Response {
    let convo = match conversations::get(&state.pg, token.user_id(), conversation_id).await {
        Ok(c) => c,
        // 404 rather than 403: someone else's conversation should not be
        // distinguishable from one that does not exist.
        Err(_) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
    };
    let turns = conversations::turns(&state.pg, conversation_id)
        .await
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(json!({ "conversation": convo, "turns": turns })),
    )
        .into_response()
}

/// POST /api/agent/conversations/{id}/messages — prompt the agent.
///
/// The whole user-facing surface: a message, and nothing else. Returns as soon as the
/// turn is spawned, because the agent keeps working whether or not anyone is watching.
pub async fn send_message(
    State(state): State<AppState>,
    token: BearerToken,
    Path(conversation_id): Path<Uuid>,
    Json(body): Json<MessageBody>,
) -> Response {
    let creds = LlmCredentialStore::new(state.pg.clone(), state.cred_crypto.clone());
    match state
        .agent
        // No workflow chip on the front door yet (AGENT-012 §3.3).
        .send_message(token.user_id(), conversation_id, &body.text, &creds, None)
        .await
    {
        Ok(run_id) => (StatusCode::ACCEPTED, Json(json!({ "run_id": run_id }))).into_response(),
        Err(e) => {
            let (status, code) = start_error(&e);
            (
                status,
                Json(json!({ "error": code, "message": e.to_string() })),
            )
                .into_response()
        }
    }
}

/// POST /api/agent/conversations/{id}/cancel — the kill switch.
pub async fn cancel_conversation(
    State(state): State<AppState>,
    token: BearerToken,
    Path(conversation_id): Path<Uuid>,
) -> Response {
    match state
        .agent
        .cancel_conversation(token.user_id(), conversation_id)
        .await
    {
        Ok(stopped) => (StatusCode::OK, Json(json!({ "stopped": stopped }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /api/agent/conversations/{id} — archive, not destroy.
pub async fn archive_conversation(
    State(state): State<AppState>,
    token: BearerToken,
    Path(conversation_id): Path<Uuid>,
) -> Response {
    match conversations::archive(&state.pg, token.user_id(), conversation_id).await {
        Ok(true) => (StatusCode::OK, Json(json!({ "archived": true }))).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ── GET /api/agent/runs ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

const RUN_COLUMNS: &str = "run_id, status, goal, provider, model, constraints_json, iterations, \
                           max_iterations, tokens_in, tokens_out, max_total_tokens, \
                           wallclock_budget_secs, error, summary, final_strategy_id, \
                           best_backtest_id, created_at, started_at, finished_at";

fn run_row_to_json(row: &sqlx::postgres::PgRow) -> Value {
    use sqlx::Row;
    let ts = |name: &str| -> Value {
        row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name)
            .ok()
            .flatten()
            .map(|t| json!(t.to_rfc3339()))
            .unwrap_or(Value::Null)
    };
    json!({
        "run_id": row.try_get::<Uuid, _>("run_id").map(|u| u.to_string()).unwrap_or_default(),
        "status": row.try_get::<String, _>("status").unwrap_or_default(),
        "goal": row.try_get::<String, _>("goal").unwrap_or_default(),
        "provider": row.try_get::<String, _>("provider").unwrap_or_default(),
        "model": row.try_get::<String, _>("model").unwrap_or_default(),
        "constraints": row.try_get::<Value, _>("constraints_json").unwrap_or(Value::Null),
        "iterations": row.try_get::<i32, _>("iterations").unwrap_or(0),
        "max_iterations": row.try_get::<i32, _>("max_iterations").unwrap_or(0),
        "tokens_in": row.try_get::<i64, _>("tokens_in").unwrap_or(0),
        "tokens_out": row.try_get::<i64, _>("tokens_out").unwrap_or(0),
        "max_total_tokens": row.try_get::<Option<i64>, _>("max_total_tokens").ok().flatten(),
        "wallclock_budget_secs": row.try_get::<i32, _>("wallclock_budget_secs").unwrap_or(0),
        "error": row.try_get::<Option<String>, _>("error").ok().flatten(),
        "summary": row.try_get::<Option<String>, _>("summary").ok().flatten(),
        "final_strategy_id": row.try_get::<Option<String>, _>("final_strategy_id").ok().flatten(),
        "best_backtest_id": row.try_get::<Option<Uuid>, _>("best_backtest_id").ok().flatten()
            .map(|u| json!(u.to_string())).unwrap_or(Value::Null),
        "created_at": ts("created_at"),
        "started_at": ts("started_at"),
        "finished_at": ts("finished_at"),
    })
}

pub async fn list_runs(
    State(state): State<AppState>,
    token: BearerToken,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(25).clamp(1, 100);
    let offset = params.offset.unwrap_or(0).max(0);
    let rows = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE user_id = $1
         ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(token.user_id())
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pg)
    .await;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_runs WHERE user_id = $1")
        .bind(token.user_id())
        .fetch_one(&state.pg)
        .await
        .unwrap_or(0);

    match rows {
        Ok(rows) => {
            let runs: Vec<Value> = rows.iter().map(run_row_to_json).collect();
            Json(json!({ "runs": runs, "total": total })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list agent runs failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "list_failed" })),
            )
                .into_response()
        }
    }
}

// ── GET /api/agent/runs/{id} ─────────────────────────────────────────────────

pub async fn get_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
) -> Response {
    let row = sqlx::query(&format!(
        "SELECT {RUN_COLUMNS} FROM agent_runs WHERE run_id = $1 AND user_id = $2"
    ))
    .bind(run_id)
    .bind(token.user_id())
    .fetch_optional(&state.pg)
    .await;

    match row {
        Ok(Some(row)) => Json(run_row_to_json(&row)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "get agent run failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "get_failed" })),
            )
                .into_response()
        }
    }
}

// ── GET /api/agent/runs/{id}/messages ────────────────────────────────────────

#[derive(Deserialize)]
pub struct MessagesParams {
    #[serde(default)]
    after_seq: Option<i32>,
    #[serde(default)]
    limit: Option<i64>,
}

pub async fn get_messages(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
    Query(params): Query<MessagesParams>,
) -> Response {
    // Ownership check via the run row (messages have no user column).
    let owned: Option<(Uuid,)> =
        sqlx::query_as("SELECT run_id FROM agent_runs WHERE run_id = $1 AND user_id = $2")
            .bind(run_id)
            .bind(token.user_id())
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();
    if owned.is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response();
    }

    let after_seq = params.after_seq.unwrap_or(0);
    let limit = params.limit.unwrap_or(200).clamp(1, 500);
    // Row shape: (seq, kind, content_json, created_at).
    type MessageRow = (i32, String, Value, chrono::DateTime<chrono::Utc>);
    let rows: Result<Vec<MessageRow>, _> = sqlx::query_as(
        "SELECT seq, kind, content_json, created_at FROM agent_messages
         WHERE run_id = $1 AND seq > $2 ORDER BY seq ASC LIMIT $3",
    )
    .bind(run_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => {
            let last_seq = rows.last().map(|(seq, _, _, _)| *seq).unwrap_or(after_seq);
            let messages: Vec<Value> = rows
                .into_iter()
                .map(|(seq, kind, content, created_at)| {
                    json!({
                        "seq": seq,
                        "kind": kind,
                        "content": content,
                        "created_at": created_at.to_rfc3339(),
                    })
                })
                .collect();
            Json(json!({ "messages": messages, "last_seq": last_seq })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "get agent messages failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "messages_failed" })),
            )
                .into_response()
        }
    }
}

// ── POST /api/agent/runs/{id}/cancel ─────────────────────────────────────────

pub async fn cancel_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
) -> Response {
    match state.agent.cancel(token.user_id(), run_id).await {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "cancel agent run failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "cancel_failed" })),
            )
                .into_response()
        }
    }
}

// ── The bounded pause: POST /api/agent/runs/{id}/pause and …/resume ──────────
//
// # Why these exist, and why `escalate` does not
//
// AGENT-012 requires a pause in §3.4 and makes it an acceptance criterion in
// §19.3. §17 marked it unbuilt and the platform agreed: both paths answered 404
// while `…/cancel` answered 401.
//
// The desk's three buttons were deleted on 2026-09-21, and that did not close
// the need. Pause was the worst of them because it did not merely do nothing —
// `onPause` set a local `paused` flag *before* issuing the request that 404'd,
// so the conversation header read **paused** over a run still spending trials,
// and the Stop dialog offered "Pause instead" as the gentler option. The
// operator was being talked out of the control that works and into the one that
// did not.
//
// `escalate` stays struck (ADR-D-27). A mid-run tier change carries its own
// budget consequences, has no caller and no demand, and the honest way to
// change a run's tier is to stop it and start one.
//
// # Bounded, and what "bounded" is doing
//
// A paused run holds its budget, its run-scoped service token and its slot in
// the manager's map, so an unbounded pause is a resource leak that looks like an
// operator being careful. Every pause carries a deadline — clamped by the
// `PauseWindow` type, not by a check a later caller could skip — and **on
// expiry the run stops. It does not resume.**
//
// The deadline is enforced by the driver task itself: it is parked inside
// `RunControl::hold_at_boundary`, selecting the deadline against the wake-up. It
// is not a column somebody hopes a sweeper reads.
//
// # The one thing an operator has to know
//
// Only a run **this process is driving** can be paused, because a pause is a
// loop being held and nothing else. An orphan, or a run of another replica,
// gets 409 with that sentence rather than a success that changes nothing.

#[derive(Debug, Deserialize)]
pub struct PauseBody {
    /// How long to hold it. Clamped into the window the platform grants; the
    /// response says what was actually granted, because a clamp the caller
    /// cannot see is a deadline they are wrong about.
    #[serde(default)]
    pub seconds: Option<i64>,
}

/// POST /api/agent/runs/{id}/pause
pub async fn pause_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
    body: Option<Json<PauseBody>>,
) -> Response {
    let window = body
        .and_then(|Json(b)| b.seconds)
        .map_or_else(super::control::PauseWindow::default, |s| {
            super::control::PauseWindow::from_seconds(s)
        });

    match state.agent.pause(token.user_id(), run_id, window).await {
        Ok(PauseOutcome::Paused { until, seconds }) => Json(json!({
            "paused": true,
            "until": until.to_rfc3339(),
            "seconds": seconds,
            // Stated in the response because it is the half of the contract an
            // operator would otherwise assume the other way round.
            "on_expiry": "stopped",
        }))
        .into_response(),
        Ok(PauseOutcome::NoSuchRun) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
        Ok(PauseOutcome::AlreadyFinished(status)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "run_already_finished",
                "status": status,
                "message": "this run has ended; there is no next step to hold it at",
            })),
        )
            .into_response(),
        Ok(PauseOutcome::NotDrivenHere(status)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "run_not_driven_here",
                "status": status,
                "message": "a pause is a loop being held at its next step, and this process is                             not driving this run — so there is nothing to hold. Cancel it instead.",
            })),
        )
            .into_response(),
        Ok(PauseOutcome::AlreadyPaused(until)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_paused",
                "until": until.to_rfc3339(),
                "message": "already held; an extension is a second decision, and a pause that                             renews itself is the unbounded pause the deadline exists to prevent",
            })),
        )
            .into_response(),
        Ok(PauseOutcome::Stopping) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "run_stopping",
                "message": "this run is already stopping",
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "pause agent run failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "pause_failed" })),
            )
                .into_response()
        }
    }
}

/// POST /api/agent/runs/{id}/resume
pub async fn resume_run(
    State(state): State<AppState>,
    token: BearerToken,
    Path(run_id): Path<Uuid>,
) -> Response {
    match state.agent.resume(token.user_id(), run_id).await {
        Ok(ResumeOutcome::Resumed) => Json(json!({ "resumed": true })).into_response(),
        Ok(ResumeOutcome::NoSuchRun) => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
        Ok(ResumeOutcome::AlreadyFinished(status)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "run_already_finished",
                "status": status,
                "message": "this run has ended; a resume cannot restart it",
            })),
        )
            .into_response(),
        Ok(ResumeOutcome::NotDrivenHere(status)) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "run_not_driven_here",
                "status": status,
                "message": "this process is not driving this run, so it holds no pause to lift",
            })),
        )
            .into_response(),
        Ok(ResumeOutcome::NotPaused) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "not_paused",
                "message": "this run is not held; it is already taking steps",
            })),
        )
            .into_response(),
        // The expiry case an operator most needs told plainly: they came back
        // after the deadline, and fail-closed means the run is gone rather than
        // that the click did nothing.
        Ok(ResumeOutcome::Stopping) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "run_stopping",
                "message": "this run is stopping — either it was cancelled, or its pause expired                             and an expired pause stops the run rather than resuming it",
            })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "resume agent run failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "resume_failed" })),
            )
                .into_response()
        }
    }
}

/// POST /api/agent/trajectory — append one agent tool step (SPEC §14.6).
///
/// Every tool step reaches this through `mcp_server_lib::dispatch_tool`, whichever
/// front door the agent came through. The tenant is the authenticated principal,
/// never a field of the body.
pub async fn record_trajectory_step(
    State(state): State<AppState>,
    token: BearerToken,
    Json(step): Json<ledger::trajectory::TrajectoryStep>,
) -> Response {
    let ledger = ledger::pg::PgTrialLedger::new(state.pg.clone());
    match ledger.record_trajectory_step_async(&token.user_id().to_string(), &step).await {
        Ok(()) => (StatusCode::CREATED, Json(json!({ "recorded": true }))).into_response(),
        Err(ledger::LedgerError::Backend(e)) if e.contains("duplicate key") => {
            (StatusCode::CONFLICT, Json(json!({ "error": "step_already_recorded" }))).into_response()
        }
        Err(ledger::LedgerError::Backend(e)) => {
            tracing::warn!(error = %e, "trajectory step insert failed");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "ledger_unavailable" }))).into_response()
        }
        Err(e) => (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": "invalid_step", "message": e.to_string() }))).into_response(),
    }
}

// ── The audit trail's front door (SPEC §15, ADR-P2-21, ADR-P2-34, AT-64) ──────

#[derive(Debug, Deserialize)]
pub struct OpenAuditBody {
    pub action: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
pub struct CloseAuditBody {
    /// What the dispatch returned. The **platform** classifies it; the caller
    /// does not get to say whether it was denied.
    #[serde(default)]
    pub result: Value,
}

/// POST /api/agent/audit — open the `pre` half of an audit record.
///
/// This is the endpoint that closes the external front door's exemption. The
/// MCP server is a separate process (ADR-0022) and it writes no rows; it asks
/// *here*, before it dispatches anything, and a call this endpoint does not
/// answer is a call `mcp_server_lib::dispatch_tool` refuses to make. So the
/// record is written by the component the caller cannot route around
/// (ADR-0025), and the caller's only power over it is to give up the dispatch.
///
/// Nothing about the principal comes from the body. The tenant, the actor kind
/// and the actor id are derived from the `sessions` row the bearer token
/// resolves to, and the approval envelope is derived from the action's name by
/// `ledger::audit::Envelope::for_action` — the same list the policy engine is
/// handed, so a caller cannot declare its own action ungated.
pub async fn open_audit_record(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<OpenAuditBody>,
) -> Response {
    let trail =
        super::audit::AuditTrail::for_session(state.pg.clone(), &token.token, token.user_id())
            .await;
    match trail.pre(&body.action, &body.arguments).await {
        Some((_, pre)) => (
            StatusCode::CREATED,
            Json(json!({ "audit_id": pre.id(), "action": body.action })),
        )
            .into_response(),
        // 503, not 500: the honest reading is "this platform cannot account for
        // a call right now", and the caller's contract is to refuse rather than
        // proceed.
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "audit_unavailable",
                "detail": "the audit trail could not record this request, so it must not be dispatched"
            })),
        )
            .into_response(),
    }
}

/// POST /api/agent/audit/{audit_id} — close it with what the platform decided.
///
/// The action, envelope and principal are read back from the stored `pre` row
/// rather than taken from this request, so the closing call carries no claim at
/// all beyond the id and the result. The verdict is computed here by
/// `verdict_of`, which reads the platform's own refusal shape — a `denied` the
/// caller cannot downgrade to a `failed`.
pub async fn close_audit_record(
    State(state): State<AppState>,
    token: BearerToken,
    Path(audit_id): Path<Uuid>,
    Json(body): Json<CloseAuditBody>,
) -> Response {
    let trail =
        super::audit::AuditTrail::for_session(state.pg.clone(), &token.token, token.user_id())
            .await;
    let Some(opened) = trail.reopen(audit_id).await else {
        // Tenant-scoped: another tenant's audit id is indistinguishable from one
        // that does not exist.
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no_such_audit_record" })),
        )
            .into_response();
    };
    let verdict = super::audit::verdict_of(&body.result);
    trail.post(Some(opened), verdict).await;
    (
        StatusCode::CREATED,
        Json(json!({ "recorded": true, "verdict": verdict.as_str() })),
    )
        .into_response()
}
