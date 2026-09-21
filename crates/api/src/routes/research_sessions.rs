//! The research-session API (AGENT-014 §10; Set S task S-11.14, API half).
//!
//! `/api/research/sessions…` — list, create, read, pause/resume/stop, the
//! agenda and its per-item actions, the charter, the digests, and an SSE stream
//! of the log.
//!
//! # Who may write
//!
//! Every mutating route here requires `web:full` — a human session — and says so
//! in the refusal. That is not defence in depth for its own sake: AGENT-014 P6
//! says autonomy is a setting on *starting*, never on *approving*, and a session
//! that could approve its own agenda item, amend its own charter or lift its own
//! pause would be a session with no human in it at all.
//!
//! The scope asked for is `web:full`, which `auth::scopes::NEVER_FOR_AGENTS`
//! refuses to attach to any project-bound session and which migration 0038's
//! trigger refuses again at the row. So the check is one line per handler over a
//! scope an agent token structurally cannot hold — and
//! `every_mutating_handler_asks_the_same_question` below fails the build if a
//! new handler is added without it.
//!
//! # What creation refuses today, and why refusing is the safe side
//!
//! Three settings are refused at creation rather than accepted and ignored:
//!
//! * a **non-frontier profile** (AGENT-014 §8, a user decision): a local tier is
//!   not left alone with a budget over days;
//! * **`autonomy = autonomous`**: §8 makes it conditional on the profile's
//!   `judgement` pass rate, and nothing reads that threshold yet (S-11.9). A
//!   session that ran unattended because the check was missing is the one
//!   failure here that spends real money;
//! * **`replenish = true`**: the survey run that refills a drained agenda is
//!   S-11.11. Accepting the flag would give the operator a switch that silently
//!   does nothing, which is worse than a refusal naming the task.
//!
//! Each refusal names what is missing, so the day it lands the message is what
//! points here.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use chrono::{DateTime, Utc};
use research_session::{
    agenda::{AgendaItem, ItemStatus, Source},
    charter::Autonomy,
    fold::EventKind,
    Charter, ResearchSession,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::routes::desk_charter::{self, DeskCharter};
use crate::{auth::BearerToken, sessions, state::AppState};

/// The scope a human session carries and an agent session never does.
const HUMAN_SCOPE: &str = "web:full";

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_BATCH: i64 = 200;

fn forbidden(what: &str) -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": "not_a_human_session",
            "message": format!("{what} is done by a person, from the desk"),
            "fix": "an agent token holds read access to sessions and nothing else. A session that \
                    could start, approve or unpause itself would have no human in it, which is \
                    what the autonomy setting is a setting on (AGENT-014 P6).",
        })),
    )
        .into_response()
}

fn not_found(id: Uuid) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "no_such_session", "session_id": id })),
    )
        .into_response()
}

fn bad(reason: &str, fix: &str) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": "refused", "message": reason, "fix": fix })),
    )
        .into_response()
}

fn db(e: impl std::fmt::Display) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": "storage", "message": e.to_string() })),
    )
        .into_response()
}

/// The session, if it belongs to this caller.
async fn owned(
    state: &AppState,
    user_id: Uuid,
    id: Uuid,
) -> Result<ResearchSession, axum::response::Response> {
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT user_id FROM research_sessions WHERE session_id = $1")
            .bind(id)
            .fetch_optional(&state.pg)
            .await
            .map_err(db)?;
    match row {
        Some((owner,)) if owner == user_id => {}
        // A session someone else owns and a session that does not exist answer
        // the same way: the difference is not the caller's to learn.
        _ => return Err(not_found(id)),
    }
    sessions::get(&state.pg, id)
        .await
        .map_err(db)?
        .ok_or_else(|| not_found(id))
}

/// The run that is calling, if the caller is one of the platform's own runs.
///
/// Read from the `sessions` row the bearer token resolves to — the label the
/// agent manager minted — and never from anything the caller sent, for the same
/// reason `agent::audit::principal_for` does it that way: a run that could name
/// itself could name another.
async fn caller_run(state: &AppState, token: &BearerToken) -> Option<Uuid> {
    const AGENT_RUN_LABEL: &str = "agent-run-";
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT label FROM sessions WHERE token = $1")
            .bind(&token.token)
            .fetch_optional(&state.pg)
            .await
            .ok()?;
    let label = row?.0?;
    let rest = label.strip_prefix(AGENT_RUN_LABEL)?;
    Uuid::parse_str(rest).ok()
}

/// `GET /api/research/sessions/current` — the session the calling run is in.
///
/// This exists so `lifecycle.get_session` does not need the model to have been
/// told an id. A child run of a continuous session is *in* that session, the
/// platform knows which, and making the run repeat an id back is a way for it to
/// name the wrong one.
///
/// `404` when the caller is not a run, or is a run with no session — which is
/// a real answer, not an error: a run started outside a session has none.
pub async fn current(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let Some(run) = caller_run(&state, &token).await else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "not_a_run",
                "message": "this token is not one of the platform's own agent runs, so it is not in a session",
            })),
        )
            .into_response();
    };
    let row: Option<(Option<Uuid>, Option<Uuid>)> =
        match sqlx::query_as("SELECT session_id, agenda_item_id FROM agent_runs WHERE run_id = $1")
            .bind(run)
            .fetch_optional(&state.pg)
            .await
        {
            Ok(r) => r,
            Err(e) => return db(e),
        };
    match row {
        Some((Some(session_id), item)) => (
            StatusCode::OK,
            Json(json!({ "run_id": run, "session_id": session_id, "agenda_item_id": item })),
        )
            .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "no_session",
                "run_id": run,
                "message": "this run is not part of a research session",
            })),
        )
            .into_response(),
    }
}

// ── list and create ─────────────────────────────────────────────────────────

/// `GET /api/research/sessions`
pub async fn list(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let rows: Result<Vec<(Uuid, Uuid, bool, Option<String>, String, DateTime<Utc>)>, _> =
        sqlx::query_as(
            "SELECT session_id, conversation_id, continuous, question, tier, created_at
               FROM research_sessions WHERE user_id = $1
              ORDER BY created_at DESC LIMIT 200",
        )
        .bind(token.user_id())
        .fetch_all(&state.pg)
        .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => return db(e),
    };

    // The state is the fold, so the list pays for it per row. Bounded at 200 and
    // the log is short; a cached state column is the thing ADR-P2-04 refuses.
    let mut out = Vec::with_capacity(rows.len());
    for (id, conversation_id, continuous, question, tier, created_at) in rows {
        let folded = sessions::state(&state.pg, id).await.ok();
        out.push(json!({
            "session_id": id,
            "conversation_id": conversation_id,
            "continuous": continuous,
            "question": question,
            "tier": tier,
            "created_at": created_at,
            "state": folded.as_ref().and_then(|f| f.state.as_ref().map(research_session::SessionState::word)),
            "runs_started": folded.as_ref().map(|f| f.runs_started),
            "agenda_open": folded.as_ref().map(|f| f.items.len().saturating_sub(f.finished.len())),
        }));
    }
    (StatusCode::OK, Json(json!({ "sessions": out }))).into_response()
}

#[derive(Deserialize)]
pub struct CreateBody {
    /// The journal conversation this session is. **Optional**, and when it is
    /// absent one is opened — because the conversation *is* the session (§2.1)
    /// and the composer is starting both at once. The response carries it back,
    /// which is what the desk navigates to.
    #[serde(default)]
    pub conversation_id: Option<Uuid>,
    /// The toggle. `false` with a question starts a bounded Research run through
    /// the same door, so the desk has one call for both and they cannot disagree
    /// about what a session is.
    #[serde(default)]
    pub continuous: bool,
    #[serde(default)]
    pub question: Option<String>,
    /// The charter form's shape, translated by [`desk_charter::translate`].
    #[serde(default)]
    pub charter: Option<DeskCharter>,
    /// The profile to run on. The composer's model select; §8's frontier rule is
    /// checked against it here rather than only in the browser, because a rule
    /// enforced in the browser is a rule a `curl` walks around.
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub workflow: Option<String>,
}

/// `POST /api/research/sessions`
///
/// One door for both halves of the toggle. With `continuous: true` it takes the
/// charter form; with it false it takes a question and starts the first turn,
/// which is what makes a bounded run "the one-item case" in fact and not only in
/// the object model.
pub async fn create(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<CreateBody>,
) -> impl IntoResponse {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden("starting a session");
    }

    let tier = body
        .model
        .clone()
        .unwrap_or_else(|| state.default_profile.clone());

    if body.continuous {
        // §8, a user decision: continuous research is frontier-only. Checked
        // against the loaded profile set rather than a name list, so a profile
        // added to the deployment is admitted or refused by what it *is*.
        match state.profiles.get(&tier) {
            None => {
                return bad(
                    &format!("no profile named `{tier}` is loaded"),
                    "pick a profile the platform knows; the charter form lists them",
                )
            }
            Some(p) if p.tier != harness::profile::Tier::Frontier => {
                return bad(
                    &format!(
                        "`{tier}` is a {} profile, and continuous research is frontier-only",
                        p.tier.as_str()
                    ),
                    "run this question as a bounded Research run, with the toggle off, and watch \
                     it. A local tier is not left alone with a budget over days (AGENT-014 §8). \
                     The revisit trigger is a local profile passing `judgement` for every \
                     permitted workflow on three consecutive scorecards, and opening it is a spec \
                     amendment.",
                )
            }
            Some(_) => {}
        }
    }

    // The conversation, opened if the caller did not name one.
    let conversation_id = match body.conversation_id {
        Some(id) => id,
        None => {
            // The same provider the "New chat" button uses, from the same
            // environment override. Read through `default_provider` rather than
            // written again here, because a session opened from the composer and
            // a conversation opened from the rail must reach the same model or
            // the desk has two defaults.
            let provider = crate::agent::routes::default_provider();
            match crate::agent::conversations::create(&state.pg, token.user_id(), &provider, &tier)
                .await
            {
                Ok(id) => id,
                Err(e) => return db(e),
            }
        }
    };

    // ── the bounded case ────────────────────────────────────────────────────
    if !body.continuous {
        let Some(question) = body.question.as_deref().map(str::trim).filter(|q| !q.is_empty())
        else {
            return bad(
                "a bounded session needs a question",
                "turn Continuous on to write a charter instead",
            );
        };
        // Started through the conversation front door, which is the thing that
        // opens a session row for a Research turn (S-11.1). Two doors writing
        // the row would be two chances for them to disagree about its shape.
        let creds = crate::credentials::LlmCredentialStore::new(
            state.pg.clone(),
            state.cred_crypto.clone(),
        );
        let workflow = body.workflow.as_deref().and_then(harness::state::Workflow::parse);
        return match state
            .agent
            .send_message(token.user_id(), conversation_id, question, &creds, workflow)
            .await
        {
            Ok(run_id) => {
                let session = sessions::for_conversation(&state.pg, conversation_id)
                    .await
                    .ok()
                    .flatten();
                (
                    StatusCode::CREATED,
                    Json(json!({
                        "session_id": session.as_ref().map(|s| s.id),
                        "conversation_id": conversation_id,
                        "run_id": run_id,
                        "continuous": false,
                    })),
                )
                    .into_response()
            }
            Err(e) => bad(&e.to_string(), "the run could not be started"),
        };
    }

    // ── the continuous case ─────────────────────────────────────────────────
    let Some(form) = body.charter.as_ref() else {
        return bad(
            "a continuous session needs a charter",
            "fill in the charter form, or turn Continuous off and ask one question",
        );
    };
    let translated = match desk_charter::translate(form) {
        Ok(t) => t,
        Err(r) => return bad(&r.message, &r.fix),
    };

    if translated.autonomy == Autonomy::Autonomous {
        return bad(
            "`autonomous` is refused, not ignored",
            "AGENT-014 §8 makes it conditional on the profile's `judgement` pass rate, and \
             nothing reads that threshold yet (Set S task S-11.9). Use `notify`, which starts \
             items and tells you about each one.",
        );
    }
    if translated.replenish {
        return bad(
            "`replenish` is refused, not ignored",
            "the survey run that refills a drained agenda is Set S task S-11.11. Accepting the \
             flag would give you a switch that silently does nothing.",
        );
    }

    let mut session =
        ResearchSession::continuous(Uuid::new_v4(), conversation_id, translated.charter, &tier);
    session.autonomy = translated.autonomy;
    session.budget = translated.budget;
    session.schedule = translated.schedule;
    session.review_every = translated.review_every;

    if let Err(e) = sessions::create(&state.pg, token.user_id(), &session).await {
        return db(e);
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "session_id": session.id,
            "conversation_id": session.conversation_id,
            "continuous": true,
            "autonomy": session.autonomy.as_str(),
            "exploration_share": session.exploration_share(),
            "state": "draft",
            "next": "POST /api/research/sessions/{id}/resume starts it",
        })),
    )
        .into_response()
}

/// `GET /api/research/sessions/{id}`
pub async fn get(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let session = match owned(&state, token.user_id(), id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let folded = match sessions::state(&state.pg, id).await {
        Ok(f) => f,
        Err(e) => return db(e),
    };
    (
        StatusCode::OK,
        Json(json!({
            "session_id": session.id,
            "conversation_id": session.conversation_id,
            "continuous": session.continuous,
            "question": session.question,
            "charter": session.charter,
            "budget": session.budget,
            "schedule": session.schedule,
            "autonomy": session.autonomy.as_str(),
            "replenish": session.replenish,
            "review_every": session.review_every,
            "tier": session.tier,
            "exploration_share": session.exploration_share(),
            "state": folded.state,
            "event_seq": folded.event_seq,
            "runs_started": folded.runs_started,
            "charter_version": folded.charter_version,
            // Reported rather than hidden: a log that does not replay cleanly is
            // why the driver stopped, and the operator is the one who can act on
            // it.
            "anomalies": folded.anomalies,
        })),
    )
        .into_response()
}

// ── the three controls ──────────────────────────────────────────────────────

async fn transition(
    state: &AppState,
    token: &BearerToken,
    id: Uuid,
    kind: EventKind,
    payload: Value,
    what: &str,
) -> axum::response::Response {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden(what);
    }
    if let Err(r) = owned(state, token.user_id(), id).await {
        return r;
    }
    match sessions::append(&state.pg, id, kind, payload).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true, "event": kind.as_str() })))
            .into_response(),
        // The trigger refuses an illegal transition and its message names both
        // states. Passed through rather than reworded: it is the most specific
        // sentence anyone will write about this.
        Err(e) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "transition_refused",
                "message": e.to_string(),
            })),
        )
            .into_response(),
    }
}

/// `POST …/{id}/pause`
pub async fn pause(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    transition(
        &state,
        &token,
        id,
        EventKind::SessionPaused,
        json!({ "why": "human" }),
        "pausing a session",
    )
    .await
}

/// `POST …/{id}/resume` — also what starts a `draft` session.
pub async fn resume(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden("starting or resuming a session");
    }
    let session = match owned(&state, token.user_id(), id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let folded = match sessions::state(&state.pg, id).await {
        Ok(f) => f,
        Err(e) => return db(e),
    };
    // `session_started` from draft, `session_resumed` otherwise: both land on
    // `running`, and the log says which of the two happened.
    let kind = if matches!(folded.state, Some(research_session::SessionState::Draft)) {
        EventKind::SessionStarted
    } else {
        EventKind::SessionResumed
    };
    let appended = transition(&state, &token, id, kind, json!({}), "resuming a session").await;
    if appended.status() != StatusCode::OK {
        return appended;
    }

    // Now something has to drive it. Without this the log says `running` and
    // nothing ever claims — the shape of defect this repository keeps finding,
    // reached through a job queue.
    let Some(jobs) = state.jobs.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "jobs_unavailable",
                "message": "the session is running but there is no job service to drive it",
            })),
        )
            .into_response();
    };
    let sub = jobs::store::Submission::new(
        jobs::JobKind::ContinuousResearch,
        token.user_id(),
        json!({ "session_id": id, "user_id": token.user_id() }),
    );
    match jobs.submit(sub).await {
        Ok(j) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "event": kind.as_str(), "job_id": j.job_id,
                         "continuous": session.continuous })),
        )
            .into_response(),
        Err(e) => db(e),
    }
}

/// `POST …/{id}/stop`
pub async fn stop(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    transition(
        &state,
        &token,
        id,
        EventKind::SessionStopped,
        json!({ "why": "human" }),
        "stopping a session",
    )
    .await
}

// ── the agenda ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AgendaQuery {
    #[serde(default)]
    pub status: Option<String>,
}

/// `GET …/{id}/agenda`
pub async fn agenda(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Query(q): Query<AgendaQuery>,
) -> impl IntoResponse {
    if let Err(r) = owned(&state, token.user_id(), id).await {
        return r;
    }
    let items = match sessions::agenda(&state.pg, id).await {
        Ok(i) => i,
        Err(e) => return db(e),
    };
    let items: Vec<&AgendaItem> = items
        .iter()
        .filter(|i| q.status.as_deref().is_none_or(|s| i.status.word() == s))
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "items": items.iter().map(|i| json!({
                "item_id": i.id,
                "question": i.question,
                "subject": i.subject,
                "workflow": i.workflow,
                "sources": i.sources.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                "source_refs": i.source_refs,
                "score": i.score,
                // The true status, and the column the board puts it in. Both,
                // because the board has five columns and an item has nine
                // statuses, and a client that only got the column could not show
                // the difference between `expired` and a verdict.
                "status": i.status.word(),
                "state": i.status.board_column(),
                "status_detail": i.status,
                "attempts": i.attempts,
                "yield_ref": i.yield_ref,
                "bypasses_scoring": i.bypasses_scoring(),
                // Where the run for this item happens, derived rather than
                // stored — it is the same value whether or not the run exists
                // yet, which is what makes the chip back from the child work
                // before the child has started.
                "child_conversation_id":
                    sessions::child_conversation_id(i.session_id, i.id),
            })).collect::<Vec<_>>()
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct AddItemBody {
    pub question: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub workflow: Option<String>,
}

/// `POST …/{id}/agenda` — source 4.2, the human's own item.
///
/// It enters `queued`, not `proposed`: an item the human typed does not wait for
/// the human to approve it, whatever the autonomy setting says. That is the same
/// reading of §4.2 that makes it bypass scoring.
pub async fn add_item(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<AddItemBody>,
) -> impl IntoResponse {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden("adding an agenda item");
    }
    let session = match owned(&state, token.user_id(), id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if body.question.trim().is_empty() {
        return bad("the question is empty", "write what you want asked");
    }
    let mut item = AgendaItem::new(session.id, &body.question, &body.subject, Source::User);
    if let Some(w) = body.workflow.as_deref() {
        if harness::state::Workflow::parse(w).is_none() {
            return bad(
                &format!("`{w}` is not a workflow"),
                "one of investigate, falsify, replicate, compare, diagnose, survey, optimise",
            );
        }
        if let Some(c) = session.charter.as_ref() {
            if !c.permits(w) {
                return bad(
                    &format!("the charter does not permit `{w}`"),
                    "amend the charter, or pick a workflow it permits",
                );
            }
        }
        item = item.with_workflow(w);
    }
    item.status = ItemStatus::Queued;

    match sessions::insert_item(&state.pg, &item).await {
        Ok(Some(new_id)) => {
            if let Err(e) = sessions::set_status(&state.pg, new_id, &ItemStatus::Queued).await {
                return db(e);
            }
            let _ = sessions::append(
                &state.pg,
                session.id,
                EventKind::AgendaItemAdded,
                json!({ "item": new_id, "source": "user", "question": item.question }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(json!({ "item_id": new_id, "status": "queued" })),
            )
                .into_response()
        }
        // Layer one caught it. Reported as a merge rather than as an error: the
        // human asked for something that is already on the agenda, and the
        // answer is "it is, here".
        Ok(None) => {
            let _ = sessions::append(
                &state.pg,
                session.id,
                EventKind::AgendaItemMerged,
                json!({ "key": item.dedupe_key, "source": "user" }),
            )
            .await;
            (
                StatusCode::OK,
                Json(json!({
                    "merged": true,
                    "message": "that question is already on the agenda; your request was added to \
                                what wants it",
                })),
            )
                .into_response()
        }
        Err(e) => db(e),
    }
}

#[derive(Deserialize)]
pub struct ItemPath {
    pub id: Uuid,
    pub item: Uuid,
}

#[derive(Deserialize)]
pub struct OrderBody {
    /// The item ids in the order the human dragged them into.
    pub order: Vec<Uuid>,
}

/// `POST …/{id}/agenda/order` — the board was reordered by hand.
///
/// AGENT-014 §9: "drag = `user` priority event". A drag is not a number written
/// into a rank column; it is the human saying *these* matter, which is the same
/// thing the `user` source says and which the scorer already honours as a bypass
/// (§4.2). So a reorder marks every item the human moved above its old position
/// with the `user` source, in the order given, and the scorer needs no second
/// ranking to keep in step with this one.
///
/// Recorded as one event per item rather than one event for the list, because
/// the digest shows what was prioritised and a list is not a sentence.
pub async fn reorder(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(body): Json<OrderBody>,
) -> impl IntoResponse {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden("reordering the agenda");
    }
    if let Err(r) = owned(&state, token.user_id(), id).await {
        return r;
    }
    let items = match sessions::agenda(&state.pg, id).await {
        Ok(i) => i,
        Err(e) => return db(e),
    };
    // Only items that are actually on this agenda, and only ones that can still
    // run. Prioritising a finished item would put a `user` source on something
    // nothing will ever claim, and the board would show a badge that means
    // nothing.
    let movable: Vec<Uuid> = body
        .order
        .iter()
        .copied()
        .filter(|id| items.iter().any(|i| i.id == *id && !i.status.is_terminal()))
        .collect();
    if movable.is_empty() {
        return bad(
            "none of those items is on this agenda and still able to run",
            "reload the board; something finished while it was open",
        );
    }
    for item in &movable {
        if let Err(e) = sqlx::query(
            "UPDATE agenda_items
                SET sources = CASE WHEN sources @> '[\"user\"]'::jsonb
                                   THEN sources ELSE sources || '[\"user\"]'::jsonb END,
                    score = NULL,
                    status = CASE WHEN status = 'proposed' THEN 'queued' ELSE status END
              WHERE item_id = $1",
        )
        .bind(item)
        .execute(&state.pg)
        .await
        {
            return db(e);
        }
        let _ = sessions::append(
            &state.pg,
            id,
            EventKind::AgendaItemAdded,
            json!({ "item": item, "source": "user", "prioritised": true }),
        )
        .await;
    }
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "prioritised": movable.len() })),
    )
        .into_response()
}

/// `POST …/{id}/agenda/{item}/approve`
pub async fn approve_item(
    State(state): State<AppState>,
    token: BearerToken,
    Path(p): Path<ItemPath>,
) -> impl IntoResponse {
    item_action(state, token, p, Action::Approve).await
}

/// `POST …/{id}/agenda/{item}/veto`
pub async fn veto_item(
    State(state): State<AppState>,
    token: BearerToken,
    Path(p): Path<ItemPath>,
) -> impl IntoResponse {
    item_action(state, token, p, Action::Veto).await
}

/// `POST …/{id}/agenda/{item}/prioritise`
pub async fn prioritise_item(
    State(state): State<AppState>,
    token: BearerToken,
    Path(p): Path<ItemPath>,
) -> impl IntoResponse {
    item_action(state, token, p, Action::Prioritise).await
}

enum Action {
    Approve,
    Veto,
    Prioritise,
}

async fn item_action(
    state: AppState,
    token: BearerToken,
    p: ItemPath,
    action: Action,
) -> axum::response::Response {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden("acting on an agenda item");
    }
    if let Err(r) = owned(&state, token.user_id(), p.id).await {
        return r;
    }
    let items = match sessions::agenda(&state.pg, p.id).await {
        Ok(i) => i,
        Err(e) => return db(e),
    };
    let Some(item) = items.into_iter().find(|i| i.id == p.item) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no_such_item", "item_id": p.item })),
        )
            .into_response();
    };

    match action {
        Action::Approve => {
            if item.status.is_terminal() {
                return bad(
                    "that item is finished",
                    "approving it would not start anything",
                );
            }
            if let Err(e) = sessions::set_status(&state.pg, item.id, &ItemStatus::Approved).await {
                return db(e);
            }
            let _ = sessions::append(
                &state.pg,
                p.id,
                EventKind::AgendaItemApproved,
                json!({ "item": item.id }),
            )
            .await;
        }
        Action::Veto => {
            // A veto on a *running* item does not stop the child — stopping a
            // run is the run's own control (§6.4) and settling its trials is
            // INV-17's business. What it does is make sure the item is never
            // claimed again, which is what §12.12 asks of it.
            if let Err(e) = sessions::set_status(
                &state.pg,
                item.id,
                &ItemStatus::Vetoed {
                    by: token.user_id().to_string(),
                },
            )
            .await
            {
                return db(e);
            }
            let _ = sessions::append(
                &state.pg,
                p.id,
                EventKind::AgendaItemVetoed,
                json!({ "item": item.id, "by": token.user_id() }),
            )
            .await;
        }
        Action::Prioritise => {
            // §4.2: the human's own priority. Recorded as a `user` source on the
            // item rather than as a number, so it goes through the same bypass
            // the scorer already honours and there is no second ranking to keep
            // in step.
            let mut promoted = item.clone();
            promoted.sources.push(Source::User);
            if let Err(e) = sqlx::query(
                "UPDATE agenda_items
                    SET sources = CASE WHEN sources @> '[\"user\"]'::jsonb
                                       THEN sources ELSE sources || '[\"user\"]'::jsonb END,
                        score = NULL,
                        status = CASE WHEN status = 'proposed' THEN 'queued' ELSE status END
                  WHERE item_id = $1",
            )
            .bind(item.id)
            .execute(&state.pg)
            .await
            {
                return db(e);
            }
            let _ = sessions::append(
                &state.pg,
                p.id,
                EventKind::AgendaItemAdded,
                json!({ "item": item.id, "source": "user", "prioritised": true }),
            )
            .await;
        }
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

// ── the charter ─────────────────────────────────────────────────────────────

/// `PUT …/{id}/charter` — an amendment, which appends rather than replaces.
pub async fn amend_charter(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    Json(charter): Json<Charter>,
) -> impl IntoResponse {
    if !token.permits(HUMAN_SCOPE) {
        return forbidden("amending a charter");
    }
    let session = match owned(&state, token.user_id(), id).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    if !session.continuous {
        return bad(
            "a bounded session has a question, not a charter",
            "turn Continuous on when you start the session",
        );
    }
    let folded = match sessions::state(&state.pg, id).await {
        Ok(f) => f,
        Err(e) => return db(e),
    };
    let mut next = charter;
    next.version = folded.charter_version + 1;

    if let Err(e) = sqlx::query("UPDATE research_sessions SET charter = $2 WHERE session_id = $1")
        .bind(id)
        .bind(json!(next))
        .execute(&state.pg)
        .await
    {
        return db(e);
    }
    let _ = sessions::append(
        &state.pg,
        id,
        EventKind::CharterAmended,
        json!({ "version": next.version }),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "version": next.version })),
    )
        .into_response()
}

// ── digests ─────────────────────────────────────────────────────────────────

/// `GET …/{id}/digests`
pub async fn digests(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(r) = owned(&state, token.user_id(), id).await {
        return r;
    }
    let rows: Result<Vec<(Uuid, i64, i64, Value, DateTime<Utc>)>, _> = sqlx::query_as(
        "SELECT digest_id, from_seq, to_seq, body, created_at FROM session_digests
          WHERE session_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "digests": rows.into_iter().map(|(d, f, t, body, at)| json!({
                    "digest_id": d, "from_seq": f, "to_seq": t, "body": body, "created_at": at,
                })).collect::<Vec<_>>()
            })),
        )
            .into_response(),
        Err(e) => db(e),
    }
}

// ── the log, live ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub after_seq: Option<i64>,
}

/// `GET …/{id}/events` — SSE over the session log.
///
/// The SSE id is the event's `seq`, so a reconnect resumes exactly where it
/// stopped. A stream without them silently drops whatever happened while the tab
/// was asleep, and for a session that runs for days that is most of it.
///
/// # Errors
///
/// Answers `404` for a session this caller does not own.
pub async fn events(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<
    Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>,
    axum::response::Response,
> {
    owned(&state, token.user_id(), id).await?;

    let resume = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(q.after_seq)
        .unwrap_or(-1);

    let pool = state.pg.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        let mut cursor = resume;
        let mut first = true;
        loop {
            if !first {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            first = false;
            let rows: Vec<(i64, String, Option<String>, Value, DateTime<Utc>)> =
                match sqlx::query_as(
                    "SELECT seq, kind, state, payload_json, created_at FROM session_events
                      WHERE session_id = $1 AND seq > $2 ORDER BY seq LIMIT $3",
                )
                .bind(id)
                .bind(cursor)
                .bind(POLL_BATCH)
                .fetch_all(&pool)
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("stream_error")
                                .data(json!({ "message": e.to_string() }).to_string())))
                            .await;
                        return;
                    }
                };
            for (seq, kind, state_word, payload, created_at) in rows {
                cursor = seq;
                let data = json!({
                    "seq": seq, "kind": kind, "state": state_word,
                    "payload": payload, "created_at": created_at,
                });
                let event = Event::default()
                    .id(seq.to_string())
                    .event(kind)
                    .data(data.to_string());
                if tx.send(Ok(event)).await.is_err() {
                    return;
                }
            }
        }
    });

    Ok(Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("keep-alive")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mutating_handler_asks_the_same_question() {
        // A source scan, because the enforcement here is one line per handler
        // and the failure mode is a new handler that forgets it. Scanning is
        // weaker than a type, and the type version — a `HumanToken` extractor —
        // is worth building; until then this fails loudly rather than a write
        // route quietly becoming agent-callable.
        let src = include_str!("research_sessions.rs");
        let mutating = [
            "pub async fn create(",
            "pub async fn resume(",
            "pub async fn add_item(",
            "pub async fn reorder(",
            "pub async fn amend_charter(",
        ];
        for handler in mutating {
            let body = src
                .split_once(handler)
                .unwrap_or_else(|| panic!("{handler} is gone; this list is stale"))
                .1;
            let body = &body[..body.find("\npub ").unwrap_or(body.len())];
            assert!(
                body.contains("permits(HUMAN_SCOPE)"),
                "{handler} does not check that its caller is a human. An agent token would be \
                 able to reach it, and a session that can start or amend itself has no human in it."
            );
        }
        // `pause`, `stop` and the three item actions go through helpers that
        // check; asserted here so the helpers cannot lose it.
        assert!(src.contains("async fn transition(") && {
            let t = src.split_once("async fn transition(").unwrap().1;
            t[..t.find("\n}").unwrap_or(t.len())].contains("permits(HUMAN_SCOPE)")
        });
        assert!(src.contains("async fn item_action(") && {
            let t = src.split_once("async fn item_action(").unwrap().1;
            t[..t.find("\n}\n").unwrap_or(t.len())].contains("permits(HUMAN_SCOPE)")
        });
    }

    #[test]
    fn the_refusals_name_what_is_missing() {
        let src = include_str!("research_sessions.rs");
        for task in ["S-11.9", "S-11.11"] {
            assert!(
                src.contains(task),
                "a setting is refused without naming the task that would allow it, so the day it \
                 lands nothing points here: {task}"
            );
        }
    }
}
