//! The dossier reader's API (AGENT-006 §5.8, AGENT-012 §5; Set S task S-8.7).
//!
//! `GET /api/dossiers/{ticket}`, `POST /api/dossiers/{ticket}/verify/{claim}`,
//! and the comment thread.
//!
//! # A caller that had no callee
//!
//! `frontend/src/components/desk/Dossier.tsx` has called `/api/dossiers/{ticket}`
//! since S-6.17 and rendered an empty state naming the route that did not exist.
//! That is the "built but unreached" defect with the halves swapped — a caller
//! with no callee across an HTTP boundary, where no compiler can see it — and it
//! is why `crates/invariants/tests/every_client_path_is_served.rs` exists.
//!
//! # `verify` re-derives; it does not re-read
//!
//! The whole value of a verify button is that it can disagree with the document.
//! So it does not return the stored `verification` — it re-renders the evidence
//! the claim aligned to **from the platform as it stands now**, re-runs the
//! alignment, and compares.
//!
//! Three answers, and they are genuinely different:
//!
//! * the claim still aligns the same way — it holds;
//! * it aligns *worse* than the dossier says — the evidence moved under it, and
//!   the document is now overstating what it can show;
//! * the evidence id no longer resolves — nothing can be said either way, which
//!   is not the same as a failure and must not be reported as one.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use harness::report::claims::{extract, Evidence, Method};

use crate::{auth::BearerToken, state::AppState};

fn method_word(m: Method) -> &'static str {
    match m {
        Method::DirectStat => "direct_stat",
        Method::Derived => "derived",
        Method::Unverified => "unverified",
    }
}

/// How strong a verification is. A recheck that comes back weaker than the
/// document claimed is a failed recheck, and one that comes back stronger is
/// not.
fn strength(word: &str) -> u8 {
    match word {
        "direct_stat" => 2,
        "derived" => 1,
        _ => 0,
    }
}

/// `GET /api/dossiers/{ticket}`
pub async fn get(
    State(state): State<AppState>,
    token: BearerToken,
    Path(ticket): Path<String>,
) -> impl IntoResponse {
    let row: Result<Option<(Value,)>, sqlx::Error> =
        sqlx::query_as("SELECT body FROM mlops.dossier WHERE tenant_id = $1 AND ticket = $2")
            .bind(token.user_id().to_string())
            .bind(&ticket)
            .fetch_optional(&state.pg)
            .await;

    match row {
        Ok(Some((body,))) => (StatusCode::OK, Json(body)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "no_dossier",
                "ticket": ticket,
                "fix": "a dossier is compiled at the end of the run that produced it. A run that \
                        ended without one had a claim its evidence did not support, or no \
                        critique — the run's events say which"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "dossier_unreadable", "message": e.to_string() })),
        )
            .into_response(),
    }
}

/// `POST /api/dossiers/{ticket}/verify/{claim}`
///
/// `claim` is the evidence id the claim aligned to, which is what the desk's
/// table has in hand for each row.
pub async fn verify(
    State(state): State<AppState>,
    token: BearerToken,
    Path((ticket, claim)): Path<(String, String)>,
) -> impl IntoResponse {
    let tenant = token.user_id().to_string();
    let row: Result<Option<(Value, Value, Option<uuid::Uuid>)>, sqlx::Error> = sqlx::query_as(
        "SELECT body, evidence, run_id FROM mlops.dossier WHERE tenant_id = $1 AND ticket = $2",
    )
    .bind(&tenant)
    .bind(&ticket)
    .fetch_optional(&state.pg)
    .await;

    let (body, stored_evidence, run_id) =
        match row {
            Ok(Some(v)) => v,
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({ "ok": false, "note": "there is no dossier with that ticket" })),
                )
                    .into_response()
            }
            Err(e) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "note": format!("the dossier could not be read: {e}") })),
            )
                .into_response(),
        };

    // The claim as the document states it, and how the document says it was
    // verified.
    let Some(stated) = body
        .get("claims")
        .and_then(Value::as_array)
        .and_then(|cs| {
            cs.iter()
                .find(|c| c.get("evidence_id").and_then(Value::as_str) == Some(claim.as_str()))
        })
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "note": format!("this dossier has no claim citing `{claim}`")
            })),
        )
            .into_response();
    };
    let sentence = stated
        .get("claim")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let was = stated
        .get("verification")
        .and_then(Value::as_str)
        .unwrap_or("unverified")
        .to_string();

    // Re-render the evidence from the platform as it stands. A run whose
    // metrics have been recomputed since is the case this exists to catch.
    let live = live_evidence(&state, token.user_id(), &claim).await;
    let Some(live) = live else {
        // Explicitly not a failure. "The evidence no longer resolves" and "the
        // claim was never supported" are different answers, and collapsing them
        // would have a reviewer discard a sound dossier because a run was
        // archived.
        let stored = stored_evidence
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|e| e.get("id").and_then(Value::as_str) == Some(claim.as_str()))
            })
            .and_then(|e| e.get("text").and_then(Value::as_str))
            .map(str::to_string);
        return (
            StatusCode::OK,
            Json(json!({
                "ok": false,
                "note": match stored {
                    Some(_) => format!(
                        "`{claim}` no longer resolves on this platform, so the claim cannot be \
                         re-derived. The bundle stored with the dossier still holds what it said \
                         at the time; this is not a failed recheck, it is an absent one"
                    ),
                    None => format!(
                        "`{claim}` resolves to nothing now and was not in the dossier's stored \
                         bundle either, which means this claim was never re-derivable"
                    ),
                },
                "run_id": run_id,
            })),
        )
            .into_response();
    };

    let redone = extract(&sentence, &[live.clone()]);
    let now = redone
        .claims
        .first()
        .map_or("unverified", |c| method_word(c.method));

    let ok = strength(now) >= strength(&was);
    (
        StatusCode::OK,
        Json(json!({
            "ok": ok,
            "note": if ok {
                format!(
                    "re-derived against `{claim}` as it stands now: {now} (the dossier says {was})"
                )
            } else {
                format!(
                    "this claim no longer holds. The dossier records it as {was}; re-deriving \
                     against `{claim}` as it stands now gives {now}. The evidence has moved \
                     under the document"
                )
            },
            "was": was,
            "now": now,
            "evidence_id": claim,
        })),
    )
        .into_response()
}

/// Render one evidence id from the platform as it stands.
///
/// Two kinds resolve: a Run (its metrics, re-read) and a memory entry (its claim
/// and measurement). Anything else is `None`, which the caller reports as an
/// *absent* recheck rather than a failed one.
async fn live_evidence(state: &AppState, user_id: uuid::Uuid, id: &str) -> Option<Evidence> {
    if let Some(result) = state.suite.run_result(user_id, id) {
        return Some(Evidence {
            id: id.to_string(),
            text: crate::dossier::render(&serde_json::to_value(&result.metrics).ok()?),
        });
    }
    type Measured = (String, Option<f64>, Option<f64>, Option<i64>);
    let row: Measured = sqlx::query_as::<_, Measured>(
        "SELECT claim, value, se, n FROM agent_memory.entry
          WHERE user_id = $1 AND entry_id = $2 AND NOT archived",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten()?;
    let (claim, value, se, n) = row;
    Some(Evidence {
        id: id.to_string(),
        text: crate::dossier::render(&json!({
            "claim": claim,
            "value": value,
            "se": se,
            "n": n,
        })),
    })
}

/// `GET /api/dossiers/{ticket}/comments`
pub async fn comments(
    State(state): State<AppState>,
    token: BearerToken,
    Path(ticket): Path<String>,
) -> impl IntoResponse {
    let rows: Result<
        Vec<(
            uuid::Uuid,
            String,
            Option<String>,
            String,
            chrono::DateTime<chrono::Utc>,
        )>,
        sqlx::Error,
    > = sqlx::query_as(
        "SELECT comment_id, author, claim, body, created_at
               FROM mlops.dossier_comment
              WHERE tenant_id = $1 AND ticket = $2
              ORDER BY created_at",
    )
    .bind(token.user_id().to_string())
    .bind(&ticket)
    .fetch_all(&state.pg)
    .await;

    match rows {
        Ok(rows) => (
            StatusCode::OK,
            Json(json!({
                "comments": rows
                    .into_iter()
                    .map(|(id, author, claim, body, at)| json!({
                        "comment_id": id,
                        "author": author,
                        "claim": claim,
                        "body": body,
                        "created_at": at,
                    }))
                    .collect::<Vec<_>>()
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "comments_unreadable", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct CommentBody {
    /// The claim this is about, when it is about one.
    #[serde(default)]
    pub claim: Option<String>,
    pub body: String,
}

/// `POST /api/dossiers/{ticket}/comments`
///
/// The author is the session's, never the body's. A comment attributed to
/// somebody else is worse than no comment.
pub async fn add_comment(
    State(state): State<AppState>,
    token: BearerToken,
    Path(ticket): Path<String>,
    Json(body): Json<CommentBody>,
) -> impl IntoResponse {
    if body.body.trim().is_empty() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "empty_comment" })),
        )
            .into_response();
    }
    let res = sqlx::query(
        "INSERT INTO mlops.dossier_comment (ticket, tenant_id, author, claim, body)
         VALUES ($1,$2,$3,$4,$5)",
    )
    .bind(&ticket)
    .bind(token.user_id().to_string())
    .bind(token.user_id().to_string())
    .bind(&body.claim)
    .bind(body.body.trim())
    .execute(&state.pg)
    .await;

    match res {
        Ok(_) => (StatusCode::CREATED, Json(json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "comment_not_saved", "message": e.to_string() })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recheck that comes back weaker than the document claimed has failed. A
    /// recheck that comes back *stronger* has not — the evidence improved, and
    /// refusing over that would train reviewers to ignore the button.
    #[test]
    fn a_recheck_fails_only_when_the_claim_got_weaker() {
        assert!(strength("direct_stat") > strength("derived"));
        assert!(strength("derived") > strength("unverified"));
        assert!(strength("anything_else") == 0);

        // was → now, and whether that is a pass.
        for (was, now, ok) in [
            ("direct_stat", "direct_stat", true),
            ("derived", "direct_stat", true),
            ("derived", "derived", true),
            ("direct_stat", "derived", false),
            ("derived", "unverified", false),
            ("unverified", "unverified", true),
        ] {
            assert_eq!(
                strength(now) >= strength(was),
                ok,
                "{was} → {now} should be ok={ok}"
            );
        }
    }

    /// The three `Method`s map to the three words the desk renders, and nothing
    /// else. A fourth spelling would render as an unstyled cell.
    #[test]
    fn the_verification_words_are_the_ones_the_desk_knows() {
        for (m, w) in [
            (Method::DirectStat, "direct_stat"),
            (Method::Derived, "derived"),
            (Method::Unverified, "unverified"),
        ] {
            assert_eq!(method_word(m), w);
            assert!(strength(w) <= 2);
        }
    }
}
