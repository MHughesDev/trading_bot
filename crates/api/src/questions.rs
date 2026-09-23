//! Closing an open question is the platform's move (AGENT-013 §9; Set S task
//! S-10.12).
//!
//! # What this is the caller for
//!
//! `harness::memory::questions::close_cited` decides which questions a finding
//! closes and has had no caller outside its own tests. So a question opened in
//! one run stayed open forever — retrieved into every later brief for its
//! subject, skipped, and retrieved again — even when a later finding answered
//! it. `Yield::questions_closed` has been zero on every run ever recorded for
//! the same reason.
//!
//! # Why the model does not close its own
//!
//! §9's rule, and the same one as the verdict: a run that has been thinking
//! about something for nine steps genuinely feels it has resolved it. What the
//! platform can check is a **fact about what was written down** — a finding that
//! cites the question's id — so that is the only thing that closes one.
//!
//! # Citing a question the store does not hold closes nothing, and is not an
//! error
//!
//! The citation may name a question from another tenant, one already closed, or
//! a typo. Refusing the finding over it would make citing a question risky, and
//! a run that has learned citing is risky stops citing — which costs the
//! platform the one signal it has for "this got answered".
//!
//! # The first close stands
//!
//! `extra->>'status' = 'open'` is in the WHERE clause. A second close would
//! overwrite *who* answered it, and "who answered this" is the part worth
//! keeping.

use harness::memory::{questions::How, Entry, Kind};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Close every open question this finding cites.
///
/// Returns the ids it closed, so the caller can count them without re-reading.
/// An entry that is not a finding closes nothing: a question citing a question
/// is a cross-reference, and a negative result citing one is a run reporting
/// that it could *not* answer it.
///
/// # Errors
/// The database failed. Nothing closed is `Ok(vec![])`, which is the common
/// case and not an error.
pub async fn close_cited(
    pg: &PgPool,
    user_id: Uuid,
    finding: &Entry,
) -> Result<Vec<String>, sqlx::Error> {
    if finding.kind != Kind::Finding {
        return Ok(Vec::new());
    }
    let cited: Vec<String> = finding.evidence_ids.values().cloned().collect();
    if cited.is_empty() {
        return Ok(Vec::new());
    }

    let closed = json!({ "closed": { "by": finding.id, "how": How::Answered.as_str() } });
    let rows: Vec<(String,)> = sqlx::query_as(
        "UPDATE agent_memory.entry
            SET extra = jsonb_set(extra, '{status}', $1::jsonb, true),
                updated_at = now()
          WHERE user_id = $2
            AND entry_id = ANY($3)
            AND kind = 'open_question'
            AND extra->>'status' = 'open'
      RETURNING entry_id",
    )
    .bind(&closed)
    .bind(user_id)
    .bind(&cited)
    .fetch_all(pg)
    .await?;

    Ok(rows.into_iter().map(|(id,)| id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::memory::{DataWindow, Provenance};

    fn finding(id: &str, cites: &[(&str, &str)]) -> Entry {
        let mut e = Entry::finding(
            id,
            "BTC-USD",
            "the funding leg survives costs on the 4h bar",
            DataWindow::new("2020-01-01", "2024-12-31"),
            Provenance::new("run_1", "record_finding", "1", "0.1.0"),
        );
        for (role, value) in cites {
            e.evidence_ids
                .insert((*role).to_string(), (*value).to_string());
        }
        e
    }

    /// A finding with no citations closes nothing, and asks the database
    /// nothing. The early return matters: `entry_id = ANY('{}')` matches no row
    /// but still costs a round trip on every finding a run writes, and most
    /// findings cite no question.
    #[test]
    fn a_finding_with_no_citations_has_nothing_to_close() {
        let e = finding("f_1", &[]);
        assert!(e.evidence_ids.is_empty());
    }

    /// Only a finding closes one.
    ///
    /// A negative result citing a question is a run saying it could **not**
    /// answer it, which is the opposite of closing it — and it is the citation
    /// shape most likely to appear, because a run that fails often cites what it
    /// was trying to settle.
    #[test]
    fn only_a_finding_closes_a_question() {
        let mut e = finding("n_1", &[("question_id", "q_1")]);
        e.kind = Kind::NegativeResult;
        assert_ne!(e.kind, Kind::Finding);

        let mut q = finding("q_2", &[("question_id", "q_1")]);
        q.kind = Kind::OpenQuestion;
        assert_ne!(q.kind, Kind::Finding);
    }

    /// The closure names its closer and how, in the shape
    /// `OpenQuestion::extra()` writes — so a question closed here and one
    /// deserialised from the store are the same object.
    #[test]
    fn the_closure_names_who_closed_it() {
        let closed = json!({ "closed": { "by": "f_1", "how": How::Answered.as_str() } });
        assert_eq!(closed["closed"]["by"], "f_1");
        assert_eq!(closed["closed"]["how"], "answered");

        // And it round-trips into the harness type, which is what the desk and
        // the brief read.
        let mut q = harness::memory::questions::open(
            "q_1",
            "BTC-USD",
            "does the funding leg survive costs",
            "the carry thesis rests on it",
            true,
            harness::memory::questions::QuestionValue::Decisive,
            "run_1",
        )
        .expect("a complete question");
        assert!(q.close_as_answered("f_1"));
        assert_eq!(q.extra()["status"], closed);
    }

    /// Every role a finding cites is a candidate, not just one named key.
    ///
    /// `evidence_ids` is keyed by role — `experiment_id`, `question_id`,
    /// whatever a producer used — and a closer that looked only under
    /// `question_id` would miss a citation written under any other name and
    /// silently close nothing.
    #[test]
    fn any_cited_id_is_a_candidate_whatever_its_role() {
        let e = finding(
            "f_1",
            &[
                ("experiment_id", "exp_7"),
                ("question_id", "q_1"),
                ("answers", "q_2"),
            ],
        );
        let cited: Vec<String> = e.evidence_ids.values().cloned().collect();
        assert!(cited.contains(&"q_1".to_string()));
        assert!(cited.contains(&"q_2".to_string()));
        // `exp_7` is in the candidate set too and closes nothing, because the
        // UPDATE's own WHERE clause requires the row to be an open question.
        assert!(cited.contains(&"exp_7".to_string()));
    }
}
