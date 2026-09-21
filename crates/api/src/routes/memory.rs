//! `/api/memory/{bucket}` — the memory browser's source (AGENT-007; Set S phase 4).
//!
//! `MemoryBrowser.tsx` renders an empty state that says, in as many words, "the
//! memory API is not built yet". This is it.
//!
//! # The buckets are not quite the kinds, and that is said out loud
//!
//! The browser offers seven tabs and `memory::Kind` has seven variants, and they
//! are **not the same seven**:
//!
//! * six line up — findings, negative results, instrument profiles, open
//!   questions, preferences, signal registry;
//! * the browser has a **`playbooks`** tab that no kind answers to;
//! * `Kind::PlatformDefect` has **no tab**, so `report_evaluator_defect`'s
//!   output is unreachable from the desk.
//!
//! A bucket with no kind returns a named refusal rather than an empty list.
//! Those render almost identically and mean opposite things: one says "nothing
//! has been recorded", which is a fact about your research, and the other says
//! "this tab is not connected to anything", which is a fact about the platform.
//! The first invites you to stop looking.
//!
//! # Deleting is archiving
//!
//! `DELETE /api/memory/{bucket}/{id}` sets `archived`, and refuses outright when
//! `cited_by_chain` is set. §2.4: nothing an evidence chain cites is ever
//! hard-deleted, because a sealed trial ticket citing a row that no longer
//! exists is a ticket nobody can check — and the point of sealing it was that
//! somebody could.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use harness::memory::{Entry, Kind};
use serde_json::{json, Value};
use sqlx::Row;

use crate::memory_store::MemoryStore;
use crate::{auth::BearerToken, state::AppState};

/// The browser's tab names, mapped to what the store actually holds.
///
/// `playbooks` is deliberately absent: see the module docs.
fn kind_for(bucket: &str) -> Option<Kind> {
    Some(match bucket {
        "findings" => Kind::Finding,
        "negative" => Kind::NegativeResult,
        "profiles" => Kind::InstrumentProfile,
        "questions" => Kind::OpenQuestion,
        "preferences" => Kind::UserPreference,
        "signals" => Kind::SignalRegistry,
        "defects" => Kind::PlatformDefect,
        _ => return None,
    })
}

/// `MemoryRow` in `frontend/src/components/desk/MemoryBrowser.tsx`.
fn row(e: &Entry) -> Value {
    json!({
        "id": e.id,
        "subject": e.subject,
        "claim": e.claim,
        "confidence": e.confidence.as_str(),
        "n": e.measurement.as_ref().and_then(|m| m.n),
        "window": format!("{}→{}", e.data_window.start, e.data_window.end),
        "valid_until": e.valid_until,
        "conflicts_with": e.conflicts_with,
        // The browser shows provenance on every row, because a claim whose
        // origin is not visible is a claim the reader has to take on trust.
        "provenance": {
            "run": e.provenance.run_id,
            "tool": e.provenance.tool,
            "window": format!("{}→{}", e.data_window.start, e.data_window.end),
        },
        "stale": e.stale,
        "corroborations": e.corroborations,
        "cited_by_chain": e.cited_by_chain,
        // §2.2's right-hand column: the fields that belong to one kind rather
        // than to all seven — a question's `blocking` and `status`, a negative
        // result's `failure_class`, a preference's `scope`.
        //
        // Stored since `0122` and served by nothing, so every tab in the browser
        // rendered the generic half of an entry and none of the half that makes
        // it actionable. An open question and a closed one looked identical, and
        // a blocking question looked like a curiosity.
        "extra": e.extra,
    })
}

fn unknown_bucket(bucket: &str) -> axum::response::Response {
    let known = [
        "findings",
        "negative",
        "profiles",
        "questions",
        "preferences",
        "signals",
        "defects",
    ];
    let (code, message) = if bucket == "playbooks" {
        (
            "bucket_has_no_kind",
            "`playbooks` is a tab in the browser and not a kind in the store — AGENT-007 §2.2 \
             has no playbook entry. This is a named refusal and not an empty list on purpose: \
             an empty list would read as \"you have recorded no playbooks\"."
                .to_string(),
        )
    } else {
        ("unknown_bucket", format!("no memory bucket `{bucket}`"))
    };
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": code, "bucket": bucket, "message": message, "known": known })),
    )
        .into_response()
}

/// What a listing may be narrowed by.
#[derive(Debug, serde::Deserialize)]
pub struct ListQuery {
    /// One instrument, hypothesis family, strategy or platform component.
    ///
    /// The question `hypothesis_precheck` asks is "has this subject already
    /// produced a negative result", which is a *subject* question over one kind
    /// — and the route could answer neither half together. Added here rather
    /// than as a second endpoint: two routes over one table is two places for
    /// the archived-and-superseded filter to be forgotten, and that filter is
    /// what stops a retracted entry being read as live.
    #[serde(default)]
    pub subject: Option<String>,
}

/// GET /api/memory/{bucket}?subject=…
pub async fn list(
    State(state): State<AppState>,
    token: BearerToken,
    Path(bucket): Path<String>,
    Query(q): Query<ListQuery>,
) -> impl IntoResponse {
    let Some(kind) = kind_for(&bucket) else {
        return unknown_bucket(&bucket);
    };
    let store = MemoryStore::new(state.pg.clone());
    let fetched = match q.subject.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        // Narrowed by subject, then by kind. The store's subject query is the
        // one that already carries the archived/superseded filter; filtering the
        // kind afterwards costs a few rows and keeps one query.
        Some(subject) => store
            .for_subject(token.user_id(), subject, 200)
            .await
            .map(|entries| entries.into_iter().filter(|e| e.kind == kind).collect()),
        None => store.by_kind(token.user_id(), kind, 200).await,
    };
    match fetched {
        Ok(entries) => {
            let rows: Vec<Value> = entries.iter().map(row).collect();
            Json(json!({
                "rows": rows,
                "bucket": bucket,
                "kind": kind.as_str(),
                // Echoed, so a caller that mistyped a subject reads an empty
                // list as "nothing for *that*" rather than as "nothing at all".
                "subject": q.subject,
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "memory_unavailable", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// DELETE /api/memory/{bucket}/{id} — archive, never erase.
pub async fn forget(
    State(state): State<AppState>,
    token: BearerToken,
    Path((bucket, id)): Path<(String, String)>,
) -> impl IntoResponse {
    if kind_for(&bucket).is_none() {
        return unknown_bucket(&bucket);
    }

    // Checked before the write rather than expressed as a `WHERE NOT
    // cited_by_chain`, so a cited row produces a refusal that says why instead
    // of a silent zero-row update the caller reads as success.
    let cited = sqlx::query(
        "SELECT cited_by_chain FROM agent_memory.entry WHERE entry_id = $1 AND user_id = $2",
    )
    .bind(&id)
    .bind(token.user_id())
    .fetch_optional(&state.pg)
    .await;

    match cited {
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "not_found", "id": id })),
            )
                .into_response()
        }
        Ok(Some(r)) if r.try_get::<bool, _>("cited_by_chain").unwrap_or(false) => {
            return (
                StatusCode::CONFLICT,
                Json(json!({
                    "error": "cited_by_evidence_chain",
                    "id": id,
                    "message": "an evidence chain cites this entry — a sealed ticket or a \
                                dossier — so it is never deleted. AGENT-007 §2.4. Mark it \
                                stale or supersede it instead; both keep the citation \
                                resolvable.",
                })),
            )
                .into_response()
        }
        Ok(Some(_)) => {}
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "memory_unavailable", "message": e.to_string() })),
            )
                .into_response()
        }
    }

    match sqlx::query(
        "UPDATE agent_memory.entry SET archived = true, updated_at = now() \
          WHERE entry_id = $1 AND user_id = $2",
    )
    .bind(&id)
    .bind(token.user_id())
    .execute(&state.pg)
    .await
    {
        Ok(_) => Json(json!({ "ok": true, "id": id, "archived": true })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "memory_unavailable", "message": e.to_string() })),
        )
            .into_response(),
    }
}

// ── the user's own knowledge (AGENT-007 §3.4; Set S task S-4.6) ─────────────

/// A preference the user stated, in their own words.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreferenceBody {
    /// `global`, or an instrument or strategy the preference is about. It is
    /// what the brief matches on, so a preference with the wrong scope is a
    /// preference that is retrieved into the wrong runs — or none.
    pub scope: String,
    /// The preference itself. Stored as the user wrote it: this is the one kind
    /// whose claim is not the platform's sentence, and paraphrasing it would
    /// make "the user said so" untrue of the text under that label.
    pub statement: String,
}

/// POST /api/memory/preferences — record what the user told the agent.
///
/// # Why this needs no table of its own
///
/// The phase plan names `migrations/0068_user_preferences.sql`. There is no such
/// migration and there should not be: `user_preference` is one of the seven
/// kinds in `agent_memory.entry` (AGENT-007 §2.2), with its scope in `extra`. A
/// second table would be a second store to retrieve from, a second retention
/// rule, and a second thing the brief has to merge — for a kind that shares
/// every shared field with the other six. Recorded as ADR-S4-01.
///
/// # The window, and why this kind has an easy answer
///
/// Every entry needs a `data_window` and the loop does not track one for
/// findings (OQ-18). A preference is the case where the question does not
/// arise: the claim is "the user said this", and it is true as of the moment
/// they said it. Today to today, and that is a fact rather than a guess.
///
/// `evidence_class` is `human` and the confidence is `established`, which is the
/// one combination where `established` needs no verification pass — the user is
/// the authority on their own preferences.
pub async fn record_preference(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<PreferenceBody>,
) -> impl IntoResponse {
    let scope = body.scope.trim();
    let statement = body.statement.trim();
    if scope.is_empty() || statement.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "missing_field",
                "message": "a preference needs a `scope` and a `statement`; `global` is a                             legitimate scope and an empty one is not",
            })),
        )
            .into_response();
    }

    let today = chrono::Utc::now().date_naive().to_string();
    let id = format!("pref_{}", uuid::Uuid::new_v4().simple());
    let mut entry = Entry::finding(
        &id,
        scope,
        statement,
        harness::memory::DataWindow::new(today.clone(), today),
        // The user is the source, and the provenance says so rather than naming
        // whichever tool happened to carry the message.
        harness::memory::Provenance::new("user", "ask_user", "1", "n/a"),
    );
    entry.kind = Kind::UserPreference;
    entry.confidence = harness::memory::Confidence::Established;
    entry.evidence_class = harness::memory::EvidenceClass::Human;
    entry.created_by = token.user_id().to_string();
    entry.extra = json!({ "scope": scope, "statement": statement });

    let store = MemoryStore::new(state.pg.clone());
    match store.put(token.user_id(), &entry).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({ "id": id, "scope": scope, "statement": statement })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "preference_rejected", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind is reachable from some bucket.
    ///
    /// The browser had no tab for `platform_defect`, which meant
    /// `report_evaluator_defect` wrote somewhere nobody could look. A kind with
    /// no route is a kind whose entries are written and never read, and the
    /// writing side has no way to notice.
    #[test]
    fn every_kind_is_reachable_from_a_bucket() {
        let all = [
            Kind::Finding,
            Kind::NegativeResult,
            Kind::InstrumentProfile,
            Kind::SignalRegistry,
            Kind::PlatformDefect,
            Kind::UserPreference,
            Kind::OpenQuestion,
        ];
        let buckets = [
            "findings",
            "negative",
            "profiles",
            "questions",
            "preferences",
            "signals",
            "defects",
        ];
        for k in all {
            assert!(
                buckets.iter().any(|b| kind_for(b) == Some(k)),
                "`{}` has no bucket, so nothing can read it back",
                k.as_str()
            );
        }
    }

    /// No two buckets answer to the same kind.
    #[test]
    fn a_kind_is_served_by_exactly_one_bucket() {
        let buckets = [
            "findings",
            "negative",
            "profiles",
            "questions",
            "preferences",
            "signals",
            "defects",
        ];
        let mut seen: Vec<Kind> = Vec::new();
        for b in buckets {
            let k = kind_for(b).expect("a declared bucket resolves");
            assert!(!seen.contains(&k), "two buckets both serve `{}`", k.as_str());
            seen.push(k);
        }
    }

    /// The browser's `playbooks` tab is refused by name, not answered with an
    /// empty list.
    #[test]
    fn the_playbooks_tab_is_refused_rather_than_answered_emptily() {
        assert!(
            kind_for("playbooks").is_none(),
            "there is no playbook kind in AGENT-007 §2.2; inventing one here would be a \
             schema decision taken in a route"
        );
    }
}
