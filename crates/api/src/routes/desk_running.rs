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
                r.ring, r.tier, r.profile_name, r.model, r.iterations,
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
                // What the run may modify (AGENT-013 §4; migration 0126, NOT
                // NULL with a CHECK). The conversation header shows it beside
                // mode, workflow and stage, and the working state that also
                // carries it is served by no route (S-10.15).
                "ring": r.get::<String, _>("ring"),
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

// ── A run's memory brief (AGENT-012 §3.5; AGENT-007 §4; Set S task S-6.10) ──

/// Whether anything in production records that a run **cited** a brief entry.
///
/// `RetrievalStore::mark_cited` is the only writer of `memory_retrievals.cited`.
/// It had no caller outside its tests until 2026-10-04, when the local driver's
/// recorder began marking the ids a `record_finding` / `finish_task` evidence
/// names (`Recorder::record_citations`). Runs from before that day carry
/// `cited = false` because nobody measured it; the column is served as measured
/// from here on, and `cited_is_unmeasured_only_while_nothing_marks_a_citation`
/// holds the constant to the source in both directions.
const CITATION_IS_RECORDED: bool = true;

/// The kind's bucket on `/api/memory/{bucket}`, so the desk's **forget** can
/// name the route without a second table of names in the browser.
fn bucket_for(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "finding" => "findings",
        "negative_result" => "negative",
        "instrument_profile" => "profiles",
        "open_question" => "questions",
        "user_preference" => "preferences",
        "signal_registry" => "signals",
        "platform_defect" => "defects",
        _ => return None,
    })
}

/// `GET /api/desk/runs/{run_id}/brief` — what retrieval put into this run's
/// prompt, why each entry was chosen, and (when it is recorded) whether the run
/// cited it.
///
/// Read from `memory_retrievals`, which `load_brief` writes before the run
/// starts — one row per entry with its slot, rank and the `why` clause the
/// brief rendered. The entries are joined in a **second** query rather than a
/// `JOIN`: a retrieval row is the record of what the run was shown, and it
/// stays true when the entry is later archived (that is what **forget** does)
/// or unreadable, so a failure on the entry side degrades to `entry: null`
/// rather than taking the brief with it.
///
/// # Errors
/// `404` for a run this caller does not own; `500` when the retrieval log
/// cannot be read — never an empty brief, which would read as "retrieval chose
/// nothing".
pub async fn brief(
    State(state): State<AppState>,
    token: BearerToken,
    axum::extract::Path(run_id): axum::extract::Path<Uuid>,
) -> impl IntoResponse {
    let owned: Result<Option<(Uuid,)>, _> =
        sqlx::query_as("SELECT run_id FROM agent_runs WHERE run_id = $1 AND user_id = $2")
            .bind(run_id)
            .bind(token.user_id())
            .fetch_optional(&state.pg)
            .await;
    match owned {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "storage", "message": e.to_string() })),
            )
                .into_response()
        }
    }

    let rows = match sqlx::query(
        "SELECT entry_id, rank, slot, why, cited, cited_at, created_at
           FROM memory_retrievals
          WHERE run_id = $1
          ORDER BY slot, rank",
    )
    .bind(run_id.to_string())
    .fetch_all(&state.pg)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": "retrievals_unavailable",
                    "message": e.to_string(),
                })),
            )
                .into_response()
        }
    };

    let ids: Vec<String> = rows.iter().map(|r| r.get::<String, _>("entry_id")).collect();
    // The entries, best-effort. `None` is "could not be read", kept apart from
    // an entry that is simply absent from the store.
    let entries: Option<std::collections::HashMap<String, Value>> = if ids.is_empty() {
        Some(std::collections::HashMap::new())
    } else {
        sqlx::query(
            "SELECT entry_id, kind, subject, claim, archived, superseded_by
               FROM agent_memory.entry
              WHERE user_id = $1 AND entry_id = ANY($2)",
        )
        .bind(token.user_id())
        .bind(&ids)
        .fetch_all(&state.pg)
        .await
        .ok()
        .map(|es| {
            es.iter()
                .map(|e| {
                    let kind: String = e.get("kind");
                    (
                        e.get::<String, _>("entry_id"),
                        json!({
                            "kind": kind,
                            "bucket": bucket_for(&kind),
                            "subject": e.get::<String, _>("subject"),
                            "claim": e.get::<String, _>("claim"),
                            // Forgotten since: the next brief will not carry it.
                            "archived": e.get::<bool, _>("archived"),
                            "superseded": e.get::<Option<String>, _>("superseded_by").is_some(),
                        }),
                    )
                })
                .collect()
        })
    };

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            let id: String = r.get("entry_id");
            json!({
                "entry_id": id,
                "rank": r.get::<i32, _>("rank"),
                "slot": r.get::<String, _>("slot"),
                "why": r.get::<String, _>("why"),
                // `null`, not `false`, while nothing records citations — see
                // `CITATION_IS_RECORDED`.
                "cited": if CITATION_IS_RECORDED { Some(r.get::<bool, _>("cited")) } else { None },
                "cited_at": r.get::<Option<DateTime<Utc>>, _>("cited_at"),
                "retrieved_at": r.get::<DateTime<Utc>, _>("created_at"),
                "entry": entries.as_ref().map(|m| m.get(&id).cloned().unwrap_or(Value::Null)),
            })
        })
        .collect();

    let mut not_recorded = Vec::new();
    if !CITATION_IS_RECORDED {
        not_recorded.push(json!({
            "field": "cited",
            "why": "nothing in production marks a retrieval as cited — the store's writer has no \
                    caller outside its tests — so whether the run used an entry is unmeasured, \
                    not false",
        }));
    }
    if entries.is_none() {
        not_recorded.push(json!({
            "field": "entry",
            "why": "the memory store could not be read for this request, so each item carries \
                    the retrieval record without the entry's claim",
        }));
    }
    not_recorded.push(json!({
        "field": "elided",
        "why": "how many entries matched and did not fit the brief is computed at assembly and \
                not persisted",
    }));

    (
        StatusCode::OK,
        Json(json!({
            "run_id": run_id,
            "items": items,
            "entries_readable": entries.is_some(),
            "not_recorded": not_recorded,
        })),
    )
        .into_response()
}

// ── A run's hypotheses, with the numbers that decided them (S-10.15) ─────────

/// A stored hypothesis, as `agent_lifecycle.hypothesis` holds it.
#[derive(Debug, Clone)]
pub struct HypothesisRow {
    pub hypothesis_id: Uuid,
    pub statement: String,
    pub falsifier: String,
    pub role: String,
    pub status: String,
    pub inconclusive_why: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
}

/// The verdict card for one stored hypothesis: what it said, the platform's
/// verdict, and, from the falsifier's own text and the ledger's rows, the
/// metric, comparator, threshold and every observed value that decided it.
///
/// `fired` on a measurement is `Comparator::fired`: a satisfied falsifier is a
/// **refutation**, so the card reads `fired: true` as "this number refuted it".
///
/// `parsed` is `null` with `parse_error` when the falsifier text does not read
/// (a registration the platform accepted and cannot grade); the card then has
/// no numbers to show and says so rather than inventing a threshold.
#[must_use]
pub fn hypothesis_card(row: &HypothesisRow, measurements: &[(Uuid, f64, DateTime<Utc>)]) -> Value {
    let parsed = harness::lifecycle::evaluate::parse(
        &row.falsifier,
        crate::routes::lifecycle::METRIC_VOCABULARY,
    );
    let (parsed_json, observed, parse_error) = match &parsed {
        Ok(f) => (
            json!({
                "metric": f.metric,
                "comparator": f.comparator.as_str(),
                "threshold": f.threshold,
            }),
            measurements
                .iter()
                .map(|(ticket, value, at)| {
                    json!({
                        "ticket_id": ticket,
                        "value": value,
                        "recorded_at": at,
                        "fired": f.comparator.fired(*value, f.threshold),
                    })
                })
                .collect::<Vec<_>>(),
            Value::Null,
        ),
        Err(e) => (Value::Null, Vec::new(), json!(e.to_string())),
    };
    json!({
        "hypothesis_id": row.hypothesis_id,
        "statement": row.statement,
        "role": row.role,
        "falsifier": row.falsifier,
        "status": row.status,
        "inconclusive_why": row.inconclusive_why,
        "created_at": row.created_at,
        "decided_at": row.decided_at,
        "parsed": parsed_json,
        "parse_error": parse_error,
        // Only values recorded up to the moment of the verdict, so a number that
        // arrived after it is not shown as having decided it.
        "observed": observed,
    })
}

/// `GET /api/desk/runs/{run_id}/hypotheses` — the run's hypotheses with the
/// platform's verdict and the numbers behind it (S-10.15's last clause).
///
/// The verdict is the platform's (`lifecycle_evaluate`), and these numbers are
/// the ledger rows it read: `mlops.trial_statistic` for the falsifier's metric,
/// joined through `mlops.trial.hypothesis_id`, up to the moment it decided.
///
/// # Errors
/// `404` for a run this caller does not own; `500` when the store cannot be read
/// (never an empty list, which would read as "no hypotheses").
pub async fn hypotheses(
    State(state): State<AppState>,
    token: BearerToken,
    axum::extract::Path(run_id): axum::extract::Path<Uuid>,
) -> impl IntoResponse {
    let storage = |e: sqlx::Error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "storage", "message": e.to_string() })),
        )
            .into_response()
    };
    match sqlx::query_as::<_, (Uuid,)>(
        "SELECT run_id FROM agent_runs WHERE run_id = $1 AND user_id = $2",
    )
    .bind(run_id)
    .bind(token.user_id())
    .fetch_optional(&state.pg)
    .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" }))).into_response()
        }
        Err(e) => return storage(e),
    }
    let rows = match sqlx::query(
        "SELECT hypothesis_id, statement, falsifier, role, status, inconclusive_why,
                created_at, decided_at
           FROM agent_lifecycle.hypothesis
          WHERE run_id = $1
          ORDER BY created_at",
    )
    .bind(run_id)
    .fetch_all(&state.pg)
    .await
    {
        Ok(r) => r,
        Err(e) => return storage(e),
    };
    let tenant = token.user_id().to_string();
    let mut cards = Vec::with_capacity(rows.len());
    for r in &rows {
        let row = HypothesisRow {
            hypothesis_id: r.get("hypothesis_id"),
            statement: r.get("statement"),
            falsifier: r.get("falsifier"),
            role: r.get("role"),
            status: r.get("status"),
            inconclusive_why: r.get("inconclusive_why"),
            created_at: r.get("created_at"),
            decided_at: r.get("decided_at"),
        };
        let metric = harness::lifecycle::evaluate::parse(
            &row.falsifier,
            crate::routes::lifecycle::METRIC_VOCABULARY,
        )
        .map(|f| f.metric)
        .ok();
        let measurements: Vec<(Uuid, f64, DateTime<Utc>)> = match metric {
            Some(m) => match sqlx::query_as(
                "SELECT s.trial_id, s.value, s.recorded_at
                   FROM mlops.trial_statistic s
                   JOIN mlops.trial t ON t.trial_id = s.trial_id
                  WHERE t.hypothesis_id = $1 AND t.tenant_id = $2 AND s.name = $3
                    AND ($4::timestamptz IS NULL OR s.recorded_at <= $4)
                  ORDER BY s.recorded_at",
            )
            .bind(row.hypothesis_id)
            .bind(&tenant)
            .bind(&m)
            .bind(row.decided_at)
            .fetch_all(&state.pg)
            .await
            {
                Ok(v) => v,
                Err(e) => return storage(e),
            },
            None => Vec::new(),
        };
        cards.push(hypothesis_card(&row, &measurements));
    }
    Json(json!({ "run_id": run_id, "hypotheses": cards })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hypothesis_card_carries_the_numbers_that_decided_it() {
        let at = Utc::now();
        let row = HypothesisRow {
            hypothesis_id: Uuid::nil(),
            statement: "momentum survives costs".into(),
            falsifier: "sharpe <= 0 on the out-of-sample window".into(),
            role: "primary".into(),
            status: "refuted".into(),
            inconclusive_why: None,
            created_at: at,
            decided_at: Some(at),
        };
        let card = hypothesis_card(&row, &[(Uuid::nil(), -0.2, at), (Uuid::nil(), 0.6, at)]);
        assert_eq!(card["parsed"]["metric"], "sharpe");
        assert_eq!(card["parsed"]["comparator"], "<=");
        assert_eq!(card["parsed"]["threshold"], 0.0);
        assert_eq!(card["observed"][0]["value"], -0.2);
        assert_eq!(card["observed"][0]["fired"], true, "-0.2 <= 0 refutes it");
        assert_eq!(card["observed"][1]["fired"], false);
        assert_eq!(card["status"], "refuted");
    }

    #[test]
    fn an_unreadable_falsifier_has_no_numbers_and_says_why() {
        let at = Utc::now();
        let row = HypothesisRow {
            hypothesis_id: Uuid::nil(),
            statement: "it works".into(),
            falsifier: "vibes get worse".into(),
            role: "primary".into(),
            status: "open".into(),
            inconclusive_why: None,
            created_at: at,
            decided_at: None,
        };
        let card = hypothesis_card(&row, &[(Uuid::nil(), 1.0, at)]);
        assert!(card["parsed"].is_null());
        assert!(card["parse_error"].as_str().is_some());
        assert_eq!(card["observed"].as_array().map(Vec::len), Some(0));
    }

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
            "ring",
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
        assert_eq!(checked, 9);
    }
    #[test]
    fn cited_is_unmeasured_only_while_nothing_marks_a_citation() {
        // The constant is a claim about the rest of the crate, so it is checked
        // against the rest of the crate. A production caller of `mark_cited`
        // makes `cited` a measurement, and serving it as `null` from then on
        // would hide a number the platform started recording.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut callers = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("src is readable") {
                let path = entry.expect("a dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(&path).expect("a source file");
                    // Only code outside `#[cfg(test)]` counts: cut at the test
                    // module, which in this crate always sits at the end.
                    let live = src.split("#[cfg(test)]").next().unwrap_or("");
                    let calls = live
                        .lines()
                        .filter(|l| !l.trim_start().starts_with("//"))
                        .any(|l| l.contains(".mark_cited("));
                    if calls {
                        callers.push(path.display().to_string());
                    }
                }
            }
        }
        assert_eq!(
            CITATION_IS_RECORDED,
            !callers.is_empty(),
            "CITATION_IS_RECORDED is {CITATION_IS_RECORDED} but production callers of mark_cited are {callers:?} — flip the constant so `cited` is served as what it is"
        );
    }

    #[test]
    fn every_memory_kind_has_a_bucket_forget_can_name() {
        // The seven kinds of `agent_memory.entry`'s CHECK. A kind with no bucket
        // is a brief item whose **forget** has no route to call.
        let kinds = [
            "finding",
            "negative_result",
            "instrument_profile",
            "signal_registry",
            "platform_defect",
            "user_preference",
            "open_question",
        ];
        let buckets: std::collections::BTreeSet<_> =
            kinds.iter().map(|k| bucket_for(k).expect("every kind has a bucket")).collect();
        assert_eq!(buckets.len(), kinds.len(), "two kinds share a bucket");
        assert!(bucket_for("playbook").is_none());
        // And each bucket is one `/api/memory/{bucket}` answers.
        let memory = include_str!("memory.rs");
        for b in &buckets {
            assert!(memory.contains(&format!("\"{b}\" =>")), "{b} is not a bucket memory.rs serves");
        }
    }
}
