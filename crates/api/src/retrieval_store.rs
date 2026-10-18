//! The retrieval log, and the recall share computed from it
//! (AGENT-007 §4; AGENT-009 §4; Set S task S-4.5).
//!
//! # What the number means, and what it does not
//!
//! `memory_recall` is the share of retrieved entries a run went on to cite. It
//! is a measure of **the retrieval policy**, not of the model: a low share means
//! the brief put the wrong things in front of it, and the fix is in the scorer.
//!
//! That reading only holds if the denominator is honest, which is why [`log`]
//! writes a row for every item the brief showed — including the ones nobody
//! expects to be cited. A log of hits is not a measurement.
//!
//! # `None` and `0.0` are different answers
//!
//! [`recall`] returns `None` when a run retrieved nothing at all, and `Some(0.0)`
//! when it retrieved things and cited none. A run with no memory to draw on has
//! not failed at recall; a run that ignored everything it was shown has. Folding
//! the two together would make the score improve whenever the store was empty,
//! which is the direction that hides the problem.

use harness::memory::brief::{Retrieval, Slot};
use sqlx::{PgPool, Row};

#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

fn slot_str(s: Slot) -> &'static str {
    match s {
        Slot::UserPreference => "user_preference",
        Slot::InstrumentProfile => "instrument_profile",
        Slot::Finding => "finding",
        Slot::NegativeResult => "negative_result",
        Slot::OpenQuestion => "open_question",
        Slot::Conflict => "conflict",
    }
}

pub struct RetrievalStore {
    pool: PgPool,
}

impl RetrievalStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Record what the brief put in front of the model.
    ///
    /// Idempotent per `(run_id, entry_id)`: a brief assembled twice in one run —
    /// a re-plan, a resumed step — must not count the same entry twice, or a run
    /// that re-planned would score a different recall from one that did not for
    /// reasons having nothing to do with memory. The re-assembly's rank wins,
    /// because it is the one the model actually saw last.
    ///
    /// # Errors
    /// [`RetrievalError::Sqlx`] if the write fails.
    pub async fn log(&self, retrievals: &[Retrieval]) -> Result<(), RetrievalError> {
        for r in retrievals {
            sqlx::query(
                "INSERT INTO memory_retrievals (run_id, entry_id, rank, slot, why) \
                 VALUES ($1,$2,$3,$4,$5) \
                 ON CONFLICT (run_id, entry_id) DO UPDATE SET \
                   rank = EXCLUDED.rank, slot = EXCLUDED.slot, why = EXCLUDED.why",
            )
            .bind(&r.run_id)
            .bind(&r.entry_id)
            .bind(i32::try_from(r.rank).unwrap_or(i32::MAX).max(1))
            .bind(slot_str(r.slot))
            .bind(&r.why)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    /// Mark entries this run cited, and report how many rows that moved.
    ///
    /// The count is returned rather than discarded because it is the useful
    /// diagnostic: a `record_finding` citing three ids that marks zero rows means
    /// the model cited entries the brief never showed it, which is a different
    /// problem from a low recall share and looks identical in the score.
    ///
    /// # Errors
    /// [`RetrievalError::Sqlx`] if the update fails.
    pub async fn mark_cited(
        &self,
        run_id: &str,
        entry_ids: &[String],
    ) -> Result<u64, RetrievalError> {
        if entry_ids.is_empty() {
            return Ok(0);
        }
        // Only the first citation sets the moment. A second `record_finding`
        // naming the same entry is not a second retrieval hit, and letting it
        // move `cited_at` would make the timestamp mean "last mentioned" while
        // reading as "first used".
        let done = sqlx::query(
            "UPDATE memory_retrievals SET cited = true, cited_at = now() \
              WHERE run_id = $1 AND entry_id = ANY($2) AND NOT cited",
        )
        .bind(run_id)
        .bind(entry_ids)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected())
    }

    /// The cited share for one run, or `None` if it retrieved nothing.
    ///
    /// # Errors
    /// [`RetrievalError::Sqlx`] if the query fails.
    pub async fn recall(&self, run_id: &str) -> Result<Option<f64>, RetrievalError> {
        let row = sqlx::query(
            "SELECT count(*)::bigint AS shown, \
                    count(*) FILTER (WHERE cited)::bigint AS cited \
               FROM memory_retrievals WHERE run_id = $1",
        )
        .bind(run_id)
        .fetch_one(&self.pool)
        .await?;
        let shown: i64 = row.try_get("shown").unwrap_or(0);
        let cited: i64 = row.try_get("cited").unwrap_or(0);
        if shown == 0 {
            // Nothing to recall is not a recall failure. See the module docs.
            return Ok(None);
        }
        #[allow(clippy::cast_precision_loss)]
        Ok(Some(cited as f64 / shown as f64))
    }

    /// Entries shown repeatedly and never cited.
    ///
    /// The question the score cannot answer: a low share says the brief is
    /// choosing badly, and this says *what* it keeps choosing. An entry shown
    /// forty times and never cited is either a retrieval-policy bug or a
    /// worthless entry, and only looking distinguishes them.
    ///
    /// # Errors
    /// [`RetrievalError::Sqlx`] if the query fails.
    pub async fn never_cited(&self, min_shown: i64) -> Result<Vec<(String, i64)>, RetrievalError> {
        let rows = sqlx::query(
            "SELECT entry_id, count(*)::bigint AS shown FROM memory_retrievals \
              GROUP BY entry_id \
             HAVING count(*) >= $1 AND count(*) FILTER (WHERE cited) = 0 \
              ORDER BY shown DESC LIMIT 50",
        )
        .bind(min_shown)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| (r.get::<String, _>("entry_id"), r.get::<i64, _>("shown")))
            .collect())
    }
}

/// Record which retrieved entries one core-tool call's evidence names (S-6.10).
///
/// Called by the run recorder with the arguments of every `record_finding` and
/// `finish_task` note: the ids the brief showed that the evidence names (whole
/// tokens only, see `harness::memory::cited_ids`) become
/// `memory_retrievals.cited`. A write failure is a warning: citation is a
/// measurement, and losing one must not end a run.
pub async fn record_citations(
    pg: &PgPool,
    run_id: uuid::Uuid,
    retrieved: &[Retrieval],
    tool: &str,
    args: &serde_json::Value,
) {
    if retrieved.is_empty()
        || !matches!(
            tool,
            harness::registry::RECORD_FINDING | harness::registry::FINISH_TASK
        )
    {
        return;
    }
    let Some(evidence) = args.get("evidence").and_then(serde_json::Value::as_str) else {
        return;
    };
    let ids = harness::memory::cited_ids(retrieved, evidence);
    if ids.is_empty() {
        return;
    }
    let store = RetrievalStore::new(pg.clone());
    if let Err(e) = store.mark_cited(&run_id.to_string(), &ids).await {
        tracing::warn!(error = %e, run_id = %run_id, "citation not recorded");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `Slot` has a string, and it is the enum's own serde spelling.
    ///
    /// The CHECK in `0124_memory_retrievals.sql` lists these, so a variant added
    /// to one and not the other writes a row the database refuses — at runtime,
    /// on a real brief, which is the worst place to find out.
    #[test]
    fn every_slot_maps_to_its_serde_name() {
        let all = [
            (Slot::UserPreference, "user_preference"),
            (Slot::InstrumentProfile, "instrument_profile"),
            (Slot::Finding, "finding"),
            (Slot::NegativeResult, "negative_result"),
            (Slot::OpenQuestion, "open_question"),
            (Slot::Conflict, "conflict"),
        ];
        for (slot, expected) in all {
            assert_eq!(slot_str(slot), expected);
            // And it is what serde would write, so the column and any JSON of
            // the same value agree.
            let json = serde_json::to_string(&slot).expect("Slot serialises");
            assert_eq!(json, format!("\"{expected}\""), "{slot:?}");
        }
    }
}
