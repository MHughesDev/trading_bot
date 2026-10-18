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
//! # One path, every method
//!
//! `GET`, `POST` and `DELETE` all hang off `{bucket}`; there is no static
//! segment under `/api/memory/`. That is deliberate and it is the fix for a real
//! defect rather than a preference: `POST /api/memory/preferences` was mounted
//! on its own literal path, `matchit` prefers a literal over a parameter, and so
//! `GET /api/memory/preferences` answered **405 Method Not Allowed** while the
//! other six buckets answered 200. Since [`kind_for`] enumerates the legitimate
//! values of `{bucket}`, any literal written under this prefix is a second
//! spelling of a bucket and will shadow it the same way. See [`record`].
//!
//! # Deleting is archiving
//!
//! `DELETE /api/memory/{bucket}/{id}` sets `archived`, and refuses outright when
//! `cited_by_chain` is set. §2.4: nothing an evidence chain cites is ever
//! hard-deleted, because a sealed trial ticket citing a row that no longer
//! exists is a ticket nobody can check — and the point of sealing it was that
//! somebody could.

use axum::{
    extract::{rejection::JsonRejection, Path, Query, State},
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

/// The one bucket a caller may *write*, and the kind that write lands under.
///
/// A closed match with no permissive arm, for the same reason `kind_for` has
/// none: the other six kinds are written by the research loop out of a
/// measurement it took, and a caller-supplied claim filed under `finding` or
/// `negative_result` would be a measurement nobody made. A preference is the one
/// kind whose authority *is* the caller — see `record`'s docs.
///
/// Pinned by [`tests::exactly_one_bucket_accepts_a_write_and_it_is_preferences`].
fn user_writable_kind_for(bucket: &str) -> Option<Kind> {
    match bucket {
        "preferences" => Some(Kind::UserPreference),
        _ => None,
    }
}

/// `MemoryRow` in `frontend/src/components/desk/MemoryBrowser.tsx`.
fn row(e: &Entry) -> Value {
    json!({
        "id": e.id,
        "subject": e.subject,
        "claim": e.claim,
        "confidence": e.confidence.as_str(),
        // What kind of evidence the claim rests on (AGENT-007 §3.1). Served
        // because the desk's provenance fence needs it and the row is where it
        // has to arrive: the browser reads `evidence_class` on every row and
        // could not render the mark without it, so a `model_summary` and a
        // `direct_stat` looked the same in the one surface whose whole job is
        // telling them apart.
        //
        // `Entry` has carried it since `0122` and `record_preference` sets it;
        // only this serialiser dropped it. That is the repository's dominant
        // defect shape with the halves swapped — a reader of a field nothing
        // writes, except here the field was written and nothing served it.
        "evidence_class": e.evidence_class,
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
        // Derived from untrusted text (KM-10). The browser shows every entry and marks
        // these; the agent's default retrieval leaves them out.
        "tainted": e.is_tainted(),
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
    // `report` is the monthly store report (AGENT-007 §2.4, S-4.16): size,
    // archive rate and cited share. It lives under the one `{bucket}` path for
    // the shadowing reason in the module docs, not on a literal of its own.
    if bucket == "report" {
        return match MemoryStore::new(state.pg.clone())
            .store_report(token.user_id())
            .await
        {
            Ok(r) => (StatusCode::OK, Json(json!(r))).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "report_failed", "message": e.to_string() })),
            )
                .into_response(),
        };
    }
    let Some(kind) = kind_for(&bucket) else {
        return unknown_bucket(&bucket);
    };
    let store = MemoryStore::new(state.pg.clone());
    let fetched = match q
        .subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Narrowed by subject, then by kind. The store's subject query is the
        // one that already carries the archived/superseded filter; filtering the
        // kind afterwards costs a few rows and keeps one query.
        Some(subject) => store
            .for_subject_including_tainted(token.user_id(), subject, 200)
            .await
            .map(|entries| entries.into_iter().filter(|e| e.kind == kind).collect()),
        None => {
            store
                .by_kind_including_tainted(token.user_id(), kind, 200)
                .await
        }
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

/// POST /api/memory/{bucket} — record what the user told the agent.
///
/// Reachable as `POST /api/memory/preferences`, which is the only bucket that
/// accepts a write. The caller's URL is unchanged from when this was mounted on
/// its own literal path; what changed is that the literal no longer exists.
///
/// # Why the write shares the reader's path
///
/// It used to be `.route("/api/memory/preferences", post(record_preference))`,
/// mounted beside `.route("/api/memory/{bucket}", get(list))`. `matchit` prefers
/// a static segment over a parameter, so the literal took every request to that
/// path and refused the method: `GET /api/memory/preferences` answered **405**
/// while its six sibling buckets answered normally. `list` was never reached for
/// the one bucket whose entries the user themselves wrote, and the 405 rendered
/// in the browser as a tab that had nothing in it.
///
/// So the write moved onto the parameter instead of the literal being patched.
/// Every legitimate value of `{bucket}` is enumerated by [`kind_for`], which
/// makes a literal under `/api/memory/` a second spelling of a bucket by
/// construction — the mistake is not documented here, it is unavailable, because
/// there is no static segment left under this prefix to make it with. The
/// general form is held by
/// `crates/api/tests/a_static_route_does_not_shadow_its_parameterised_sibling.rs`,
/// which found four more pairs of this shape in the router.
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
pub async fn record(
    State(state): State<AppState>,
    token: BearerToken,
    Path(bucket): Path<String>,
    // Taken as a `Result` rather than extracted, so the bucket is judged first.
    // With a plain `Json<PreferenceBody>` the extractor rejects before the
    // handler runs, and a write to a bucket that takes no writes would be
    // refused for the shape of its body instead of for the bucket — the caller
    // would go and fix the body.
    body: Result<Json<PreferenceBody>, JsonRejection>,
) -> impl IntoResponse {
    if kind_for(&bucket).is_none() {
        return unknown_bucket(&bucket);
    }
    if user_writable_kind_for(&bucket).is_none() {
        // Deliberately not 405. This route's whole repair was a 405 that a reader
        // could not tell from an empty bucket, and answering one here would put
        // the same ambiguity back on the write side.
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "bucket_is_not_user_writable",
                "bucket": bucket,
                "writable": ["preferences"],
                "message": "only `preferences` accepts a write. The other six kinds are \
                            recorded by the research loop out of a measurement it took, and a \
                            claim posted under one of those labels would be a measurement \
                            nobody made — with a provenance saying otherwise. AGENT-007 §2.2.",
            })),
        )
            .into_response();
    }
    let Json(body) = match body {
        Ok(json) => json,
        Err(rejection) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": "malformed_body",
                    "bucket": bucket,
                    "message": rejection.body_text(),
                })),
            )
                .into_response()
        }
    };

    let scope = body.scope.trim();
    let statement = body.statement.trim();
    if scope.is_empty() || statement.is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "missing_field",
                "message": "a preference needs a `scope` and a `statement`; `global` is a \
                            legitimate scope and an empty one is not",
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
            assert!(
                !seen.contains(&k),
                "two buckets both serve `{}`",
                k.as_str()
            );
            seen.push(k);
        }
    }

    /// Exactly one bucket accepts a write, and it is `preferences`.
    ///
    /// `record` is mounted on `{bucket}`, so every bucket name reaches it. What
    /// stops it writing a `finding` the caller made up is
    /// `user_writable_kind_for`, and this is the test that says so: widening it
    /// to a second bucket would give a caller a way to post a claim whose
    /// provenance says the platform measured it.
    #[test]
    fn exactly_one_bucket_accepts_a_write_and_it_is_preferences() {
        let buckets = [
            "findings",
            "negative",
            "profiles",
            "questions",
            "preferences",
            "signals",
            "defects",
        ];
        let writable: Vec<&str> = buckets
            .into_iter()
            .filter(|b| user_writable_kind_for(b).is_some())
            .collect();
        assert_eq!(
            writable,
            ["preferences"],
            "these buckets accept a caller-supplied write: {writable:?}. Only `preferences` may \
             — the other six carry a provenance claiming the platform measured the thing."
        );
        assert_eq!(
            user_writable_kind_for("preferences"),
            Some(Kind::UserPreference),
        );
    }

    /// Anything writable is readable: a bucket `record` accepts is one `list`
    /// serves.
    ///
    /// The defect this module was repaired for was exactly the asymmetry — the
    /// one bucket the user could write to was the one bucket nobody could read
    /// back. A writable bucket that `kind_for` does not know would recreate it.
    #[test]
    fn a_bucket_that_accepts_a_write_can_also_be_read() {
        for b in [
            "findings",
            "negative",
            "profiles",
            "questions",
            "preferences",
            "signals",
            "defects",
            "playbooks",
            "nonsense",
        ] {
            if user_writable_kind_for(b).is_some() {
                assert!(
                    kind_for(b).is_some(),
                    "`{b}` accepts a write and `list` does not serve it, so an entry could be \
                     recorded into a bucket nothing can read back"
                );
            }
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
