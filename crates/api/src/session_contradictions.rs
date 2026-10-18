//! The contradiction pair a session can read (AGENT-014 §4.5; AGENT-007 §2.2.1;
//! Set S task S-11.2; migration 0161).
//!
//! # What was missing
//!
//! Source 4.5 turns *"two findings the platform has decided conflict"* into a
//! `diagnose` item, and the platform held a **count**
//! (`agent_runs.contradictions_surfaced`) and a **list on one entry**
//! (`agent_memory.entry.conflicts_with`) but never the pair with the question it
//! raises. So the subscription was asked nothing and said so on the log, and a
//! card for it would have had nothing to open.
//!
//! # What this does
//!
//! [`refresh`] materialises a pair for every live entry that lists another live
//! entry in its `conflicts_with`, ordered so `{a, b}` and `{b, a}` are one row,
//! and resolves a pair when either side is archived or superseded — the two ways
//! memory itself says a claim is no longer standing. [`open_pairs`] reads the
//! unresolved ones as `research_session` wants them.
//!
//! Nothing here picks a winner. §2.2.1 says conflicts are surfaced and never
//! merged, the memory schema has no `winner` column on purpose, and the pair's
//! only resolution is one of its entries leaving the live set — which is a
//! research act (the item is a `diagnose`) and not this module's decision.

use research_session::sources::memory::ContradictionPair;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// How much of each claim goes into the question. Enough to recognise, short
/// enough that the card is a card.
const CLAIM_CHARS: usize = 160;

/// What one refresh did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Refreshed {
    pub found: u32,
    pub resolved: u32,
}

/// The question a pair raises, composed from the two claims.
#[must_use]
pub fn question_for(subject: &str, left: &str, right: &str) -> String {
    let clip = |s: &str| -> String {
        let s = s.trim();
        if s.chars().count() <= CLAIM_CHARS {
            s.to_string()
        } else {
            let mut out: String = s.chars().take(CLAIM_CHARS).collect();
            out.push('…');
            out
        }
    };
    let on = if subject.trim().is_empty() {
        String::new()
    } else {
        format!(" about {}", subject.trim())
    };
    format!(
        "Two findings{on} disagree. Which one holds, and what would settle it? A: \"{}\" B: \"{}\"",
        clip(left),
        clip(right)
    )
}

/// Order a pair so it has one identity whichever entry listed the other.
#[must_use]
pub fn ordered<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Find new pairs and resolve the ones memory has since settled.
///
/// # Errors
/// Database errors.
pub async fn refresh(pg: &PgPool, user_id: Uuid) -> Result<Refreshed, sqlx::Error> {
    type Candidate = (String, String, String, String, String);
    // Live on both sides, and not already recorded in either order.
    let candidates: Vec<Candidate> = sqlx::query_as(
        "SELECT e.entry_id, o.entry_id, e.subject, e.claim, o.claim
           FROM agent_memory.entry e
           CROSS JOIN LATERAL unnest(e.conflicts_with) AS c(other)
           JOIN agent_memory.entry o ON o.entry_id = c.other AND o.user_id = e.user_id
          WHERE e.user_id = $1
            AND NOT e.archived AND e.superseded_by IS NULL
            AND NOT o.archived AND o.superseded_by IS NULL
            AND e.entry_id <> o.entry_id
            AND NOT EXISTS (
                SELECT 1 FROM session_contradictions p
                 WHERE p.user_id = e.user_id
                   AND p.left_entry = LEAST(e.entry_id, o.entry_id)
                   AND p.right_entry = GREATEST(e.entry_id, o.entry_id))
          LIMIT 200",
    )
    .bind(user_id)
    .fetch_all(pg)
    .await?;

    let mut found = 0;
    for (a, b, subject, claim_a, claim_b) in candidates {
        let (left, right, claim_left, claim_right) = if a <= b {
            (a, b, claim_a, claim_b)
        } else {
            (b, a, claim_b, claim_a)
        };
        let n = sqlx::query(
            "INSERT INTO session_contradictions (user_id, left_entry, right_entry, subject, question)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (user_id, left_entry, right_entry) DO NOTHING",
        )
        .bind(user_id)
        .bind(&left)
        .bind(&right)
        .bind(&subject)
        .bind(question_for(&subject, &claim_left, &claim_right))
        .execute(pg)
        .await?
        .rows_affected();
        found += u32::try_from(n).unwrap_or(0);
    }

    let resolved = sqlx::query(
        "UPDATE session_contradictions p
            SET resolved_at = now(),
                resolved_why = 'one of the two entries is no longer standing: it was archived or \
                                superseded'
          WHERE p.user_id = $1 AND p.resolved_at IS NULL
            AND EXISTS (
                SELECT 1 FROM agent_memory.entry e
                 WHERE e.user_id = p.user_id
                   AND e.entry_id IN (p.left_entry, p.right_entry)
                   AND (e.archived OR e.superseded_by IS NOT NULL))",
    )
    .bind(user_id)
    .execute(pg)
    .await?
    .rows_affected();

    Ok(Refreshed {
        found,
        resolved: u32::try_from(resolved).unwrap_or(0),
    })
}

/// The unresolved pairs, as source 4.5 reads them.
///
/// # Errors
/// Database errors.
pub async fn open_pairs(pg: &PgPool, user_id: Uuid) -> Result<Vec<ContradictionPair>, sqlx::Error> {
    let rows: Vec<(String, String, String, String)> = sqlx::query_as(
        "SELECT left_entry, right_entry, subject, question
           FROM session_contradictions
          WHERE user_id = $1 AND resolved_at IS NULL
          ORDER BY found_at, pair_id LIMIT 200",
    )
    .bind(user_id)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(l, r, s, q)| ContradictionPair::new(&l, &r, &s, &q))
        .collect())
}

/// One pair, as the desk lists it.
#[derive(Debug, Clone, Serialize)]
pub struct PairView {
    pub pair_id: Uuid,
    pub left_entry: String,
    pub right_entry: String,
    pub subject: String,
    pub question: String,
    pub found_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
    pub resolved_why: Option<String>,
}

/// Every pair of this user's, newest first — what "opens in the desk" opens.
///
/// # Errors
/// Database errors.
pub async fn list(pg: &PgPool, user_id: Uuid) -> Result<Vec<PairView>, sqlx::Error> {
    type Row = (
        Uuid,
        String,
        String,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<String>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT pair_id, left_entry, right_entry, subject, question, found_at, resolved_at,
                resolved_why
           FROM session_contradictions WHERE user_id = $1
          ORDER BY found_at DESC, pair_id LIMIT 200",
    )
    .bind(user_id)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(pair_id, left_entry, right_entry, subject, question, found_at, resolved_at, why)| {
                PairView {
                    pair_id,
                    left_entry,
                    right_entry,
                    subject,
                    question,
                    found_at,
                    resolved_at,
                    resolved_why: why,
                }
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_has_one_identity_whichever_entry_listed_the_other() {
        assert_eq!(ordered("f_b", "f_a"), ("f_a", "f_b"));
        assert_eq!(ordered("f_a", "f_b"), ("f_a", "f_b"));
        assert_eq!(
            ContradictionPair::new("f_b", "f_a", "BTC", "q").key(),
            ContradictionPair::new("f_a", "f_b", "BTC", "q").key()
        );
    }

    #[test]
    fn the_question_names_both_claims_and_clips_a_long_one() {
        let long = "x".repeat(400);
        let q = question_for("BTC-USD", "momentum works", &long);
        assert!(q.contains("BTC-USD") && q.contains("momentum works"));
        assert!(q.chars().count() < 400, "the card is a card: {}", q.len());
        assert!(q.contains('…'));
        let no_subject = question_for("  ", "a", "b");
        assert!(!no_subject.contains("about"), "{no_subject}");
    }
}
