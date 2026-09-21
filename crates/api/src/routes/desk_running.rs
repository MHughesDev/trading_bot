//! "Running now" (AGENT-012 §2.2; Set S phase 6).
//!
//! `GET /api/desk/running` — one row per run that is actually working, with the
//! per-run facts the panel asks for.
//!
//! # Why this is not a field on `/api/agent/conversations`
//!
//! The conversation list answers "what have I got open": one row per
//! conversation, title, model, whether *any* turn is active. §2.2's panel asks a
//! different question — "what is working right now, on what, at what cost" — and
//! every one of its columns is a fact about a **run**, not a conversation. A
//! conversation with three finished turns and one working one has one tier, one
//! step and one token count that matter, and they are the working turn's;
//! folding them onto the conversation row would mean picking a turn and not
//! saying which.
//!
//! So this route joins the other way: runs first, the conversation carried along
//! as the thing the row opens.
//!
//! # What it deliberately does not send
//!
//! **A dollar figure.** §2.2 asks for "tokens and dollars so far". Tokens are
//! recorded per run; dollars are not. `llm_usage.cost_usd` keys on an
//! `agent_sessions` id, not on a run, so calling it with a run id returns
//! all-zeros — a previous track found that and refused to wire it, and a `$0.00`
//! beside a working run is worse than a blank because it reads as a measurement.
//! The response says so in `not_recorded` rather than leaving the reader to
//! wonder whether the run really was free.
//!
//! **A cache hit percentage.** `cache_read_tokens` and `prompt_tokens` are both
//! sent and the ratio is the reader's to take, but it is not computed here: a
//! run with `prompt_tokens = NULL` has no denominator, and a percentage computed
//! from a missing number would be an invented one.
//!
//! **The working-state step.** §2.2's "current step (from the working state)"
//! needs the per-run working-state snapshot route that §17 lists as unbuilt.
//! (Spelled out rather than quoted: this crate's route scan counts any `/api/…`
//! literal in its source as a route it serves, doc comments included, so writing
//! that path here would make an unbuilt one read as built.)
//! What is recorded is the run's lifecycle `stage` and the last entry in its own
//! transcript, and both are sent under names that say which they are. Neither is
//! called `step`, because a reader who believes they are looking at the working
//! state's step is reading something else.
//!
//! Everything nullable here is sent as `null` when it was not recorded. A run
//! that crashed before it filled its telemetry is still a run (migration 0062
//! made all ten columns nullable for exactly that reason), and a zero in
//! `trials_spent` is a claim that the run searched nothing.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

use crate::agent::conversations::ACTIVE_RUN_STATES;
use crate::{auth::BearerToken, state::AppState};

/// How many working runs the panel will render. A desk with more than this
/// running has a different problem than a truncated list.
const LIMIT: i64 = 100;

/// `GET /api/desk/running`
pub async fn running(State(state): State<AppState>, token: BearerToken) -> impl IntoResponse {
    let rows = match sqlx::query(
        "SELECT r.run_id, r.conversation_id, c.title, r.status, r.mode, r.workflow, r.stage,
                r.tier, r.profile_name, r.model, r.iterations,
                r.prompt_tokens, r.cache_read_tokens, r.trials_spent, r.outcome,
                r.created_at, r.started_at,
                m.seq AS step_seq, m.kind AS step_kind, m.created_at AS step_at
           FROM agent_runs r
           LEFT JOIN agent_conversations c ON c.conversation_id = r.conversation_id
           LEFT JOIN LATERAL (
                SELECT seq, kind, created_at
                  FROM agent_messages
                 WHERE run_id = r.run_id
                 ORDER BY seq DESC
                 LIMIT 1
           ) m ON TRUE
          WHERE r.user_id = $1 AND r.status = ANY($2)
          ORDER BY r.created_at DESC
          LIMIT $3",
    )
    .bind(token.user_id())
    .bind(ACTIVE_RUN_STATES)
    .bind(LIMIT)
    .fetch_all(&state.pg)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "storage", "message": e.to_string() })),
            )
                .into_response()
        }
    };

    let runs: Vec<Value> = rows
        .iter()
        .map(|r| {
            let step_seq: Option<i32> = r.get("step_seq");
            let step_kind: Option<String> = r.get("step_kind");
            let step_at: Option<DateTime<Utc>> = r.get("step_at");
            json!({
                "run_id": r.get::<Uuid, _>("run_id"),
                "conversation_id": r.get::<Option<Uuid>, _>("conversation_id"),
                "title": r.get::<Option<String>, _>("title"),
                "status": r.get::<String, _>("status"),
                "mode": r.get::<String, _>("mode"),
                // Null for a Chat turn, which is not doing one of the seven.
                "workflow": r.get::<Option<String>, _>("workflow"),
                "stage": r.get::<String, _>("stage"),
                // The tier the run was dispatched at, and the profile by name.
                // Both nullable: a run started before 0040/0062 has neither, and
                // a tier guessed from the model string would be a capability
                // claim rather than a record.
                "tier": r.get::<Option<String>, _>("tier"),
                "profile_name": r.get::<Option<String>, _>("profile_name"),
                "model": r.get::<String, _>("model"),
                "iterations": r.get::<i32, _>("iterations"),
                // Prompt size, not what was billed: with caching on the two differ
                // by the hit rate, and this is the one a context budget compares to.
                "prompt_tokens": r.get::<Option<i64>, _>("prompt_tokens"),
                "cache_read_tokens": r.get::<Option<i64>, _>("cache_read_tokens"),
                "trials_spent": r.get::<Option<i32>, _>("trials_spent"),
                "outcome": r.get::<Option<String>, _>("outcome"),
                "created_at": r.get::<DateTime<Utc>, _>("created_at"),
                "started_at": r.get::<Option<DateTime<Utc>>, _>("started_at"),
                // The last entry in the run's own transcript. Null for a run that
                // has not written one yet — which is a real state, not an error.
                "last_transcript_entry": step_seq.map(|seq| json!({
                    "seq": seq, "kind": step_kind, "at": step_at,
                })),
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({
            "runs": runs,
            // Named rather than omitted, so a blank column is a thing the platform
            // says it cannot measure instead of a thing the panel forgot.
            "not_recorded": NOT_RECORDED
                .iter()
                .map(|[field, why]| json!({ "field": field, "why": why }))
                .collect::<Vec<_>>(),
        })),
    )
        .into_response()
}

/// Columns §2.2 asks for that nothing in this deployment records.
///
/// Each names why, because "dollars: —" with no explanation is read as free.
const NOT_RECORDED: &[[&str; 2]] = &[
    [
        "cost_usd",
        "llm_usage.cost_usd keys on an agent_sessions id, not on a run, so asking it for a run's \
         spend returns zero rather than an answer. There is no per-run USD anywhere, which is the \
         same reason session digests are denominated in trials (migration 0139).",
    ],
    [
        "working_state_step",
        "the per-run working-state snapshot route is unbuilt (AGENT-012 §17). \
         `stage` and `last_transcript_entry` are what is recorded and are not the same thing.",
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_is_told_what_cannot_be_measured() {
        // The failure this guards is the one the desk tracks kept finding: a
        // column that renders blank or zero, indistinguishable from a measured
        // zero. Every entry has to carry a reason, or it is a blank with a label.
        assert!(!NOT_RECORDED.is_empty());
        for [field, why] in NOT_RECORDED {
            assert!(!field.is_empty(), "a field with no name explains nothing");
            assert!(
                why.len() > 40,
                "{field} is listed as unrecorded without saying why"
            );
        }
        let names: Vec<&str> = NOT_RECORDED.iter().map(|[f, _]| *f).collect();
        assert!(
            names.contains(&"cost_usd"),
            "the dollar column is the one §2.2 asks for and nothing measures; dropping it from \
             this list is how a $0.00 gets rendered as a measurement"
        );
    }

    #[test]
    fn no_dollar_figure_is_computed_anywhere_in_this_route() {
        // `llm_usage` keys on an auth session and this route keys on runs.
        // Reaching for it here would return all-zeros and look like an answer.
        let src = include_str!("desk_running.rs");
        let query_start = src
            .find("SELECT r.run_id")
            .expect("the query is gone; this test is stale");
        let query = &src[query_start..];
        let query = &query[..query.find("\")").unwrap_or(query.len())];
        assert!(
            !query.contains("llm_usage") && !query.contains("cost_usd"),
            "the running-now query reached for a cost column; per-run USD does not exist and what \
             comes back is zero"
        );
    }

    #[test]
    fn every_column_the_panel_reads_is_selected() {
        // A source scan over the pair that has to agree: the SQL projection and
        // the JSON the handler builds. The failure it catches is the one this
        // whole track exists for — a field the panel wants that the query never
        // asked the database for.
        let src = include_str!("desk_running.rs");
        let query_start = src.find("SELECT r.run_id").expect("the query is gone");
        let query = &src[query_start..];
        let query = &query[..query.find("\")").unwrap_or(query.len())];
        let mut checked = 0;
        for column in [
            "profile_name",
            "tier",
            "stage",
            "prompt_tokens",
            "cache_read_tokens",
            "trials_spent",
            "outcome",
            "conversation_id",
        ] {
            assert!(
                query.contains(column),
                "{column} is read by the handler and never selected"
            );
            assert!(
                src.contains(&format!("\"{column}\"")),
                "{column} is selected and never sent"
            );
            checked += 1;
        }
        // A loop that matched nothing would pass silently.
        assert_eq!(checked, 8);
    }
}
