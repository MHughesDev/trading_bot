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
    // The amendments, newest first. `charter_version` on its own reports that
    // decisions were taken without letting anyone read them, which is what the
    // Charter tab had to say in as many words.
    let amendments: Vec<(i64, Value, DateTime<Utc>)> = match sqlx::query_as(
        "SELECT seq, payload_json, created_at FROM session_events
          WHERE session_id = $1 AND kind = $2 ORDER BY seq DESC LIMIT 100",
    )
    .bind(id)
    .bind(EventKind::CharterAmended.as_str())
    .fetch_all(&state.pg)
    .await
    {
        Ok(rows) => rows,
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
            "amendments": amendments
                .iter()
                .map(|(seq, payload, at)| amendment_json(*seq, payload, *at))
                .collect::<Vec<_>>(),
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
///
/// # Why "moved above its old position" is enforced here and not in the client
///
/// A `user` source is not a nudge, it is a **bypass**:
/// `AgendaItem::bypasses_scoring` is true for it and `select::select` returns
/// such an item immediately with propensity 1.0, without scoring the slate. So
/// marking every id in the body would mean one drag of one card switching the
/// selection policy off for the whole session — no scores, no exploration draw,
/// no candidate set in `mlops.decision` — and AGENT-014 P2 ("the agenda is
/// scored, not vibed") and the INV-21 exploration floor would both stop applying
/// without anything on screen saying so.
///
/// A drag-and-drop client naturally posts the **whole board**, so the common
/// case is the dangerous one. [`moved_earlier`] is the difference between the
/// two, and it lives in the route because a client that posts only the moved
/// cards is a discipline, and the next caller — a script, a replay, a second
/// client — does not inherit it.
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
    // An order is a statement about what comes before what, and a list of one
    // makes no such statement — there is nothing in the body for it to have
    // moved past. Refused rather than answered with `prioritised: 0`, because a
    // client that posts only the card it dragged would otherwise get a 200 for a
    // request that did nothing, which is the shape of failure that survives
    // longest. The single-card intent has its own route.
    if movable.len() < 2 {
        return bad(
            "an order of one item does not say what it comes before",
            "post the board's whole order, or use the item's own prioritise action to say this \
             one matters",
        );
    }
    // A card cannot be in two places at once, and an order that says it is has
    // two answers for where it moved. Refused rather than deduplicated: guessing
    // which of the two positions was meant is how a card gets the `user` bypass
    // on the strength of a client bug.
    if movable
        .iter()
        .enumerate()
        .any(|(i, id)| movable[..i].contains(id))
    {
        return bad(
            "that order lists the same item twice",
            "post each item once, in the order the board shows them",
        );
    }
    // `sessions::agenda` reads `ORDER BY created_at, item_id`, so the vector's
    // own order **is** the board order the human dragged against.
    let current: Vec<Uuid> = items.iter().map(|i| i.id).collect();
    let promoted = moved_earlier(&current, &movable);
    for item in &promoted {
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
        Json(json!({
            "ok": true,
            "prioritised": promoted.len(),
            // What the body asked about, so a client that posted nine cards and
            // moved none can tell that from a body the route threw away.
            "considered": movable.len(),
        })),
    )
        .into_response()
}

/// The submitted ids the human actually dragged **up**, in submitted order.
///
/// Both ranks are taken over the same set — the submitted ids in the board's
/// current order, against the body's order — so posting a subset compares like
/// with like instead of reading "nine of eleven" as "everything moved up two".
///
/// An id the board does not hold has no old position and so cannot have moved;
/// it is dropped rather than treated as new, which is what makes posting an
/// unchanged board mark nothing.
///
/// # Displacement is not a decision
///
/// Ranking alone is not enough. Dragging the top card to the bottom moves every
/// other card up one place, and marking those would mean a single
/// *de*-prioritising drag handing the `user` bypass to the rest of the board —
/// the same failure as marking the whole body, arrived at from the other
/// direction. So the cards that are already in board order relative to each
/// other are subtracted first ([`already_in_order`]): what is left is the
/// smallest set of moves that explains the submitted order, and of those only
/// the ones that ended up earlier are a priority.
///
/// The result is always a subset of "everything that ranks earlier", never a
/// superset, so this can only ever hand out fewer bypasses.
fn moved_earlier(current: &[Uuid], submitted: &[Uuid]) -> Vec<Uuid> {
    let baseline: Vec<Uuid> = current
        .iter()
        .copied()
        .filter(|id| submitted.contains(id))
        .collect();
    let ranks: Vec<Option<usize>> = submitted
        .iter()
        .map(|id| baseline.iter().position(|b| b == id))
        .collect();
    let stayed = already_in_order(&ranks);
    submitted
        .iter()
        .copied()
        .enumerate()
        .filter(|(now, _)| !stayed.contains(now))
        .filter(|(now, _)| ranks[*now].is_some_and(|before| *now < before))
        .map(|(_, id)| id)
        .collect()
}

/// The longest run of submitted positions that is already in board order.
///
/// A longest strictly-increasing subsequence of the positions, by patience
/// sorting with predecessor links. These are the cards that did not have to move
/// for the body to read the way it does; every other card is one somebody
/// dragged. `None` — an id the board does not hold — is never part of the run,
/// because a card that was nowhere cannot have stayed where it was.
fn already_in_order(ranks: &[Option<usize>]) -> std::collections::BTreeSet<usize> {
    let mut tails: Vec<usize> = Vec::new();
    let mut prev: Vec<Option<usize>> = vec![None; ranks.len()];
    for (i, rank) in ranks.iter().enumerate() {
        let Some(rank) = *rank else { continue };
        let at = tails.partition_point(|&t| {
            ranks[t].expect("only ranked positions are ever pushed") < rank
        });
        prev[i] = at.checked_sub(1).map(|p| tails[p]);
        if at == tails.len() {
            tails.push(i);
        } else {
            tails[at] = i;
        }
    }
    let mut run = std::collections::BTreeSet::new();
    let mut cursor = tails.last().copied();
    while let Some(i) = cursor {
        run.insert(i);
        cursor = prev[i];
    }
    run
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

/// What one amendment changed, field by field.
///
/// The charter column is overwritten in place, so before this the only record an
/// amendment left was its version number — and a Charter tab that can report
/// "version 4" and nothing else is telling the operator that three decisions were
/// taken and none of them can be read. The diff is computed here, where both
/// documents are in hand, and written into the append-only event payload; the
/// column stays the one copy of the *current* charter, and this is the history,
/// which lived nowhere.
///
/// Top-level fields only. A charter's fields are the things a human amends —
/// the mandate, the constraints, the exploration share — and a recursive diff
/// would report a path rather than a decision.
///
/// An amendment that changed nothing returns an empty object, which is not the
/// same as the `null` an amendment recorded before this existed serves: one says
/// "nothing moved", the other says "nobody wrote down what moved".
fn charter_changes(previous: Option<&Charter>, next: &Charter) -> Value {
    let before = previous.map_or_else(|| json!({}), |c| json!(c));
    let after = json!(next);
    let mut changed = serde_json::Map::new();
    let (Some(before), Some(after)) = (before.as_object(), after.as_object()) else {
        return Value::Object(changed);
    };
    for key in before.keys().chain(after.keys()) {
        if changed.contains_key(key.as_str()) {
            continue;
        }
        let from = before.get(key).unwrap_or(&Value::Null);
        let to = after.get(key).unwrap_or(&Value::Null);
        if from != to {
            changed.insert(key.clone(), json!({ "from": from, "to": to }));
        }
    }
    Value::Object(changed)
}

/// One `charter_amended` event, as the Charter tab reads it.
///
/// `by` and `changed` are `null` for an amendment appended before either was
/// recorded. Serving `{}` for `changed` there would say the amendment changed
/// nothing, which is a claim about a decision nobody wrote down.
fn amendment_json(seq: i64, payload: &Value, at: DateTime<Utc>) -> Value {
    json!({
        "seq": seq,
        "version": payload.get("version").and_then(Value::as_i64),
        "by": payload.get("by").and_then(Value::as_str),
        "changed": payload.get("changed").filter(|c| c.is_object()),
        "at": at,
    })
}

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

    // Computed against the charter this call is about to overwrite, and before
    // the overwrite: afterwards there is nothing left to compare against.
    let changed = charter_changes(session.charter.as_ref(), &next);

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
        json!({ "version": next.version, "by": token.user_id(), "changed": changed }),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "version": next.version })),
    )
        .into_response()
}

// ── digests ─────────────────────────────────────────────────────────────────

/// `(digest_id, from_seq, to_seq, body, spend_unit, created_at)`.
///
/// Named because the column order and [`digest_json`]'s argument order have to
/// agree, and two `i64`s and two strings next to each other is a swap that
/// compiles.
type DigestRow = (Uuid, i64, i64, Value, String, DateTime<Utc>);

/// One digest, as the desk reads it.
///
/// `spend_unit` is a field on the wire rather than a convention because
/// `body->budget->spent` is a bare number rendered beside a day budget
/// denominated in **dollars** (`charter::SessionBudget.usd_per_day`), while the
/// only per-run cost this platform records is **trials**. `0139` made the column
/// NOT NULL with no default and CHECKed it to the two units that exist, so every
/// row already knows which it counted; the read dropped it, and a card that
/// prints "3.00 spent" without it is telling the reader a number nobody measured.
///
/// It is never `null`: the column is NOT NULL, so a digest that exists has a unit.
fn digest_json(
    digest_id: Uuid,
    from_seq: i64,
    to_seq: i64,
    body: Value,
    spend_unit: String,
    created_at: DateTime<Utc>,
) -> Value {
    json!({
        "digest_id": digest_id,
        "from_seq": from_seq,
        "to_seq": to_seq,
        "body": body,
        "spend_unit": spend_unit,
        "created_at": created_at,
    })
}

/// `GET …/{id}/digests`
pub async fn digests(
    State(state): State<AppState>,
    token: BearerToken,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(r) = owned(&state, token.user_id(), id).await {
        return r;
    }
    let rows: Result<Vec<DigestRow>, _> = sqlx::query_as(
        "SELECT digest_id, from_seq, to_seq, body, spend_unit, created_at FROM session_digests
          WHERE session_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(id)
    .fetch_all(&state.pg)
    .await;
    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "digests": rows
                    .into_iter()
                    .map(|(d, f, t, body, unit, at)| digest_json(d, f, t, body, unit, at))
                    .collect::<Vec<_>>()
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

    /// Four distinct ids, stable within a test.
    fn ids(n: usize) -> Vec<Uuid> {
        (1..=n)
            .map(|i| {
                let mut b = [0u8; 16];
                b[15] = u8::try_from(i).expect("small");
                Uuid::from_bytes(b)
            })
            .collect()
    }

    #[test]
    fn posting_the_board_unchanged_prioritises_nothing() {
        // The case a drag-and-drop client produces most often, and the one the
        // route got exactly backwards: it marked every id in the body `user`,
        // and a `user` item bypasses scoring outright. One drag of one card
        // therefore switched the scorer off for the whole session — silently,
        // permanently, and with the exploration floor going with it.
        let board = ids(4);
        assert_eq!(
            moved_earlier(&board, &board),
            Vec::<Uuid>::new(),
            "an unchanged board disabled the selection policy for the session"
        );
    }

    #[test]
    fn only_the_card_that_went_up_is_marked() {
        let board = ids(4);
        // The human drags the fourth card to the front. Everything else shifts
        // down by one — and a card that shifted down did not get prioritised.
        let dragged = vec![board[3], board[0], board[1], board[2]];
        assert_eq!(moved_earlier(&board, &dragged), vec![board[3]]);
    }

    #[test]
    fn a_card_dragged_down_prioritises_nothing_at_all() {
        let board = ids(4);
        // First card to the back. Three cards rank one place earlier than they
        // did — and nobody dragged any of them. Handing those three the `user`
        // bypass is the same failure as marking the whole body, reached from the
        // other direction: one de-prioritising drag, three-quarters of the board
        // out of the scorer's hands.
        let dragged = vec![board[1], board[2], board[3], board[0]];
        assert_eq!(
            moved_earlier(&board, &dragged),
            Vec::<Uuid>::new(),
            "displacement was read as a decision about the cards that were displaced"
        );
    }

    #[test]
    fn one_card_lifted_past_one_other_is_the_only_one_marked() {
        let board = ids(4);
        let dragged = vec![board[1], board[0], board[2], board[3]];
        assert_eq!(moved_earlier(&board, &dragged), vec![board[1]]);
    }

    #[test]
    fn the_run_that_did_not_move_is_the_longest_one() {
        // The subtraction the marking rests on. `[1, 2, 3, 0]` is three cards in
        // board order with one dropped behind them, not three cards that moved.
        let ranks = [Some(1), Some(2), Some(3), Some(0)];
        assert_eq!(
            already_in_order(&ranks),
            [0, 1, 2].into_iter().collect::<std::collections::BTreeSet<_>>()
        );
        // An id the board does not hold was never in place, so it is never in
        // the run — and a run computed over nothing would be silently empty.
        let unknown = [None, Some(0)];
        assert_eq!(
            already_in_order(&unknown),
            [1].into_iter().collect::<std::collections::BTreeSet<_>>()
        );
    }

    #[test]
    fn a_partial_board_is_ranked_against_itself() {
        // A client that posts only some cards must not have them read as though
        // they had jumped over the ones it left out.
        let board = ids(4);
        let posted = vec![board[1], board[2]];
        assert_eq!(
            moved_earlier(&board, &posted),
            Vec::<Uuid>::new(),
            "a subset in its existing relative order was read as a promotion"
        );
        assert_eq!(moved_earlier(&board, &[board[2], board[1]]), vec![board[2]]);
    }

    #[test]
    fn a_body_of_one_is_refused_rather_than_answered_with_nothing() {
        // A single id asserts no order, so no rank rule can find a move in it —
        // and a client that posts only the card it dragged would get a 200 for a
        // request that changed nothing. The route refuses it and names the two
        // ways to say what was meant.
        let src = include_str!("research_sessions.rs");
        let body = src
            .split_once("pub async fn reorder(")
            .expect("reorder is gone; this test is stale")
            .1;
        let body = &body[..body.find("\nfn moved_earlier").unwrap_or(body.len())];
        assert!(
            body.contains("movable.len() < 2"),
            "a one-item order is accepted again; it can only ever prioritise nothing"
        );
        let refusal = body
            .split_once("movable.len() < 2")
            .expect("checked above")
            .1;
        let refusal = &refusal[..refusal.find("\n    }").unwrap_or(refusal.len())];
        assert!(
            refusal.contains("prioritise"),
            "the refusal does not name the route that does what the caller meant"
        );
    }

    #[test]
    fn an_id_the_board_does_not_hold_cannot_have_moved() {
        let board = ids(3);
        let stranger = ids(9)[8];
        assert_eq!(moved_earlier(&board, &[stranger]), Vec::<Uuid>::new());
    }

    #[test]
    fn the_route_marks_what_moved_and_not_what_was_posted() {
        // The property, where it is enforced: `reorder` writes the `user` source
        // for `moved_earlier`'s answer, never for the body.
        let src = include_str!("research_sessions.rs");
        let body = src
            .split_once("pub async fn reorder(")
            .expect("reorder is gone; this test is stale")
            .1;
        let body = &body[..body.find("\nfn moved_earlier").unwrap_or(body.len())];
        assert!(
            body.contains("moved_earlier(&current, &movable)"),
            "reorder stopped comparing the submitted order against the board's"
        );
        let loop_start = body
            .find("for item in &")
            .expect("the write loop is gone; this test is stale");
        assert!(
            body[loop_start..].starts_with("for item in &promoted"),
            "reorder writes the `user` source over the whole submitted body again; one drag of \
             one card turns the scorer off for the session"
        );
    }

    #[test]
    fn a_digest_says_which_unit_it_counted_in() {
        // `body->budget->spent` is a bare number rendered beside a dollar day
        // budget, and the only per-run cost this platform records is trials.
        // Migration 0139 made the column NOT NULL with no default precisely so
        // the unit could not be left to the reader; dropping it on the way out
        // puts it back.
        let row = digest_json(
            Uuid::nil(),
            0,
            7,
            json!({ "budget": { "spent": 3.0 } }),
            "trials".into(),
            Utc::now(),
        );
        assert_eq!(row.get("spend_unit").and_then(Value::as_str), Some("trials"));
        // And the column is actually asked for: a JSON key built from a SELECT
        // that does not name it would not compile, but a SELECT that names it
        // and a key that does not is exactly the shape of the original bug.
        let src = include_str!("research_sessions.rs");
        let q = src
            .split_once("FROM session_digests")
            .expect("the digests query is gone; this test is stale")
            .0;
        let select = &q[q.rfind("SELECT").expect("no SELECT before the FROM")..];
        assert!(
            select.contains("spend_unit"),
            "the digests query stopped selecting the unit, so the card is back to rendering \
             trials where the reader expects dollars"
        );
    }

    fn a_charter(exclusions: Vec<String>) -> Charter {
        Charter {
            scope: research_session::charter::Scope::default(),
            objective: research_session::charter::Objective::BuildKnowledge { about: vec![] },
            seeds: vec![],
            permitted_workflows: vec![],
            exclusions,
            standing_constraints: vec![],
            exploration_share: research_session::charter::ExplorationShare::default(),
            stop_when: vec![],
            delta_practical_default: None,
            version: 1,
        }
    }

    #[test]
    fn an_amendment_records_which_fields_moved() {
        let before = a_charter(vec![]);
        let mut after = a_charter(vec!["optimise".into()]);
        after.version = 2;

        let changed = charter_changes(Some(&before), &after);
        let obj = changed.as_object().expect("a diff is an object");
        // Two fields moved and no others: the exclusion the human added, and the
        // version the route bumps. A diff that reported every field would be a
        // second copy of the charter wearing a diff's name.
        assert!(obj.contains_key("exclusions"), "the change itself is missing");
        assert!(obj.contains_key("version"));
        assert_eq!(obj.len(), 2, "unchanged fields were reported as changes: {obj:?}");
        assert_eq!(
            obj["exclusions"]["from"],
            json!([]),
            "the diff does not say what it was, so nobody can read what changed"
        );
        assert_eq!(obj["exclusions"]["to"], json!(["optimise"]));
    }

    #[test]
    fn an_amendment_that_changed_nothing_is_not_an_amendment_nobody_recorded() {
        // The two have to be distinguishable. `{}` is "nothing moved"; `null` is
        // "this was appended before anyone wrote down what moved", which is
        // every amendment already in the log.
        let same = charter_changes(Some(&a_charter(vec![])), &a_charter(vec![]));
        assert_eq!(same, json!({}));

        let old = amendment_json(4, &json!({ "version": 3 }), Utc::now());
        assert_eq!(old["changed"], Value::Null, "an unrecorded diff read as an empty one");
        assert_eq!(old["by"], Value::Null);
        assert_eq!(old["version"], json!(3));

        let new = amendment_json(
            5,
            &json!({ "version": 4, "by": "u", "changed": { "version": { "from": 3, "to": 4 } } }),
            Utc::now(),
        );
        assert_eq!(new["changed"]["version"]["to"], json!(4));
        assert_eq!(new["by"], json!("u"));
    }

    #[test]
    fn a_session_with_no_previous_charter_still_reports_what_was_set() {
        // A bounded session has no charter, so the first amendment's `from` side
        // is genuinely absent. Reporting no changes there would say the charter
        // arrived from nowhere.
        let changed = charter_changes(None, &a_charter(vec!["optimise".into()]));
        let obj = changed.as_object().expect("a diff is an object");
        assert!(!obj.is_empty(), "the first charter registered as no change at all");
        assert_eq!(obj["exclusions"]["from"], Value::Null);
        assert_eq!(obj["exclusions"]["to"], json!(["optimise"]));
    }

    #[test]
    fn the_amendments_are_read_back_by_the_route_that_reports_the_version() {
        // The defect: `PUT …/charter` appended amendments and nothing read them,
        // so the Charter tab could report "version 4" and not what the other
        // three were.
        let src = include_str!("research_sessions.rs");
        let get = src
            .split_once("pub async fn get(")
            .expect("the session read is gone; this test is stale")
            .1;
        let get = &get[..get.find("\npub ").unwrap_or(get.len())];
        assert!(
            get.contains("\"amendments\"") && get.contains("CharterAmended"),
            "the session read reports charter_version without the amendments behind it"
        );
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
