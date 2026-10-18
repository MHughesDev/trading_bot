//! What a replenish `survey` proposed, stored, and absorbed into the agenda
//! (AGENT-014 §4.9, §7; Set S task S-11.11; migration 0162; ADR-P11-06).
//!
//! # What was missing
//!
//! `replenish::absorb` is the decision: a survey's `proposed_questions[]` become
//! agenda items carrying the survey's own value estimates, deduped through layer
//! one, and the survey's yield is an **explicit** count of the questions that were
//! genuinely new — so a fully-deduped survey counts as barren. It had no caller,
//! because nothing stored a survey's proposals. They reached the agenda by the
//! ordinary harvest, as `open_question` entries scored like any other source 4.3
//! item, and two things were lost: the survey's estimates, and the explicit zero.
//!
//! # Where a survey's proposals come from
//!
//! A `survey` cannot issue a trial ticket (AGENT-013 §3), and the one door it has
//! for "what to ask next" is `lifecycle.open_question` — a write into semantic
//! memory carrying the question, why it matters, `blocking`, and `value`
//! (`curious | useful | decisive`). So the proposals are those entries, read back
//! by the run that wrote them ([`read_proposals`]), stored as the run's own rows
//! ([`store_proposals`]) and handed to [`replenish::absorb`].
//!
//! # The estimate is declared, not measured
//!
//! `value` is three words and `ProposedQuestion::value_estimate` is a number in
//! [0, 1]. The reading ([`estimate_for`]) is ordinal and written down once:
//! `curious` 0.25, `useful` 0.5, `decisive` 1.0. It is the survey's *opinion* and
//! is stored with its label so nothing downstream mistakes it for a measurement;
//! a proposal with no label has **no** estimate (NULL), never 0 — an absent
//! estimate is not a worthless one.

use research_session::replenish::{ProposedQuestion, Replenished, SurveyReport};
use research_session::Insert;
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

/// The declared ordinal reading of `lifecycle.open_question`'s `value`.
///
/// `None` for a word that is not one of the three — including an absent one.
#[must_use]
pub fn estimate_for(label: Option<&str>) -> Option<f64> {
    match label?.trim() {
        "curious" => Some(0.25),
        "useful" => Some(0.5),
        "decisive" => Some(1.0),
        _ => None,
    }
}

/// One question a survey run wrote down, before it is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposalRow {
    pub entry_id: String,
    pub question: String,
    pub subject: String,
    pub workflow: Option<String>,
    pub label: Option<String>,
}

impl ProposalRow {
    #[must_use]
    pub fn estimate(&self) -> Option<f64> {
        estimate_for(self.label.as_deref())
    }

    fn proposed(&self) -> ProposedQuestion {
        ProposedQuestion {
            question: self.question.clone(),
            subject: self.subject.clone(),
            workflow: self.workflow.clone(),
            value_estimate: self.estimate(),
        }
    }
}

/// The survey run's proposals: the `open_question` entries it filed.
///
/// # Errors
/// Database errors.
pub async fn read_proposals(
    pg: &PgPool,
    user_id: Uuid,
    run_id: Uuid,
) -> Result<Vec<ProposalRow>, sqlx::Error> {
    let rows: Vec<(String, String, String, Value)> = sqlx::query_as(
        "SELECT entry_id, claim, subject, extra FROM agent_memory.entry
          WHERE user_id = $1 AND kind = 'open_question' AND prov_run_id = $2 AND NOT archived
          ORDER BY created_at, entry_id",
    )
    .bind(user_id)
    .bind(run_id.to_string())
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(entry_id, question, subject, extra)| ProposalRow {
            entry_id,
            question,
            subject,
            workflow: extra
                .get("workflow")
                .and_then(Value::as_str)
                .map(str::to_string),
            label: extra
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
        .collect())
}

/// The report [`replenish::absorb`] takes.
#[must_use]
pub fn report(session: Uuid, item: Uuid, run: Uuid, rows: &[ProposalRow]) -> SurveyReport {
    SurveyReport {
        session,
        item,
        run,
        proposed_questions: rows.iter().map(ProposalRow::proposed).collect(),
    }
}

/// Store the run's proposals. Idempotent on `(run, question)`, so a driver that
/// restarts between storing and absorbing writes nothing twice.
///
/// # Errors
/// Database errors.
pub async fn store_proposals(
    pg: &PgPool,
    session: Uuid,
    item: Uuid,
    run: Uuid,
    rows: &[ProposalRow],
) -> Result<(), sqlx::Error> {
    for r in rows {
        if r.question.trim().is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO session_survey_proposals
                 (session_id, item_id, run_id, source_entry, question, subject, workflow,
                  value_label, value_estimate)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
             ON CONFLICT (run_id, question) DO NOTHING",
        )
        .bind(session)
        .bind(item)
        .bind(run)
        .bind(&r.entry_id)
        // `question` is NOT NULL and non-blank; trimmed here so the unique key
        // and the dedupe agree about what the same question is.
        .bind(r.question.trim())
        .bind(&r.subject)
        .bind(r.workflow.as_deref())
        // The label is stored only when it is one this module can read, so the
        // `an_estimate_has_its_label` CHECK and `estimate_for` cannot disagree.
        .bind(
            r.label
                .as_deref()
                .filter(|l| estimate_for(Some(l)).is_some()),
        )
        .bind(r.estimate())
        .execute(pg)
        .await?;
    }
    Ok(())
}

/// What absorbing a survey did, as the counts the absorption row holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub proposed: i32,
    pub new_questions: i32,
    pub merged: i32,
    pub skipped: i32,
}

impl Counts {
    /// From what [`replenish::absorb`] returned and how many were proposed.
    #[must_use]
    pub fn of(proposed: usize, r: &Replenished) -> Self {
        let merged = r
            .inserted
            .iter()
            .filter(|i| matches!(i, Insert::Merged { .. }))
            .count();
        let to_i32 = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
        Self {
            proposed: to_i32(proposed),
            new_questions: to_i32(r.new_questions()),
            merged: to_i32(merged),
            skipped: to_i32(r.skipped.len()),
        }
    }
}

/// Write the absorption row, once. A second call for the same run is a no-op:
/// the row is what the barren streak reads and it is not rewritten.
///
/// # Errors
/// Database errors.
pub async fn record_absorption(
    pg: &PgPool,
    session: Uuid,
    item: Uuid,
    run: Uuid,
    c: Counts,
) -> Result<bool, sqlx::Error> {
    let n = sqlx::query(
        "INSERT INTO session_survey_absorptions
             (run_id, session_id, item_id, proposed, new_questions, merged, skipped)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (run_id) DO NOTHING",
    )
    .bind(run)
    .bind(session)
    .bind(item)
    .bind(c.proposed)
    .bind(c.new_questions)
    .bind(c.merged)
    .bind(c.skipped)
    .execute(pg)
    .await?
    .rows_affected();
    Ok(n == 1)
}

/// Mark what became of each proposal.
///
/// # Errors
/// Database errors.
pub async fn mark_outcomes(
    pg: &PgPool,
    run: Uuid,
    added: &[String],
    merged: &[String],
    skipped: &[String],
) -> Result<(), sqlx::Error> {
    for (outcome, questions) in [("added", added), ("merged", merged), ("skipped", skipped)] {
        for q in questions {
            sqlx::query(
                "UPDATE session_survey_proposals SET outcome = $3
                  WHERE run_id = $1 AND question = $2 AND outcome = 'pending'",
            )
            .bind(run)
            .bind(q.trim())
            .bind(outcome)
            .execute(pg)
            .await?;
        }
    }
    Ok(())
}

/// How many questions a survey run added that were genuinely new, or `None` when
/// the run was never absorbed — which is not zero.
///
/// # Errors
/// Database errors.
pub async fn new_questions(pg: &PgPool, run: Uuid) -> Result<Option<i32>, sqlx::Error> {
    let row: Option<(i32,)> =
        sqlx::query_as("SELECT new_questions FROM session_survey_absorptions WHERE run_id = $1")
            .bind(run)
            .fetch_optional(pg)
            .await?;
    Ok(row.map(|r| r.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_estimate_is_the_declared_ordinal_reading_and_an_absent_label_is_none_not_zero() {
        assert_eq!(estimate_for(Some("curious")), Some(0.25));
        assert_eq!(estimate_for(Some("useful")), Some(0.5));
        assert_eq!(estimate_for(Some("decisive")), Some(1.0));
        assert_eq!(estimate_for(None), None);
        assert_eq!(estimate_for(Some("important")), None);
        assert_eq!(estimate_for(Some("")), None);
    }

    #[test]
    fn a_report_carries_each_proposal_with_its_estimate_and_never_an_invented_one() {
        let rows = vec![
            ProposalRow {
                entry_id: "q1".into(),
                question: "does funding lead returns on SOL?".into(),
                subject: "SOL-USD".into(),
                workflow: None,
                label: Some("decisive".into()),
            },
            ProposalRow {
                entry_id: "q2".into(),
                question: "is the spread stable?".into(),
                subject: "SOL-USD".into(),
                workflow: Some("diagnose".into()),
                label: None,
            },
        ];
        let r = report(Uuid::nil(), Uuid::nil(), Uuid::nil(), &rows);
        assert_eq!(r.proposed_questions.len(), 2);
        assert_eq!(r.proposed_questions[0].value_estimate, Some(1.0));
        assert_eq!(r.proposed_questions[1].value_estimate, None);
        assert_eq!(
            r.proposed_questions[1].workflow.as_deref(),
            Some("diagnose")
        );
    }
}
