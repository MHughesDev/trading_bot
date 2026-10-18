//! The comments an author wrote on a strategy, kept beside its revision
//! (S-5.15; migration 0239).
//!
//! `POST /api/strategy-notes` replaces the notes of one revision;
//! `GET /api/strategy-notes/{revision_hash}` reads them. Keyed by the content hash
//! of the canonical definition, which contains no comment, so none of this can
//! change a revision's identity.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::{auth::BearerToken, state::AppState};

/// Most clauses a strategy has comments on, and most lines a note may carry. A
/// bound on what one call may write, not a judgement about comment length.
pub const MAX_NOTES: usize = 256;
pub const MAX_LINES: usize = 64;
pub const MAX_LINE_BYTES: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub clause: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PutBody {
    pub revision_hash: String,
    pub notes: Vec<Note>,
}

fn valid_hash(h: &str) -> bool {
    h.len() == 64
        && h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Replace the notes of `(user, revision)` with `notes`, in one transaction. An
/// empty list clears them.
///
/// # Errors
/// A message naming what was refused, or the database error.
pub async fn replace(
    pool: &PgPool,
    user: Uuid,
    revision_hash: &str,
    notes: &[Note],
) -> Result<usize, String> {
    if !valid_hash(revision_hash) {
        return Err("revision_hash must be 64 lowercase hex characters".into());
    }
    if notes.len() > MAX_NOTES {
        return Err(format!("at most {MAX_NOTES} notes per revision"));
    }
    for n in notes {
        if n.clause.trim().is_empty() || n.lines.is_empty() {
            return Err("a note needs a clause and at least one line".into());
        }
        if n.lines.len() > MAX_LINES || n.lines.iter().any(|l| l.len() > MAX_LINE_BYTES) {
            return Err(format!(
                "a note carries at most {MAX_LINES} lines of {MAX_LINE_BYTES} bytes"
            ));
        }
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM strategy_revision_notes WHERE user_id = $1 AND revision_hash = $2")
        .bind(user)
        .bind(revision_hash)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    for (i, n) in notes.iter().enumerate() {
        sqlx::query(
            "INSERT INTO strategy_revision_notes (user_id, revision_hash, clause, lines, position) \
             VALUES ($1,$2,$3,$4,$5) ON CONFLICT (user_id, revision_hash, clause) \
             DO UPDATE SET lines = EXCLUDED.lines, position = EXCLUDED.position, updated_at = now()",
        )
        .bind(user)
        .bind(revision_hash)
        .bind(&n.clause)
        .bind(&n.lines)
        .bind(i32::try_from(i).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(notes.len())
}

/// The notes of `(user, revision)`, in source order.
///
/// # Errors
/// The database error.
pub async fn load(
    pool: &PgPool,
    user: Uuid,
    revision_hash: &str,
) -> Result<Vec<Note>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT clause, lines FROM strategy_revision_notes \
          WHERE user_id = $1 AND revision_hash = $2 ORDER BY position, clause",
    )
    .bind(user)
    .bind(revision_hash)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| Note {
            clause: r.get("clause"),
            lines: r.get("lines"),
        })
        .collect())
}

/// POST /api/strategy-notes
pub async fn put(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<PutBody>,
) -> impl IntoResponse {
    match replace(&state.pg, token.user_id(), &body.revision_hash, &body.notes).await {
        Ok(n) => (
            StatusCode::OK,
            Json(json!({ "stored": n, "revision_hash": body.revision_hash })),
        ),
        Err(why) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "notes_refused", "why": why })),
        ),
    }
}

/// GET /api/strategy-notes/{revision_hash}
pub async fn get(
    State(state): State<AppState>,
    token: BearerToken,
    Path(revision_hash): Path<String>,
) -> impl IntoResponse {
    if !valid_hash(&revision_hash) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "revision_hash_malformed" })),
        );
    }
    match load(&state.pg, token.user_id(), &revision_hash).await {
        Ok(notes) => (
            StatusCode::OK,
            Json(json!({ "revision_hash": revision_hash, "notes": notes })),
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({ "error": "notes_unreadable", "message": e.to_string() })),
        ),
    }
}
