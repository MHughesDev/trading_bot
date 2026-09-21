//! Reading and writing a research session (AGENT-014 §2, §10; Set S task
//! S-11.1).
//!
//! The decisions live in `research_session`; this is the part that touches
//! Postgres. It is deliberately thin, and two things about it are load-bearing:
//!
//! * **[`append`] never chooses a `seq`.** The trigger does. Two appends racing
//!   cannot both claim the same position and neither has to know what the last
//!   one was, which is what makes the log safe to write from a driver that may
//!   be running twice during a hand-over.
//! * **[`claim`] is a conditional UPDATE, not a read followed by a write.** Only
//!   one caller can move an item out of a claimable status, so two drivers
//!   cannot start two children for one item. That, plus the derived conversation
//!   id in [`child_conversation_id`], is AT-59's guarantee for a child that is a
//!   conversation rather than a job.

use research_session::{
    agenda::{AgendaItem, Insert, ItemStatus, Score, Source},
    charter::{Autonomy, ReviewEvery, Schedule, SessionBudget},
    fold::{EventKind, SessionEvent},
    Charter, Folded, ResearchSession,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

/// The namespace the derived ids are minted in.
///
/// A fixed v5 namespace rather than `Uuid::new_v4`: the child conversation for
/// an agenda item must be the *same* conversation after a restart, and the only
/// way to guarantee that without a lookup that could itself be interrupted is to
/// derive it from facts that do not change.
const SESSION_NS: Uuid = Uuid::from_bytes([
    0x8f, 0x2a, 0x4c, 0x1e, 0x7b, 0x63, 0x4d, 0x91, 0xa5, 0x0c, 0x3e, 0x88, 0x11, 0xd7, 0x62, 0x40,
]);

/// The conversation an agenda item's child run happens in.
///
/// Derived from `(session_id, agenda_item_id)` — the pair AGENT-014 §3 names as
/// the idempotency key. A driver killed between creating the conversation and
/// recording that it did re-derives this exact id, finds the row already there,
/// and carries on; it cannot open a second conversation for the same item
/// because there is no second id to open it under.
#[must_use]
pub fn child_conversation_id(session_id: Uuid, item_id: Uuid) -> Uuid {
    let mut name = Vec::with_capacity(32);
    name.extend_from_slice(session_id.as_bytes());
    name.extend_from_slice(item_id.as_bytes());
    Uuid::new_v5(&SESSION_NS, &name)
}

#[derive(Debug, thiserror::Error)]
pub enum SessionStoreError {
    #[error("{0}")]
    Db(#[from] sqlx::Error),
    #[error("{0}")]
    Shape(#[from] research_session::SessionError),
    #[error("no session {0}")]
    NoSuchSession(Uuid),
}

// ── the session row ─────────────────────────────────────────────────────────

/// Write a session and open its log.
///
/// The `session_created` event is written in the same transaction as the row, so
/// a session with no log — which [`fold`](research_session::fold) would read as
/// a session in no state at all — cannot exist.
///
/// # Errors
///
/// The shape rule ([`ResearchSession::validate`]) is checked here as well as by
/// the table's CHECK. Both, because the CHECK is what holds against every writer
/// and this is what gives the caller a sentence rather than a constraint name.
pub async fn create(
    pg: &PgPool,
    user_id: Uuid,
    session: &ResearchSession,
) -> Result<(), SessionStoreError> {
    session.validate()?;
    let mut tx = pg.begin().await?;
    sqlx::query(
        "INSERT INTO research_sessions
             (session_id, user_id, conversation_id, continuous, question, charter,
              budget, schedule, autonomy, replenish, review_every, tier)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (session_id) DO NOTHING",
    )
    .bind(session.id)
    .bind(user_id)
    .bind(session.conversation_id)
    .bind(session.continuous)
    .bind(session.question.as_deref())
    .bind(session.charter.as_ref().map(|c| json!(c)))
    .bind(json!(session.budget))
    .bind(json!(session.schedule))
    .bind(session.autonomy.as_str())
    .bind(session.replenish)
    .bind(json!(session.review_every))
    .bind(&session.tier)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO session_events (session_id, kind, state, payload_json)
         SELECT $1, $2, $3, $4
          WHERE NOT EXISTS (SELECT 1 FROM session_events WHERE session_id = $1)",
    )
    .bind(session.id)
    .bind(EventKind::SessionCreated.as_str())
    .bind(EventKind::SessionCreated.moves_to())
    .bind(json!({ "continuous": session.continuous, "tier": session.tier }))
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// The session a conversation is.
///
/// Every Research conversation has one, which is why this returns a row rather
/// than a search: the unique index on `conversation_id` is what makes it a
/// lookup.
///
/// # Errors
///
/// Propagates database errors.
pub async fn for_conversation(
    pg: &PgPool,
    conversation_id: Uuid,
) -> Result<Option<ResearchSession>, SessionStoreError> {
    let row: Option<SessionRow> = sqlx::query_as(
        "SELECT session_id, conversation_id, continuous, question, charter, budget,
                schedule, autonomy, replenish, review_every, tier
           FROM research_sessions WHERE conversation_id = $1",
    )
    .bind(conversation_id)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(Into::into))
}

/// One session by id.
///
/// # Errors
///
/// Propagates database errors.
pub async fn get(pg: &PgPool, id: Uuid) -> Result<Option<ResearchSession>, SessionStoreError> {
    let row: Option<SessionRow> = sqlx::query_as(
        "SELECT session_id, conversation_id, continuous, question, charter, budget,
                schedule, autonomy, replenish, review_every, tier
           FROM research_sessions WHERE session_id = $1",
    )
    .bind(id)
    .fetch_optional(pg)
    .await?;
    Ok(row.map(Into::into))
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    session_id: Uuid,
    conversation_id: Uuid,
    continuous: bool,
    question: Option<String>,
    charter: Option<Value>,
    budget: Value,
    schedule: Value,
    autonomy: String,
    replenish: bool,
    review_every: Value,
    tier: String,
}

impl From<SessionRow> for ResearchSession {
    /// Reads what parses and falls back to the platform default for what does
    /// not.
    ///
    /// A row written before a field existed, or by an older build, still reads:
    /// the alternative is a session that cannot be loaded at all, which loses the
    /// log as well as the field. What it does **not** do is fall back on the
    /// charter — `charter: None` on a `continuous` row is left as it is, so
    /// [`ResearchSession::validate`] fails loudly rather than the session running
    /// under a charter this function invented.
    fn from(r: SessionRow) -> Self {
        Self {
            id: r.session_id,
            conversation_id: r.conversation_id,
            continuous: r.continuous,
            question: r.question,
            charter: r
                .charter
                .and_then(|c| serde_json::from_value::<Charter>(c).ok()),
            budget: serde_json::from_value::<SessionBudget>(r.budget).unwrap_or_default(),
            schedule: serde_json::from_value::<Schedule>(r.schedule).unwrap_or_default(),
            autonomy: Autonomy::parse(&r.autonomy).unwrap_or_default(),
            replenish: r.replenish,
            review_every: serde_json::from_value::<ReviewEvery>(r.review_every).unwrap_or_default(),
            tier: r.tier,
        }
    }
}

// ── the log ─────────────────────────────────────────────────────────────────

/// Append one event. The trigger assigns its position.
///
/// # Errors
///
/// A transition the table refuses comes back as a database error carrying the
/// trigger's sentence, which names both states. That is the refusal a caller
/// should surface rather than retry.
pub async fn append(
    pg: &PgPool,
    session_id: Uuid,
    kind: EventKind,
    payload: Value,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "INSERT INTO session_events (session_id, kind, state, payload_json)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(session_id)
    .bind(kind.as_str())
    .bind(kind.moves_to())
    .bind(payload)
    .execute(pg)
    .await?;
    Ok(())
}

/// Read the log and fold it.
///
/// # Errors
///
/// Propagates database errors. A row whose `kind` this build does not know is
/// **skipped**, and the fold reports the hole it leaves as an anomaly — which
/// stops the driver claiming rather than letting it act on a log it only
/// partly understood.
pub async fn state(pg: &PgPool, session_id: Uuid) -> Result<Folded, SessionStoreError> {
    let rows: Vec<(i64, String, Value)> = sqlx::query_as(
        "SELECT seq, kind, payload_json FROM session_events
          WHERE session_id = $1 ORDER BY seq",
    )
    .bind(session_id)
    .fetch_all(pg)
    .await?;
    let events: Vec<SessionEvent> = rows
        .into_iter()
        .filter_map(|(seq, kind, payload)| {
            EventKind::parse(&kind).map(|k| SessionEvent::new(session_id, seq, k, payload))
        })
        .collect();
    Ok(research_session::fold(&events))
}

// ── the agenda ──────────────────────────────────────────────────────────────

/// Insert an item, or report that its key already had one.
///
/// Layer one of the three-layer refusal (AGENT-014 P3).
///
/// **The decision is [`research_session::agenda::offer`]'s, not this
/// function's.** An earlier version of this file made the same decision again in
/// SQL, which meant the rule had two definitions and the tested one was the one
/// nothing called. So the live agenda is read inside the transaction, locked,
/// and handed to `offer`; what comes back says whether to insert or to extend.
/// The partial unique index stays as the backstop it should be — it holds
/// against every writer, including one that never goes through this function.
///
/// # Errors
///
/// Propagates database errors.
pub async fn insert_item(
    pg: &PgPool,
    item: &AgendaItem,
) -> Result<Option<Uuid>, SessionStoreError> {
    let mut tx = pg.begin().await?;
    // The live agenda, locked for the duration. Bounded by `max_runs` in
    // practice, and the lock is what makes "decide, then write" one step rather
    // than two with a race between them.
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT item_id, session_id, question, subject, workflow, sources, source_refs,
                dedupe_key, score, status, status_detail, depends_on, attempts, yield_ref
           FROM agenda_items
          WHERE session_id = $1 AND status <> ALL($2)
          ORDER BY created_at, item_id
          FOR UPDATE",
    )
    .bind(item.session_id)
    .bind(ItemStatus::TERMINAL_WORDS)
    .fetch_all(&mut *tx)
    .await?;
    let mut live: Vec<AgendaItem> = rows.into_iter().map(Into::into).collect();

    if let Insert::Merged { into, source } =
        research_session::agenda::offer(&mut live, item.clone())
    {
        // The key matched. Extend what wanted it and drop the score, which was
        // computed for a different source list (§5.4).
        sqlx::query(
            "UPDATE agenda_items
                SET sources = CASE WHEN sources @> $2::jsonb THEN sources ELSE sources || $2::jsonb END,
                    source_refs = source_refs || $3::jsonb,
                    score = NULL
              WHERE item_id = $1",
        )
        .bind(into)
        .bind(json!([source.as_str()]))
        .bind(json!(item.source_refs))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(None);
    }

    sqlx::query(
        "INSERT INTO agenda_items
             (item_id, session_id, question, subject, workflow, sources, source_refs,
              dedupe_key, status, depends_on)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(item.id)
    .bind(item.session_id)
    .bind(&item.question)
    .bind(&item.subject)
    .bind(&item.workflow)
    .bind(json!(item
        .sources
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()))
    .bind(json!(item.source_refs))
    .bind(&item.dedupe_key)
    .bind(item.status.word())
    .bind(json!(item.depends_on))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(item.id))
}

/// The agenda, oldest first — which is also §5.4's tie-break order.
///
/// # Errors
///
/// Propagates database errors.
pub async fn agenda(pg: &PgPool, session_id: Uuid) -> Result<Vec<AgendaItem>, SessionStoreError> {
    let rows: Vec<ItemRow> = sqlx::query_as(
        "SELECT item_id, session_id, question, subject, workflow, sources, source_refs,
                dedupe_key, score, status, status_detail, depends_on, attempts, yield_ref
           FROM agenda_items WHERE session_id = $1 ORDER BY created_at, item_id",
    )
    .bind(session_id)
    .fetch_all(pg)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct ItemRow {
    item_id: Uuid,
    session_id: Uuid,
    question: String,
    subject: String,
    workflow: String,
    sources: Value,
    source_refs: Value,
    dedupe_key: String,
    score: Option<Value>,
    status: String,
    status_detail: Value,
    depends_on: Value,
    attempts: i32,
    yield_ref: Option<String>,
}

impl From<ItemRow> for AgendaItem {
    fn from(r: ItemRow) -> Self {
        let sources: Vec<Source> = r
            .sources
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().and_then(Source::parse))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            id: r.item_id,
            session_id: r.session_id,
            question: r.question,
            subject: r.subject,
            workflow: r.workflow,
            // An item whose every source word is unreadable would otherwise
            // weigh zero and never be picked. `CharterSeed` is the neutral
            // weight (1.0), and the row is still visibly odd on the board.
            sources: if sources.is_empty() {
                vec![Source::CharterSeed]
            } else {
                sources
            },
            source_refs: r
                .source_refs
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            dedupe_key: r.dedupe_key,
            score: r.score.and_then(|s| serde_json::from_value::<Score>(s).ok()),
            status: status_from(&r.status, &r.status_detail),
            depends_on: r
                .depends_on
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                        .collect()
                })
                .unwrap_or_default(),
            attempts: r.attempts,
            yield_ref: r.yield_ref,
        }
    }
}

/// The status word plus whatever the word alone does not carry.
///
/// A word this build does not know reads as `Proposed` — the status that can do
/// the least. Reading it as anything else would let an unknown word act like a
/// claimable one.
fn status_from(word: &str, detail: &Value) -> ItemStatus {
    let id = |k: &str| {
        detail
            .get(k)
            .and_then(Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
    };
    let text = |k: &str| {
        detail
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("unstated")
            .to_string()
    };
    match word {
        "approved" => ItemStatus::Approved,
        "queued" => ItemStatus::Queued,
        "claimed" => id("run_id").map_or(ItemStatus::Queued, |run_id| ItemStatus::Claimed { run_id }),
        "running" => id("run_id").map_or(ItemStatus::Queued, |run_id| ItemStatus::Running { run_id }),
        "done" => ItemStatus::Done {
            verdict: text("verdict"),
        },
        "vetoed" => ItemStatus::Vetoed { by: text("by") },
        "superseded" => id("by_item").map_or(ItemStatus::Proposed, |by_item| {
            ItemStatus::Superseded { by_item }
        }),
        "expired" => ItemStatus::Expired { why: text("why") },
        _ => ItemStatus::Proposed,
    }
}

/// Move an item to a new status.
///
/// # Errors
///
/// Propagates database errors.
pub async fn set_status(
    pg: &PgPool,
    item_id: Uuid,
    status: &ItemStatus,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "UPDATE agenda_items
            SET status = $2, status_detail = $3,
                finished_at = CASE WHEN $2 IN ('done','vetoed','superseded','expired')
                                   THEN now() ELSE finished_at END
          WHERE item_id = $1",
    )
    .bind(item_id)
    .bind(status.word())
    .bind(detail_of(status))
    .execute(pg)
    .await?;
    Ok(())
}

fn detail_of(status: &ItemStatus) -> Value {
    match status {
        ItemStatus::Claimed { run_id } | ItemStatus::Running { run_id } => {
            json!({ "run_id": run_id })
        }
        ItemStatus::Done { verdict } => json!({ "verdict": verdict }),
        ItemStatus::Vetoed { by } => json!({ "by": by }),
        ItemStatus::Superseded { by_item } => json!({ "by_item": by_item }),
        ItemStatus::Expired { why } => json!({ "why": why }),
        _ => json!({}),
    }
}

/// Take an item, if it is still there to take.
///
/// A conditional UPDATE and not a read-then-write: two drivers folding the same
/// log reach the same conclusion about what to claim next, and exactly one of
/// them gets it. Returns whether this caller is the one that did.
///
/// # Errors
///
/// Propagates database errors.
pub async fn claim(
    pg: &PgPool,
    item_id: Uuid,
    run_id: Uuid,
) -> Result<bool, SessionStoreError> {
    let done = sqlx::query(
        "UPDATE agenda_items
            SET status = 'claimed', status_detail = $2, claimed_at = now()
          WHERE item_id = $1 AND status IN ('approved', 'queued')",
    )
    .bind(item_id)
    .bind(json!({ "run_id": run_id }))
    .execute(pg)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Raise an item's attempt count and put it back, or expire it.
///
/// AGENT-014 §6.6: one retry, then the item is `expired{why}` and the reason is
/// in the digest. Returns the status it ended at, so the caller writes the event
/// that matches what happened rather than guessing.
///
/// # Errors
///
/// Propagates database errors.
pub async fn requeue_or_expire(
    pg: &PgPool,
    item_id: Uuid,
    why: &str,
) -> Result<ItemStatus, SessionStoreError> {
    let row: Option<(i32,)> = sqlx::query_as("SELECT attempts FROM agenda_items WHERE item_id = $1")
        .bind(item_id)
        .fetch_optional(pg)
        .await?;
    let attempts = row.map_or(0, |r| r.0) + 1;
    if attempts >= research_session::agenda::MAX_ATTEMPTS {
        let status = ItemStatus::Expired {
            why: why.to_string(),
        };
        sqlx::query(
            "UPDATE agenda_items SET attempts = $2, status = 'expired', status_detail = $3,
                                     finished_at = now()
              WHERE item_id = $1",
        )
        .bind(item_id)
        .bind(attempts)
        .bind(json!({ "why": why }))
        .execute(pg)
        .await?;
        return Ok(status);
    }
    sqlx::query(
        "UPDATE agenda_items SET attempts = $2, status = 'queued', status_detail = '{}'::jsonb
          WHERE item_id = $1",
    )
    .bind(item_id)
    .bind(attempts)
    .execute(pg)
    .await?;
    Ok(ItemStatus::Queued)
}

/// Attach a finished run's verdict to its item.
///
/// # Errors
///
/// Propagates database errors.
pub async fn finish_item(
    pg: &PgPool,
    item_id: Uuid,
    verdict: &str,
    yield_ref: Option<&str>,
) -> Result<(), SessionStoreError> {
    sqlx::query(
        "UPDATE agenda_items
            SET status = 'done', status_detail = $2, yield_ref = $3, finished_at = now()
          WHERE item_id = $1",
    )
    .bind(item_id)
    .bind(json!({ "verdict": verdict }))
    .bind(yield_ref)
    .execute(pg)
    .await?;
    Ok(())
}

/// Stamp a run with the session and item it belongs to (AGENT-010 §4 amended).
///
/// # Errors
///
/// Propagates database errors.
pub async fn attribute_run(
    pg: &PgPool,
    run_id: Uuid,
    session_id: Uuid,
    item_id: Uuid,
) -> Result<(), SessionStoreError> {
    sqlx::query("UPDATE agent_runs SET session_id = $2, agenda_item_id = $3 WHERE run_id = $1")
        .bind(run_id)
        .bind(session_id)
        .bind(item_id)
        .execute(pg)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_conversation_id_is_the_same_one_after_a_restart() {
        let s = Uuid::new_v4();
        let i = Uuid::new_v4();
        assert_eq!(child_conversation_id(s, i), child_conversation_id(s, i));
        assert_ne!(child_conversation_id(s, i), child_conversation_id(s, Uuid::new_v4()));
        assert_ne!(child_conversation_id(s, i), child_conversation_id(Uuid::new_v4(), i));
        // Not symmetric: swapping the two must not collide, which a naive XOR or
        // a sorted concatenation would.
        assert_ne!(child_conversation_id(s, i), child_conversation_id(i, s));
    }

    #[test]
    fn an_unknown_status_word_reads_as_the_one_that_can_do_least() {
        assert_eq!(status_from("something_new", &json!({})), ItemStatus::Proposed);
        assert!(!status_from("something_new", &json!({})).is_claimable());
        // And a claimed row with no run id is not readable as claimed, because a
        // claim with no run is a claim nothing can be resumed from.
        assert_eq!(status_from("claimed", &json!({})), ItemStatus::Queued);
    }

    #[test]
    fn a_status_round_trips_through_its_word_and_detail() {
        let run = Uuid::new_v4();
        for status in [
            ItemStatus::Approved,
            ItemStatus::Queued,
            ItemStatus::Claimed { run_id: run },
            ItemStatus::Running { run_id: run },
            ItemStatus::Done {
                verdict: "supported".into(),
            },
            ItemStatus::Vetoed { by: "mason".into() },
            ItemStatus::Superseded { by_item: run },
            ItemStatus::Expired {
                why: "two failures".into(),
            },
        ] {
            assert_eq!(
                status_from(status.word(), &detail_of(&status)),
                status,
                "{status:?} did not survive the column"
            );
        }
    }
}
