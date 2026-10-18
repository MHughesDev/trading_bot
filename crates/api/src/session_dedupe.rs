//! Two sessions, one question, one item (AGENT-014 P3, §4, §12.2; Set S task
//! S-11.4; migration 0164; ADR-P11-07).
//!
//! # What was missing
//!
//! Layer one of the duplicate refusal is the partial unique index
//! `agenda_items_live_key (session_id, dedupe_key)`: one live item per key
//! **within a session**. The acceptance clause says *two sessions with
//! overlapping scope produce one item with two sources*, and with the key per
//! session two continuous sessions of one user each queued the question and each
//! spent trials on it.
//!
//! # What this does
//!
//! [`offer`] is `sessions::insert_item` with one more question asked first: does
//! another **live** session of the same user already hold a **live** item with
//! this key? If so the offer merges onto it — the item's `sources` list is
//! extended (one item, two sources), the offering session is recorded as a
//! sponsor in `agenda_item_sponsors`, and the offering session's log says where
//! the question went. The item stays on the board where it was first raised.
//!
//! # Three limits, each chosen
//!
//! * **Same user, same key.** Two sessions can have different charters for the
//!   same question and the key is `normalise(question) + subject`, which is what
//!   makes them the same work. Different users never merge: a question is
//!   private to the person who asked it.
//! * **A `user`-sourced offer is never moved.** A human who typed a question into
//!   *this* session's agenda asked this session. It runs here, with the bypass
//!   that §5.4 gives it; merging it away would silently reroute an instruction.
//! * **Only live sessions and live items.** A stopped or complete session's board
//!   is not a place a question will be answered, and a finished item is a
//!   question that may be asked again (replication is what the platform is for).
//!
//! The advisory lock on the key is what stops two drivers offering the same
//! question in the same millisecond from each finding nothing and each inserting;
//! the per-session unique index is still the backstop and still holds.

use research_session::agenda::{AgendaItem, ItemStatus, Source};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::sessions::{self, SessionStoreError};

/// What became of an offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offered {
    /// Handled inside the offering session: inserted (`Some(id)`) or merged onto
    /// one of its own items (`None`) — `sessions::insert_item`'s own answer.
    Here(Option<Uuid>),
    /// Merged onto another live session's live item.
    Elsewhere { item: Uuid, owner: Uuid },
}

/// The sessions whose states can still answer a question.
const LIVE_SESSION_STATES: &[&str] = &["running", "paused", "stalled"];

/// Offer an item to the agenda, across sessions.
///
/// # Errors
/// Database errors.
pub async fn offer(
    pg: &PgPool,
    user_id: Uuid,
    item: &AgendaItem,
) -> Result<Offered, SessionStoreError> {
    // A human's own question is never rerouted, and a session with no foreign
    // neighbour pays one indexed lookup.
    if item.sources.contains(&Source::User) {
        return Ok(Offered::Here(sessions::insert_item(pg, item).await?));
    }

    let mut tx = pg.begin().await?;
    // One writer per key at a time, across sessions. `hashtextextended` over the
    // user and the key, so unrelated questions never wait on each other.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || ':' || $2, 0))")
        .bind(user_id.to_string())
        .bind(&item.dedupe_key)
        .execute(&mut *tx)
        .await?;

    // This session's own live item with the key wins: layer one is the one that
    // handles it (a merge onto the item already here).
    let local: Option<(Uuid,)> = sqlx::query_as(
        "SELECT item_id FROM agenda_items
          WHERE session_id = $1 AND dedupe_key = $2 AND status <> ALL($3) LIMIT 1",
    )
    .bind(item.session_id)
    .bind(&item.dedupe_key)
    .bind(ItemStatus::TERMINAL_WORDS)
    .fetch_optional(&mut *tx)
    .await?;
    if local.is_some() {
        tx.rollback().await?;
        return Ok(Offered::Here(sessions::insert_item(pg, item).await?));
    }

    let foreign: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT ai.item_id, ai.session_id
           FROM agenda_items ai
           JOIN research_sessions rs ON rs.session_id = ai.session_id
          WHERE rs.user_id = $1 AND ai.session_id <> $2 AND ai.dedupe_key = $3
            AND ai.status <> ALL($4)
            AND COALESCE(
                  (SELECT state FROM session_events
                    WHERE session_id = ai.session_id AND state IS NOT NULL
                    ORDER BY seq DESC LIMIT 1), 'draft') = ANY($5)
          ORDER BY ai.created_at, ai.item_id LIMIT 1
          FOR UPDATE OF ai",
    )
    .bind(user_id)
    .bind(item.session_id)
    .bind(&item.dedupe_key)
    .bind(ItemStatus::TERMINAL_WORDS)
    .bind(LIVE_SESSION_STATES)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((owner_item, owner_session)) = foreign else {
        tx.rollback().await?;
        return Ok(Offered::Here(sessions::insert_item(pg, item).await?));
    };

    // One item, two sources: every source the offer carried that the item does
    // not already have, and the offer's refs. The score is dropped because it was
    // computed for a different source list (§5.4).
    for source in &item.sources {
        sqlx::query(
            "UPDATE agenda_items
                SET sources = CASE WHEN sources @> $2::jsonb THEN sources ELSE sources || $2::jsonb END,
                    score = NULL
              WHERE item_id = $1",
        )
        .bind(owner_item)
        .bind(json!([source.as_str()]))
        .execute(&mut *tx)
        .await?;
    }
    let mut refs = item.source_refs.clone();
    refs.push(format!("session:{}", item.session_id));
    sqlx::query(
        "UPDATE agenda_items
            SET source_refs = source_refs || $2::jsonb
          WHERE item_id = $1",
    )
    .bind(owner_item)
    .bind(json!(refs))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO agenda_item_sponsors (item_id, session_id, source, source_refs)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (item_id, session_id) DO UPDATE
             SET source_refs = agenda_item_sponsors.source_refs || EXCLUDED.source_refs",
    )
    .bind(owner_item)
    .bind(item.session_id)
    .bind(item.sources.first().map_or("unknown", |s| s.as_str()))
    .bind(json!(item.source_refs))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(Offered::Elsewhere {
        item: owner_item,
        owner: owner_session,
    })
}

/// The sessions that also wanted `item` and have not been told how it ended.
///
/// # Errors
/// Database errors.
pub async fn untold_sponsors(pg: &PgPool, item: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT session_id FROM agenda_item_sponsors
          WHERE item_id = $1 AND told_at IS NULL ORDER BY offered_at",
    )
    .bind(item)
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

/// Mark a sponsor as told. `Ok(false)` when another driver already did.
///
/// # Errors
/// Database errors.
pub async fn mark_told(pg: &PgPool, item: Uuid, session: Uuid) -> Result<bool, sqlx::Error> {
    let n = sqlx::query(
        "UPDATE agenda_item_sponsors SET told_at = now()
          WHERE item_id = $1 AND session_id = $2 AND told_at IS NULL",
    )
    .bind(item)
    .bind(session)
    .execute(pg)
    .await?
    .rows_affected();
    Ok(n == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_states_that_can_still_answer_a_question_are_the_three_that_are_not_over() {
        // `stopped` is terminal and `complete` is a board with nothing left on
        // it; `draft` has not started. If the fold's vocabulary changes this has
        // to be looked at, which is why it is a test and not a comment.
        for s in research_session::SessionState::WORDS {
            let live = LIVE_SESSION_STATES.contains(s);
            let expected = matches!(*s, "running" | "paused" | "stalled");
            assert_eq!(live, expected, "{s}");
        }
    }
}
