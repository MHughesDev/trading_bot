//! "Since you were away" (AGENT-012 §2.2a, §19; Set S task S-6.24).
//!
//! `AwayDigest.tsx` has been calling `/api/desk/digest` since it was written and
//! nobody served it.
//!
//! # The question it answers
//!
//! A research agent that runs for hours without you is only useful if coming
//! back to it takes a minute, and a reverse-chronological event list is not
//! that: it makes the human reconstruct a state the platform already knows.
//!
//! So every line is a **verdict or a state change**, never an event name. "Run
//! finished" is an event; "the EMA-cross family did not clear Gate 3" is a
//! verdict. The difference is whether reading the line saves you opening the
//! thing.
//!
//! # What it does not invent
//!
//! The digest AGENT-012 describes has six kinds of line. Four have a source in
//! this deployment and are served:
//!
//! * `concluded` — from `agent_runs.summary`, which is the run's own conclusion.
//! * `approval_defaulted` — from `approval_requests` that timed out, with what
//!   defaulted.
//! * `budget` — from `llm_usage`, summed over the gap.
//! * `waiting` — approvals still pending, which is the only forward-looking line.
//!
//! Two have no source and are **omitted rather than approximated**:
//!
//! * `finding` / `refuted` need the memory tables from phase 4. `crates/harness`
//!   has the memory and eval engines; the migrations that would persist findings
//!   do not exist, so there is nothing to read.
//! * `monitor` needs a state-*change* log. `monitor_lease`, `kill_switch` and
//!   `monitor_limits` record current state, and a digest built by diffing
//!   current state against a guess at the old one would report changes that did
//!   not happen.
//!
//! A digest that quietly dropped those would be a digest that says "nothing was
//! refuted" when it means "I cannot see refutations", so the response names the
//! kinds it could not cover. That is the difference between an empty digest and
//! a blind one, and the operator reading it is deciding whether to stop looking.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;

use crate::{auth::BearerToken, state::AppState};

/// Below this the digest is absent entirely.
///
/// Not a tidiness rule: the panel is dismissable and resets the marker, so
/// showing it after a five-minute gap trains the operator to dismiss it without
/// reading — and then it is not there when the gap was eight hours.
const MIN_GAP: Duration = Duration::hours(1);

#[derive(Debug, Deserialize)]
pub struct DigestQuery {
    /// When the operator last looked. The gap is exactly `[since, now]`.
    pub since: DateTime<Utc>,
}

fn line(id: String, at: DateTime<Utc>, kind: &str, text: String, href: Option<String>) -> Value {
    json!({ "id": id, "at": at, "kind": kind, "text": text, "href": href })
}

/// GET /api/desk/digest?since=…
pub async fn digest(
    State(state): State<AppState>,
    token: BearerToken,
    Query(q): Query<DigestQuery>,
) -> impl IntoResponse {
    let until = Utc::now();
    let user = token.user_id();

    if until - q.since < MIN_GAP {
        // Absent, not empty. The two render differently and mean different
        // things: "nothing happened" against "you have not been away".
        return Json(json!({
            "since": q.since,
            "until": until,
            "lines": [],
            "absent": true,
            "reason": "the gap is under an hour",
        }))
        .into_response();
    }

    let mut lines: Vec<Value> = Vec::new();

    // ── runs that concluded, and what they concluded ────────────────────────
    let runs = sqlx::query(
        "SELECT run_id, status, goal, summary, error, finished_at \
           FROM agent_runs \
          WHERE user_id = $1 AND finished_at > $2 AND finished_at <= $3 \
          ORDER BY finished_at DESC LIMIT 50",
    )
    .bind(user)
    .bind(q.since)
    .bind(until)
    .fetch_all(&state.pg)
    .await;

    match runs {
        Ok(rows) => {
            for r in rows {
                let id: uuid::Uuid = r.get("run_id");
                let status: String = r.get("status");
                let goal: String = r.get("goal");
                let summary: Option<String> = r.try_get("summary").ok().flatten();
                let error: Option<String> = r.try_get("error").ok().flatten();
                let at: Option<DateTime<Utc>> = r.try_get("finished_at").ok().flatten();

                // The conclusion if there is one, and the failure if there is
                // not. The goal is the fallback and it is the weakest line here,
                // because a goal is what was asked rather than what was found.
                let text = match (&summary, &error) {
                    (Some(s), _) if !s.trim().is_empty() => s.trim().to_string(),
                    (_, Some(e)) if !e.trim().is_empty() => {
                        format!("{status}: {}", e.trim())
                    }
                    _ => format!("{status}, with no conclusion recorded — “{goal}”"),
                };
                lines.push(line(
                    format!("run:{id}"),
                    at.unwrap_or(until),
                    "concluded",
                    text,
                    Some(format!("/desk/runs/{id}")),
                ));
            }
        }
        Err(e) => return db_error("agent_runs", &e),
    }

    // ── approvals that timed out, and what defaulted ────────────────────────
    let defaulted = sqlx::query(
        "SELECT a.approval_id, a.kind, a.default_option, a.answered_at, a.created_at \
           FROM approval_requests a \
           JOIN research_projects p ON p.project_id = a.project_id \
          WHERE p.user_id = $1 AND a.state = 'defaulted' \
            AND COALESCE(a.answered_at, a.created_at) > $2 \
            AND COALESCE(a.answered_at, a.created_at) <= $3 \
          ORDER BY COALESCE(a.answered_at, a.created_at) DESC LIMIT 50",
    )
    .bind(user)
    .bind(q.since)
    .bind(until)
    .fetch_all(&state.pg)
    .await;

    match defaulted {
        Ok(rows) => {
            for r in rows {
                let id: uuid::Uuid = r.get("approval_id");
                let kind: String = r.get("kind");
                let default_option: Option<String> = r.try_get("default_option").ok().flatten();
                let at: Option<DateTime<Utc>> = r.try_get("answered_at").ok().flatten();
                let created: DateTime<Utc> = r.get("created_at");
                // Naming what defaulted is the whole point: an approval that
                // timed out still decided something.
                let text = match default_option {
                    Some(opt) => format!("`{kind}` timed out and defaulted to **{opt}**"),
                    None => format!(
                        "`{kind}` timed out with no default recorded, so nothing was decided"
                    ),
                };
                lines.push(line(
                    format!("approval:{id}"),
                    at.unwrap_or(created),
                    "approval_defaulted",
                    text,
                    Some(format!("/desk/approvals/{id}")),
                ));
            }
        }
        Err(e) => return db_error("approval_requests", &e),
    }

    // ── what is waiting on you now ──────────────────────────────────────────
    let pending = sqlx::query(
        "SELECT a.approval_id, a.kind, a.created_at, a.timeout_at \
           FROM approval_requests a \
           JOIN research_projects p ON p.project_id = a.project_id \
          WHERE p.user_id = $1 AND a.state = 'pending' \
          ORDER BY a.created_at ASC LIMIT 50",
    )
    .bind(user)
    .fetch_all(&state.pg)
    .await;

    match pending {
        Ok(rows) => {
            for r in rows {
                let id: uuid::Uuid = r.get("approval_id");
                let kind: String = r.get("kind");
                let created: DateTime<Utc> = r.get("created_at");
                let timeout: Option<DateTime<Utc>> = r.try_get("timeout_at").ok().flatten();
                let text = match timeout {
                    Some(t) if t > until => format!(
                        "`{kind}` is waiting on you, and defaults in {} minutes",
                        (t - until).num_minutes().max(0)
                    ),
                    Some(_) => format!("`{kind}` is waiting on you and is past its timeout"),
                    None => format!("`{kind}` is waiting on you, with no timeout"),
                };
                lines.push(line(
                    format!("waiting:{id}"),
                    created,
                    "waiting",
                    text,
                    Some(format!("/desk/approvals/{id}")),
                ));
            }
        }
        Err(e) => return db_error("approval_requests", &e),
    }

    // ── what it cost ────────────────────────────────────────────────────────
    let spend = sqlx::query(
        "SELECT COALESCE(SUM(cost_usd), 0)::float8 AS usd, COUNT(*)::bigint AS calls \
           FROM llm_usage WHERE user_id = $1 AND created_at > $2 AND created_at <= $3",
    )
    .bind(user)
    .bind(q.since)
    .bind(until)
    .fetch_one(&state.pg)
    .await;

    match spend {
        Ok(r) => {
            let usd: f64 = r.try_get("usd").unwrap_or(0.0);
            let calls: i64 = r.try_get("calls").unwrap_or(0);
            if calls > 0 {
                lines.push(line(
                    "budget".into(),
                    until,
                    "budget",
                    format!("${usd:.2} spent over {calls} model calls while you were away"),
                    Some("/desk/budget".into()),
                ));
            }
        }
        Err(e) => return db_error("llm_usage", &e),
    }

    // Newest first, which is how the panel reads.
    lines.sort_by(|a, b| b["at"].as_str().cmp(&a["at"].as_str()));

    Json(json!({
        "since": q.since,
        "until": until,
        "lines": lines,
        // Said out loud. "No findings" and "findings are unreadable here" look
        // identical in an empty list, and only one of them means you can stop
        // looking.
        "kinds_not_covered": [
            {
                "kind": "finding",
                "why": "findings are not persisted in this deployment — the memory engine \
                        exists and its migrations do not"
            },
            {
                "kind": "refuted",
                "why": "same source as `finding`"
            },
            {
                "kind": "monitor",
                "why": "the monitor tables record current state, not state changes, and a \
                        change inferred by diffing against a guess would report changes that \
                        did not happen"
            }
        ],
    }))
    .into_response()
}

fn db_error(table: &str, e: &sqlx::Error) -> axum::response::Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({
            "error": "digest_unavailable",
            "table": table,
            "message": e.to_string(),
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gap rule is the acceptance's third clause, and it is arithmetic, so
    /// it is tested without a database.
    #[test]
    fn a_short_gap_is_below_the_threshold_and_a_long_one_is_not() {
        let now = Utc::now();
        assert!(now - (now - Duration::minutes(59)) < MIN_GAP);
        assert!(now - (now - Duration::minutes(61)) >= MIN_GAP);
    }

    /// Every kind the frontend's `DigestLine` union admits is either emitted or
    /// declared uncovered. A kind that is neither is a silent hole.
    #[test]
    fn every_line_kind_is_either_served_or_declared_uncovered() {
        // `DigestLine['kind']` in frontend/src/api/desk.ts.
        const KINDS: [&str; 7] = [
            "concluded",
            "finding",
            "refuted",
            "approval_defaulted",
            "monitor",
            "budget",
            "waiting",
        ];
        const SERVED: [&str; 4] = ["concluded", "approval_defaulted", "waiting", "budget"];
        const UNCOVERED: [&str; 3] = ["finding", "refuted", "monitor"];
        for k in KINDS {
            assert!(
                SERVED.contains(&k) != UNCOVERED.contains(&k),
                "`{k}` is in neither list, or in both — a kind that is neither served nor \
                 declared uncovered reads as \"this did not happen\" when it means \"I cannot \
                 see it\""
            );
        }
    }
}
