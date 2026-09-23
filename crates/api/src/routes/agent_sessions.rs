//! The orchestrator's HTTP surface (AGENT-001 §16, §18; COMP-006 §3, §4).
//!
//! What the workspace UI reads and writes: sessions, the event timeline, steering,
//! approvals, and usage.
//!
//! **The timeline is SSE over a durable table, not a broadcast channel.** Events go
//! to `agent_events` first and are read back from there, so a reconnecting browser
//! resumes from `Last-Event-ID` and sees everything it missed. A broadcast channel
//! is simpler and loses the session's history the moment a tab sleeps, a laptop
//! closes, or a web node restarts — and the timeline is the only record a human has
//! of what the agent did.
//!
//! **Steering goes through a table too**, for the same reason turned around: the
//! thing being steered is a container the API does not share memory with. A steering
//! message that vanished with an API restart would have been typed by a user,
//! acknowledged by the UI, and never seen by the agent.

use std::{convert::Infallible, time::Duration};

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
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// How often the SSE stream looks for new events.
///
/// Polling rather than LISTEN/NOTIFY: the events are already in a table that has to
/// be read on reconnect anyway, so one query path serves both, and a 500 ms poll on
/// an indexed `(session_id, id)` lookup is cheaper than the bug where a notification
/// arrives before the row it refers to is committed.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Events per poll. Caps a reconnect that has thousands of rows to catch up on, so
/// the first response arrives quickly instead of after the whole backlog.
const POLL_BATCH: i64 = 200;

fn internal(e: impl std::fmt::Display) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal", "message": e.to_string()})),
    )
        .into_response()
}

/// Confirms the caller owns the session, and reports "not found" if not.
///
/// Not "forbidden": a caller should not be able to discover which session ids exist
/// by the difference between the two answers.
async fn owned_session(
    state: &AppState,
    user_id: Uuid,
    session_id: Uuid,
) -> Result<(Uuid, Uuid), axum::response::Response> {
    let row: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT project_id, user_id FROM agent_sessions WHERE session_id = $1")
            .bind(session_id)
            .fetch_optional(&state.pg)
            .await
            .map_err(internal)?;

    match row {
        Some((project_id, owner)) if owner == user_id => Ok((project_id, owner)),
        _ => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "not_found", "session_id": session_id})),
        )
            .into_response()),
    }
}

// ── Sessions ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListSessionsQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// GET /api/agent/sessions
pub async fn list_sessions(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<ListSessionsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).clamp(1, 500);
    let rows: Result<Vec<SessionRow>, _> = sqlx::query_as(
        "SELECT session_id, project_id, state, is_initializer, spend_usd::float8, \
                started_at, ended_at, created_at, report_id, abort_reason \
           FROM agent_sessions \
          WHERE user_id = $1 AND ($2::uuid IS NULL OR project_id = $2) \
          ORDER BY created_at DESC LIMIT $3",
    )
    .bind(token.user_id())
    .bind(q.project_id)
    .bind(limit)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({"sessions": rows.iter().map(session_json).collect::<Vec<_>>()})),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

type SessionRow = (
    Uuid,
    Uuid,
    String,
    bool,
    f64,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    Option<String>,
    Option<String>,
);

fn session_json(r: &SessionRow) -> Value {
    json!({
        "session_id": r.0,
        "project_id": r.1,
        "state": r.2,
        "is_initializer": r.3,
        "spend_usd": r.4,
        "started_at": r.5,
        "ended_at": r.6,
        "created_at": r.7,
        "report_id": r.8,
        "abort_reason": r.9,
    })
}

/// GET /api/agent/sessions/{id}
pub async fn get_session(
    State(state): State<AppState>,
    token: BearerToken,
    Path(session_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(resp) = owned_session(&state, token.user_id(), session_id).await {
        return resp;
    }
    let row: Result<Option<SessionRow>, _> = sqlx::query_as(
        "SELECT session_id, project_id, state, is_initializer, spend_usd::float8, \
                started_at, ended_at, created_at, report_id, abort_reason \
           FROM agent_sessions WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_optional(&state.pg)
    .await;
    match row {
        Ok(Some(r)) => (StatusCode::OK, Json(session_json(&r))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response(),
        Err(e) => internal(e),
    }
}

// ── The timeline (UI-02) ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    /// Resume point. `Last-Event-ID` takes precedence when the browser sends it,
    /// which it does automatically on a dropped connection.
    #[serde(default)]
    pub after_id: Option<i64>,
}

/// GET /api/agent/sessions/{id}/events — the SSE timeline (UI-02, AGENT-001 §18).
///
/// Each event carries its `agent_events.id` as the SSE id, so a reconnect resumes
/// exactly where it stopped. That is the whole reason the ids are exposed: an SSE
/// stream without them silently drops whatever happened while the tab was asleep,
/// and the timeline is the only record a human has of what the agent did.
pub async fn session_events(
    State(state): State<AppState>,
    token: BearerToken,
    Path(session_id): Path<Uuid>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<
    Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>,
    axum::response::Response,
> {
    owned_session(&state, token.user_id(), session_id).await?;

    // The browser's own resume header wins over the query parameter: it is what the
    // browser actually last received, and a stale client-side cursor would silently
    // replay or skip.
    let resume = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<i64>().ok())
        .or(q.after_id)
        .unwrap_or(0);

    let pool = state.pg.clone();
    // A bounded channel, filled by a task that stops the moment the receiver is
    // dropped. That is what makes a closed browser tab stop querying the database:
    // an unbounded channel with no backpressure would keep polling for a reader that
    // no longer exists.
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);
    tokio::spawn(async move {
        let mut cursor = resume;
        let mut first = true;
        loop {
            if !first {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            first = false;

            let rows: Vec<(i64, i32, String, Value, DateTime<Utc>)> = match sqlx::query_as(
                "SELECT id, seq, kind, payload, created_at FROM agent_events \
                  WHERE session_id = $1 AND id > $2 ORDER BY id LIMIT $3",
            )
            .bind(session_id)
            .bind(cursor)
            .bind(POLL_BATCH)
            .fetch_all(&pool)
            .await
            {
                Ok(rows) => rows,
                Err(e) => {
                    // Tell the client rather than going quiet. A timeline that
                    // silently stops updating reads as "the agent is thinking".
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("stream_error")
                            .data(json!({ "message": e.to_string() }).to_string())))
                        .await;
                    return;
                }
            };

            for (id, seq, kind, payload, created_at) in rows {
                cursor = id;
                let data = json!({
                    "id": id,
                    "seq": seq,
                    "kind": kind,
                    "payload": payload,
                    "created_at": created_at,
                });
                let event = Event::default()
                    .id(id.to_string())
                    .event(kind)
                    .data(data.to_string());
                if tx.send(Ok(event)).await.is_err() {
                    return; // the client went away
                }
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            // A named comment rather than a bare colon: proxies that buffer SSE are
            // much easier to diagnose when the keep-alive says what it is.
            .text("keep-alive"),
    ))
}

// ── Steering (UI-03, AGENT-001 §16) ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SteerRequest {
    /// `steer`, `interrupt`, `stop` or `answer`.
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    /// For `answer`: the approval being answered and the option chosen.
    #[serde(default)]
    pub approval_id: Option<Uuid>,
    #[serde(default)]
    pub option: Option<String>,
}

const INBOX_KINDS: &[&str] = &["steer", "interrupt", "stop", "answer"];

/// States in which an instruction can still reach the agent.
///
/// Queueing steering for an ended session is not harmless: the UI would accept the
/// message and nothing would ever read it, which looks exactly like the agent
/// ignoring the user.
const STEERABLE_STATES: &[&str] = &["starting", "running", "waiting", "compacting"];

/// POST /api/agent/sessions/{id}/steer
pub async fn steer_session(
    State(state): State<AppState>,
    token: BearerToken,
    Path(session_id): Path<Uuid>,
    Json(req): Json<SteerRequest>,
) -> impl IntoResponse {
    if let Err(resp) = owned_session(&state, token.user_id(), session_id).await {
        return resp;
    }
    if !INBOX_KINDS.contains(&req.kind.as_str()) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "unknown_kind",
                "message": format!("unknown steering kind {:?}", req.kind),
                "known": INBOX_KINDS,
            })),
        )
            .into_response();
    }
    if req.kind == "steer" && req.text.as_deref().unwrap_or("").trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "empty_steer",
                "message": "a steering message needs text",
                "fix": "send `kind: \"interrupt\"` to stop the current turn without saying anything",
            })),
        )
            .into_response();
    }

    let session_state: Option<String> =
        sqlx::query_scalar("SELECT state FROM agent_sessions WHERE session_id = $1")
            .bind(session_id)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();
    let Some(session_state) = session_state else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response();
    };
    if !STEERABLE_STATES.contains(&session_state.as_str()) {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "session_not_steerable",
                "message": format!("this session is {session_state}"),
                "fix": "start a new session; an ended one has nothing left to read the message",
            })),
        )
            .into_response();
    }

    let body = json!({
        "text": req.text,
        "approval_id": req.approval_id,
        "option": req.option,
    });

    let id: Result<i64, _> = sqlx::query_scalar(
        "INSERT INTO session_inbox (session_id, kind, body, sent_by) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(session_id)
    .bind(&req.kind)
    .bind(&body)
    .bind(token.user_id())
    .fetch_one(&state.pg)
    .await;

    match id {
        Ok(id) => (
            StatusCode::ACCEPTED,
            Json(json!({"queued": id, "kind": req.kind})),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

/// GET /api/agent/sessions/{id}/inbox — what the agent-host drains.
///
/// Marks what it returns as delivered in the same statement. Read-then-mark would
/// let a host crash between the two and re-read an instruction the agent has already
/// acted on, which is worse than losing one.
pub async fn drain_inbox(
    State(state): State<AppState>,
    token: BearerToken,
    Path(session_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(resp) = owned_session(&state, token.user_id(), session_id).await {
        return resp;
    }
    let rows: Result<Vec<(i64, String, Value)>, _> = sqlx::query_as(
        "UPDATE session_inbox SET delivered_at = now() \
          WHERE id IN ( \
            SELECT id FROM session_inbox \
             WHERE session_id = $1 AND delivered_at IS NULL \
             ORDER BY id FOR UPDATE SKIP LOCKED ) \
          RETURNING id, kind, body",
    )
    .bind(session_id)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "messages": rows.iter().map(|(id, kind, body)| json!({
                    "id": id, "kind": kind, "body": body,
                })).collect::<Vec<_>>(),
            })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

// ── Approvals inbox (UI-05, COMP-006 §4) ─────────────────────────────────────

/// Every approval kind the inbox covers (AGENT-001 §5).
pub const APPROVAL_KINDS: &[&str] = &[
    "ask_user",
    // The policy engine paused a tool call (ADR-0032). One kind, not one per policy
    // code: the code varies with the rule that fired and belongs in the payload,
    // while the kind is what the UI groups and filters on.
    "tool_call",
    "plan",
    "budget",
    "qc_waiver",
    "model_promotion",
    "skill_promotion",
    "paper_deployment",
];

#[derive(Debug, Deserialize)]
pub struct ApprovalsQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub state: Option<String>,
}

/// GET /api/approvals — the global inbox.
pub async fn list_approvals(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<ApprovalsQuery>,
) -> impl IntoResponse {
    let want = q.state.unwrap_or_else(|| "pending".into());
    // An approval belongs to a research project OR to an agent run (migration 0040),
    // so both joins are LEFT and ownership comes from whichever side is present.
    //
    // The inner join this replaced made every run-scoped approval invisible, which is
    // not a display bug: the GOVERNOR loop suspends in `AwaitingApproval` until someone
    // answers, so an approval nobody can see is a run that never finishes.
    let rows: Result<Vec<ApprovalRow>, _> = sqlx::query_as(
        "SELECT a.approval_id, a.project_id, a.session_id, a.kind, a.payload, a.options, \
                a.default_option, a.timeout_at, a.state, a.answer, a.answered_by, \
                a.answered_at, a.created_at, a.run_id \
           FROM approval_requests a \
           LEFT JOIN research_projects p ON p.project_id = a.project_id \
           LEFT JOIN agent_runs r ON r.run_id = a.run_id \
          WHERE COALESCE(p.user_id, r.user_id) = $1 \
            AND ($2::uuid IS NULL OR a.project_id = $2) \
            AND ($3 = 'all' OR a.state = $3) \
          ORDER BY a.created_at DESC LIMIT 200",
    )
    .bind(token.user_id())
    .bind(q.project_id)
    .bind(&want)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "approvals": rows.iter().map(approval_json).collect::<Vec<_>>(),
                "kinds": APPROVAL_KINDS,
            })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

type ApprovalRow = (
    Uuid,
    Option<Uuid>,
    Option<Uuid>,
    String,
    Value,
    Option<Value>,
    Option<String>,
    Option<DateTime<Utc>>,
    String,
    Option<Value>,
    Option<Uuid>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    Option<Uuid>,
);

fn approval_json(r: &ApprovalRow) -> Value {
    json!({
        "approval_id": r.0,
        "project_id": r.1,
        "session_id": r.2,
        "kind": r.3,
        "payload": r.4,
        "options": r.5,
        "default_option": r.6,
        "timeout_at": r.7,
        "state": r.8,
        "answer": r.9,
        "answered_by": r.10,
        "answered_at": r.11,
        "created_at": r.12,
        "run_id": r.13,
    })
}

/// `(project_id, owner, state, kind, options)` for an approval being answered.
/// `project_id` is `None` for a run-scoped approval, whose owner is the run's.
type ApprovalOwnerRow = (Option<Uuid>, Option<Uuid>, String, String, Option<Value>);

#[derive(Debug, Deserialize)]
pub struct AnswerRequest {
    pub option: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// POST /api/approvals/{id}/answer
///
/// Records who answered and when, and refuses to answer twice. The second answer is
/// the interesting case: an approval that could be re-answered is one where a
/// decision can be changed after the agent has already acted on it, and the audit
/// record would show only the second.
pub async fn answer_approval(
    State(state): State<AppState>,
    token: BearerToken,
    Path(approval_id): Path<Uuid>,
    Json(req): Json<AnswerRequest>,
) -> impl IntoResponse {
    let row: Result<Option<ApprovalOwnerRow>, _> = sqlx::query_as(
        "SELECT a.project_id, COALESCE(p.user_id, r.user_id), a.state, a.kind, a.options \
           FROM approval_requests a \
           LEFT JOIN research_projects p ON p.project_id = a.project_id \
           LEFT JOIN agent_runs r ON r.run_id = a.run_id \
          WHERE a.approval_id = $1",
    )
    .bind(approval_id)
    .fetch_optional(&state.pg)
    .await;

    let found = match row {
        Ok(v) => v,
        Err(e) => return internal(e),
    };
    let Some((_project_id, owner, current_state, _kind, options)) = found else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response();
    };
    // 404 rather than 403: an approval belonging to someone else should not be
    // distinguishable from one that does not exist. `None` here means the row is
    // orphaned — its project or run is gone — and is treated the same way.
    if owner != Some(token.user_id()) {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response();
    }
    if current_state != "pending" {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "already_answered",
                "state": current_state,
                "message": "this approval has already been decided",
            })),
        )
            .into_response();
    }

    // The chosen option must be one that was offered. A free-text answer to a
    // multiple-choice approval is an instruction nobody designed a handler for.
    if let Some(Value::Array(opts)) = options {
        let offered: Vec<String> = opts
            .iter()
            .filter_map(|o| {
                o.as_str()
                    .map(str::to_string)
                    .or_else(|| o.get("id").and_then(Value::as_str).map(str::to_string))
            })
            .collect();
        if !offered.is_empty() && !offered.contains(&req.option) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "unknown_option",
                    "message": format!("{:?} was not offered", req.option),
                    "options": offered,
                })),
            )
                .into_response();
        }
    }

    let answer = json!({ "option": req.option, "note": req.note });
    let updated: Result<Option<Uuid>, _> = sqlx::query_scalar(
        "UPDATE approval_requests \
            SET state = 'answered', answer = $2, answered_by = $3, answered_at = now() \
          WHERE approval_id = $1 AND state = 'pending' \
          RETURNING approval_id",
    )
    .bind(approval_id)
    .bind(&answer)
    .bind(token.user_id())
    .fetch_optional(&state.pg)
    .await;

    match updated {
        Ok(Some(_)) => (
            StatusCode::OK,
            Json(json!({
                "approval_id": approval_id,
                "state": "answered",
                "answer": answer,
                "answered_by": token.user_id(),
            })),
        )
            .into_response(),
        // Lost the race against another answer or a timeout default.
        Ok(None) => (
            StatusCode::CONFLICT,
            Json(json!({"error": "already_answered"})),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

// ── Usage and budget (UI-07) ─────────────────────────────────────────────────

/// What `llm_usage` recorded for a scope.
///
/// `calls` is the field that makes the other five readable. Four zero counters with
/// `calls = 0` is a table holding no row for this caller; four zero counters with
/// `calls = 12` is twelve calls that really did move no tokens. The response used to
/// carry the counters without the count, so the two printed identically — and on this
/// deployment it is always the first, because `llm_usage` has no rows at all.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PricedUsage {
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// `None` when no row was summed. `SUM` over no rows is not zero dollars, and the
    /// `COALESCE(SUM(cost_usd),0)` this replaces reported an empty table as free.
    pub cost_usd: Option<f64>,
}

/// What `agent_runs` recorded for a scope: per-run totals, never priced.
///
/// A separate type from [`PricedUsage`] because they are separate measurements with
/// separate coverage, and a reader who cannot tell them apart is reading a number
/// whose denominator they do not know.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RunTokenVolume {
    /// Runs in scope, whatever they recorded.
    pub runs: i64,
    /// Runs that recorded a token count. The denominator of the two sums below.
    pub runs_with_tokens: i64,
    /// Summed over `runs_with_tokens` only. `None` when that set is empty — an
    /// absence, not a zero.
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
}

impl RunTokenVolume {
    /// Runs in scope that recorded nothing. Not runs that spent nothing.
    pub fn runs_without_tokens(self) -> i64 {
        self.runs - self.runs_with_tokens
    }
}

/// Why no dollar figure appears, stated in the response rather than in a comment.
///
/// A reader who sees a missing field and no reason will assume the field broke.
pub const COST_NOT_RECORDED: &str = "not recorded. The only priced table is llm_usage, \
    whose cost_usd is written per LLM call by the API LLM proxy and keys on an auth \
    session, not on a run — so it cannot price an agent run even when it has rows. \
    agent_runs records token counts and no price at all, and this platform has no price \
    table to multiply them by. Absent, not zero, and deliberately not estimated.";

/// What the four `llm_usage` counters are measuring, and what they systematically miss.
pub const PRICED_SOURCE: &str = "llm_usage — one row per LLM call, priced. Its only \
    writer is the API LLM proxy, and the agent driver does not call through the proxy, \
    so runs driven by it are absent here rather than recorded as zero.";

/// What the run counters are measuring.
pub const RUN_SOURCE: &str = "agent_runs — one row per agent run, tokens_in/tokens_out \
    summed over the runs that recorded a count. Unpriced. Coverage is runs_with_tokens \
    of runs_total; the rest recorded nothing and are excluded from the sums rather than \
    summed in as zeros.";

/// Why a run whose counters are both zero is read as unmeasured.
///
/// `agent_runs.tokens_in` and `tokens_out` are `BIGINT NOT NULL DEFAULT 0` (migration
/// 0035), so the column cannot distinguish a run measured at zero from a run nothing
/// ever wrote to. That ambiguity is in the schema, not in this read, and the response
/// says so rather than resolving it silently.
pub const RUN_ZERO_IS_AMBIGUOUS: &str = "agent_runs.tokens_in and tokens_out are NOT \
    NULL DEFAULT 0, so a run holding 0/0 may have been measured at zero or never \
    written to. Such runs are counted in runs_without_tokens and excluded from the \
    sums. Telling the two apart needs a nullable column, not a different query.";

/// Why the run counters are withheld when a project is asked for.
pub const RUN_TOKENS_NOT_PROJECT_SCOPED: &str = "withheld. agent_runs has no project_id \
    column, so it cannot be narrowed to one project. Returning the unscoped totals here \
    would attribute every run on the deployment to the project that was asked for.";

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    #[serde(default)]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    pub session_id: Option<Uuid>,
}

/// The response body, built from the two sources without merging them.
///
/// Split out from the handler so the shaping can be tested without a database: every
/// rule worth defending here is about which fields are absent, and an absence is the
/// one thing an integration test against a populated database cannot show you.
pub fn usage_body(priced: PricedUsage, runs: Option<RunTokenVolume>, verdicts: i64) -> Value {
    let billable_prompt = priced.input_tokens + priced.cache_read;
    let mut body = json!({
        // llm_usage: per call, priced.
        "input_tokens": priced.input_tokens,
        "output_tokens": priced.output_tokens,
        "cache_read": priced.cache_read,
        "cache_write": priced.cache_write,
        "llm_usage_calls": priced.calls,
        "llm_usage_source": PRICED_SOURCE,
        "cost_usd": priced.cost_usd,
        "cost_usd_note": COST_NOT_RECORDED,
        // A rate over nothing billable is not a rate of zero. It was 0.0, which is a
        // measurement, and the only cache hit rate this deployment can honestly
        // report is that nobody has one.
        "cache_hit_rate": if billable_prompt > 0 {
            Some(priced.cache_read as f64 / billable_prompt as f64)
        } else {
            None
        },
        "verdicts": verdicts,
        // Arithmetic over two recorded figures, or nothing. Never a default.
        "dollars_per_verdict": match priced.cost_usd {
            Some(c) if verdicts > 0 => Some(c / verdicts as f64),
            _ => None,
        },
    });

    let obj = body.as_object_mut().expect("json! built an object");
    match runs {
        Some(r) => {
            obj.insert("runs_total".into(), json!(r.runs));
            obj.insert("runs_with_tokens".into(), json!(r.runs_with_tokens));
            obj.insert("runs_without_tokens".into(), json!(r.runs_without_tokens()));
            obj.insert("run_input_tokens".into(), json!(r.input_tokens));
            obj.insert("run_output_tokens".into(), json!(r.output_tokens));
            obj.insert("run_tokens_source".into(), json!(RUN_SOURCE));
            obj.insert("run_zero_note".into(), json!(RUN_ZERO_IS_AMBIGUOUS));
            // Unpriced is a property of the source, not of this scope, so it is said
            // whenever the run block is present.
            obj.insert("run_cost_usd".into(), Value::Null);
        }
        None => {
            obj.insert("run_tokens_source".into(), json!(RUN_SOURCE));
            obj.insert(
                "run_tokens_scope_note".into(),
                json!(RUN_TOKENS_NOT_PROJECT_SCOPED),
            );
        }
    }
    body
}

/// Reads `agent_runs` for one user, optionally narrowed to one session.
///
/// The `> 0` filter is the whole point: `SUM` over every row folds runs that recorded
/// nothing into the total as zeros, which is how an average over this table becomes a
/// number nobody measured.
pub async fn run_token_volume(
    pg: &sqlx::PgPool,
    user_id: Uuid,
    session_id: Option<Uuid>,
) -> Result<RunTokenVolume, sqlx::Error> {
    let (runs, runs_with_tokens, input_tokens, output_tokens): (
        i64,
        i64,
        Option<i64>,
        Option<i64>,
    ) = sqlx::query_as(
        "SELECT count(*)::bigint, \
                (count(*) FILTER (WHERE tokens_in > 0 OR tokens_out > 0))::bigint, \
                (SUM(tokens_in)  FILTER (WHERE tokens_in > 0 OR tokens_out > 0))::bigint, \
                (SUM(tokens_out) FILTER (WHERE tokens_in > 0 OR tokens_out > 0))::bigint \
           FROM agent_runs \
          WHERE user_id = $1 \
            AND ($2::uuid IS NULL OR session_id = $2)",
    )
    .bind(user_id)
    .bind(session_id)
    .fetch_one(pg)
    .await?;
    Ok(RunTokenVolume {
        runs,
        runs_with_tokens,
        input_tokens,
        output_tokens,
    })
}

/// Token volume, and what is known about its price, from both tables that hold any.
///
/// Cache hit rate is `cache_read / (cache_read + input)`, the fraction of prompt
/// tokens not re-billed at full price. It is on the budget pane because it is the
/// number that tells a reader whether the cost is the work or the overhead — and
/// because D-03 says capability is never traded for tokens, so the only honest way to
/// spend less is to waste less.
///
/// **Two sources, reported separately.** `llm_usage` is per-call and priced but only
/// the LLM proxy writes it; `agent_runs` is per-run and unpriced but is what the agent
/// driver actually fills. Summing them would produce one total whose coverage nobody
/// could state, so the response keeps them apart and names each.
pub async fn get_usage(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<UsageQuery>,
) -> impl IntoResponse {
    let priced: Result<(i64, i64, i64, i64, i64, Option<f64>), _> = sqlx::query_as(
        "SELECT count(*)::bigint, \
                COALESCE(SUM(input_tokens),0)::bigint, \
                COALESCE(SUM(output_tokens),0)::bigint, \
                COALESCE(SUM(cache_read),0)::bigint, \
                COALESCE(SUM(cache_write),0)::bigint, \
                SUM(cost_usd)::float8 \
           FROM llm_usage \
          WHERE user_id = $1 \
            AND ($2::uuid IS NULL OR project_id = $2) \
            AND ($3::uuid IS NULL OR session_id = $3)",
    )
    .bind(token.user_id())
    .bind(q.project_id)
    .bind(q.session_id)
    .fetch_one(&state.pg)
    .await;

    let priced = match priced {
        Ok((calls, input_tokens, output_tokens, cache_read, cache_write, cost_usd)) => {
            PricedUsage {
                calls,
                input_tokens,
                output_tokens,
                cache_read,
                cache_write,
                cost_usd,
            }
        }
        Err(e) => return internal(e),
    };

    // Withheld rather than answered wrong when a project was asked for: `agent_runs`
    // has no `project_id`, and `session_id` is a column it does have.
    let runs = if q.project_id.is_some() {
        None
    } else {
        match run_token_volume(&state.pg, token.user_id(), q.session_id).await {
            Ok(v) => Some(v),
            Err(e) => return internal(e),
        }
    };

    // Verdicts: sessions that filed a report. Dollars per verdict is the number that
    // makes an efficiency change arguable, and it is meaningless without it.
    let verdicts: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM agent_sessions \
          WHERE user_id = $1 AND report_id IS NOT NULL \
            AND ($2::uuid IS NULL OR project_id = $2)",
    )
    .bind(token.user_id())
    .bind(q.project_id)
    .fetch_one(&state.pg)
    .await
    .unwrap_or(0);

    (StatusCode::OK, Json(usage_body(priced, runs, verdicts))).into_response()
}

// ── Notebook pane (UI-04, COMP-006 §3) ───────────────────────────────────────

/// Files the Notebook pane may ask for.
///
/// An allow-list rather than a path sanitiser. The agent's workspace also holds
/// code, scratch data and whatever else it wrote; the Notebook pane wants three
/// named documents, and a reader that can fetch any path is a reader that can be
/// pointed at something it should not surface.
pub const WORKSPACE_FILES: &[&str] = &["NOTEBOOK.md", "RESEARCH_PLAN.json", "GIT_LOG.txt"];

#[derive(Debug, Deserialize)]
pub struct WorkspaceFileQuery {
    pub path: String,
}

/// GET /api/agent/projects/{id}/workspace/files?path=NOTEBOOK.md
///
/// Serves the newest `workspace_snapshot` artifact for the project rather than
/// reading the container's volume. The API has no docker socket and should not get
/// one — handing it the ability to read an agent's filesystem is the same capability
/// `only_the_workspace_volume_is_mounted` exists to deny the agent itself.
///
/// The snapshot route is also the better product: the pane can show what the plan
/// said *at the moment a verdict was reached*, because each snapshot is a citable
/// content-addressed handle rather than a file whose history is gone.
pub async fn workspace_file(
    State(state): State<AppState>,
    token: BearerToken,
    Path(project_id): Path<Uuid>,
    Query(q): Query<WorkspaceFileQuery>,
) -> impl IntoResponse {
    if !WORKSPACE_FILES.contains(&q.path.as_str()) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "unknown_file",
                "message": format!("{:?} is not a notebook file", q.path),
                "known": WORKSPACE_FILES,
            })),
        )
            .into_response();
    }

    let owner: Option<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM research_projects WHERE project_id = $1")
            .bind(project_id)
            .fetch_optional(&state.pg)
            .await
            .ok()
            .flatten();
    if owner != Some(token.user_id()) {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))).into_response();
    }

    let Some(registry) = state.artifacts.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "artifacts_unavailable"})),
        )
            .into_response();
    };

    let handle: Option<String> = sqlx::query_scalar(
        "SELECT handle FROM artifacts           WHERE type = 'workspace_snapshot' AND project_id = $1             AND manifest->>'path' = $2           ORDER BY created_at DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(&q.path)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten();

    let Some(handle) = handle else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "no_snapshot",
                "path": q.path,
                "message": "this project has no snapshot of that file yet",
                // One line: a `\` continuation inside a string literal keeps the
                // indentation, and this text is read raw by the `tbot` CLI as well
                // as by a browser that would collapse it.
                "fix": "snapshots are written at each session checkpoint; a project with no completed checkpoint has nothing to show",
            })),
        )
            .into_response();
    };

    // The project scope is passed explicitly: `read` reports a cross-project handle
    // as not-found, and this pane must not become the one place that reads around it.
    match registry.read(&handle, Some(project_id)).await {
        Ok(bytes) => (
            StatusCode::OK,
            Json(json!({
                "path": q.path,
                "handle": handle,
                "content": String::from_utf8_lossy(&bytes),
            })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_approval_kind_in_the_spec_is_listed() {
        // COMP-006 §4 names seven kinds. The inbox has to cover all of them, because
        // a kind with no card is an approval that blocks a session forever with
        // nothing on screen to answer it.
        for kind in [
            "ask_user",
            "plan",
            "budget",
            "qc_waiver",
            "model_promotion",
            "skill_promotion",
            "paper_deployment",
            // The policy engine's pause (ADR-0032). `ApprovalsInbox.tsx` renders the
            // tool and its arguments for this one: an approval that does not show the
            // call is asking someone to authorise something they cannot see.
            "tool_call",
        ] {
            assert!(APPROVAL_KINDS.contains(&kind), "{kind} is missing");
        }
        assert_eq!(APPROVAL_KINDS.len(), 8);
    }

    #[test]
    fn an_ended_session_is_not_steerable() {
        // Accepting steering for a session nothing will read looks exactly like the
        // agent ignoring the user.
        for dead in ["completed", "failed", "aborted", "cancelled"] {
            assert!(!STEERABLE_STATES.contains(&dead), "{dead} should not steer");
        }
        assert!(STEERABLE_STATES.contains(&"running"));
        assert!(STEERABLE_STATES.contains(&"waiting"));
    }

    #[test]
    fn the_inbox_kinds_are_exactly_the_migration_check() {
        // Migration 0039 CHECKs the same four. Two lists that can drift produce a
        // 500 on a valid-looking request, so this pins them together.
        assert_eq!(INBOX_KINDS, &["steer", "interrupt", "stop", "answer"]);
    }
}
