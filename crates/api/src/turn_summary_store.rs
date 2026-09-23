//! Storing and rendering turn summaries (AGENT-007 §2.1; Set S task S-4.1).
//!
//! # The carry this replaces
//!
//! `conversations::prior_context` told the next turn "you concluded: …" using
//! `agent_runs.summary` — the model's own closing prose. Nothing checked that
//! against what the run did, and the hosted driver ignored it entirely. A turn
//! could therefore claim a conclusion it had not reached and be believed by the
//! next one, which is the one place in the loop where a hallucination compounds
//! instead of being caught by a tool.
//!
//! [`harness::memory::TurnSummary::from_state`] takes `WorkingState` and the two
//! ledgers and nothing else. This module is the half that makes that survive a
//! process boundary.
//!
//! # Rendering truncates by confidence and never drops an id
//!
//! The budget is §2.1's 600 tokens. What goes first is `suggestive` findings,
//! then rejected ideas, then open questions — in that order, because an
//! established finding is the thing a later turn must not re-derive, and an id
//! is what makes it re-derivable at all. A rendering that dropped ids to fit
//! would leave the model a claim it cannot check, which is worse than leaving
//! the claim out.

use harness::memory::{Confidence, TurnSummary};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Roughly §2.1's 600-token budget, in characters.
///
/// Characters rather than tokens because this crate has no tokeniser and a
/// wrong tokeniser is worse than an honest ratio: four characters per token is
/// the standard approximation, and the budget is a ceiling rather than a target.
const RENDER_BUDGET_CHARS: usize = 600 * 4;

#[derive(Debug, thiserror::Error)]
pub enum TurnSummaryError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("turn summary for {conversation}/{turn} is not a TurnSummary: {source}")]
    Malformed {
        conversation: Uuid,
        turn: i32,
        source: serde_json::Error,
    },
}

pub struct TurnSummaryStore {
    pool: PgPool,
}

impl TurnSummaryStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Write the summary for one turn, replacing any earlier attempt at it.
    ///
    /// # Errors
    /// [`TurnSummaryError::Sqlx`] if the write fails.
    pub async fn put(
        &self,
        conversation_id: Uuid,
        summary: &TurnSummary,
    ) -> Result<(), TurnSummaryError> {
        let doc = serde_json::to_value(summary).unwrap_or(serde_json::Value::Null);
        sqlx::query(
            "INSERT INTO agent_turn_summaries \
               (conversation_id, turn, outcome, trials_spent, summary) \
             VALUES ($1,$2,$3,$4,$5) \
             ON CONFLICT (conversation_id, turn) DO UPDATE SET \
               outcome = EXCLUDED.outcome, \
               trials_spent = EXCLUDED.trials_spent, \
               summary = EXCLUDED.summary, \
               updated_at = now()",
        )
        .bind(conversation_id)
        .bind(summary.turn)
        .bind(&summary.outcome)
        .bind(i32::try_from(summary.trials_spent).unwrap_or(i32::MAX))
        .bind(doc)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Every summary before `turn`, oldest first.
    ///
    /// # Errors
    /// [`TurnSummaryError::Sqlx`] if the query fails;
    /// [`TurnSummaryError::Malformed`] for a row that is not a `TurnSummary` —
    /// named rather than skipped, because a turn silently missing from the carry
    /// is a turn the model is told did not happen.
    pub async fn before(
        &self,
        conversation_id: Uuid,
        turn: i32,
    ) -> Result<Vec<TurnSummary>, TurnSummaryError> {
        let rows = sqlx::query(
            "SELECT turn, summary FROM agent_turn_summaries \
              WHERE conversation_id = $1 AND turn < $2 ORDER BY turn ASC",
        )
        .bind(conversation_id)
        .bind(turn)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|r| {
                let doc: serde_json::Value = r.get("summary");
                serde_json::from_value(doc).map_err(|source| TurnSummaryError::Malformed {
                    conversation: conversation_id,
                    turn: r.get("turn"),
                    source,
                })
            })
            .collect()
    }
}

/// The carry a later turn reads, rendered from the structured summaries.
///
/// Returns an empty string when there is nothing before this turn, so a first
/// turn carries no header at all rather than a heading with nothing under it.
#[must_use]
pub fn render(summaries: &[TurnSummary]) -> String {
    if summaries.is_empty() {
        return String::new();
    }
    let mut out = String::from("Earlier in this conversation:\n");
    for s in summaries {
        out.push_str(&format!("\n[turn {}] goal: {}\n", s.turn, s.goal));
        out.push_str(&format!("outcome: {}", s.outcome));
        if s.trials_spent > 0 {
            out.push_str(&format!(", {} trial(s) spent", s.trials_spent));
        }
        if let Some(n) = s.n_eff_after {
            out.push_str(&format!(", n_eff now {n:.2}"));
        }
        out.push('\n');

        // Established first, and with their ids. This is the ordering the budget
        // truncates against: an id is what makes a claim checkable, so a line
        // that would have to drop one is a line not worth keeping.
        let mut established: Vec<&_> = Vec::new();
        let mut suggestive: Vec<&_> = Vec::new();
        for f in &s.findings {
            if f.confidence == Confidence::Established {
                established.push(f);
            } else {
                suggestive.push(f);
            }
        }
        for f in established.iter().chain(suggestive.iter()) {
            // The supporting ids go on the same line as the claim. Not a second
            // block: a claim and the evidence for it are one thing, and
            // separating them is how a truncation keeps the claim and drops the
            // evidence — leaving the model a number it cannot check.
            let ids = if f.ids.is_empty() {
                String::new()
            } else {
                let mut parts: Vec<String> =
                    f.ids.iter().map(|(k, v)| format!("{k}={v}")).collect();
                parts.sort();
                format!(" [{}]", parts.join(", "))
            };
            let n = f.n.map_or_else(String::new, |n| format!(", n={n}"));
            out.push_str(&format!(
                "  [{}] {} — {}{n}{ids}\n",
                f.id,
                f.confidence.as_str(),
                f.claim
            ));
        }
        for n in &s.negative_results {
            out.push_str(&format!("  [{}] ruled out: {}\n", n.ticket_id, n.why));
        }
        for (gate, state) in &s.gate_states {
            out.push_str(&format!("  gate {gate}: {state}\n"));
        }
        for r in &s.rejected_ideas {
            out.push_str(&format!("  rejected \"{}\": {}\n", r.idea, r.reason));
        }
        for q in &s.open_questions {
            out.push_str(&format!("  open question: {q}\n"));
        }
        if let Some(step) = &s.current_step {
            out.push_str(&format!("  left off at: {step}\n"));
        }
    }

    truncate_without_dropping_ids(out)
}

/// Trim to the budget on a line boundary, never mid-line.
///
/// Cutting mid-line would truncate an id, and half an id is worse than no id: it
/// looks citeable and resolves to nothing. So whole lines go, newest-first
/// within the last turn, and the result says how many it dropped — a carry that
/// is silently short teaches the model the earlier turns did less than they did.
fn truncate_without_dropping_ids(text: String) -> String {
    if text.len() <= RENDER_BUDGET_CHARS {
        return text;
    }
    let mut lines: Vec<&str> = text.lines().collect();
    let mut dropped = 0;
    while lines.iter().map(|l| l.len() + 1).sum::<usize>() > RENDER_BUDGET_CHARS && lines.len() > 1
    {
        lines.pop();
        dropped += 1;
    }
    let mut out = lines.join("\n");
    out.push_str(&format!(
        "\n… {dropped} earlier line(s) omitted to fit the carry budget; nothing above was \
         abbreviated."
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness::memory::{SummaryFinding, SummaryNegative, TurnBudget};
    use harness::state::Stage;

    fn summary(turn: i32) -> TurnSummary {
        TurnSummary {
            turn,
            goal: "find an edge in BTC-USD 1h".into(),
            outcome: "finished".into(),
            findings: vec![
                SummaryFinding {
                    id: "f_2".into(),
                    claim: "a model summary of the window".into(),
                    ids: std::collections::BTreeMap::new(),
                    n: None,
                    se: None,
                    confidence: Confidence::Suggestive,
                },
                SummaryFinding {
                    id: "f_1".into(),
                    claim: "the residual t-stat is 3.1".into(),
                    ids: std::collections::BTreeMap::from([(
                        "experiment_id".to_string(),
                        "exp_42".to_string(),
                    )]),
                    n: Some(812),
                    se: Some(0.4),
                    confidence: Confidence::Established,
                },
            ],
            negative_results: vec![SummaryNegative {
                ticket_id: "t_9".into(),
                class: "weak_effect".into(),
                why: "the effect vanished out of sample".into(),
                retry_condition: None,
            }],
            trials_spent: 2,
            n_eff_after: Some(7.5),
            gate_states: std::collections::BTreeMap::new(),
            open_questions: vec!["does it survive a costed fill?".into()],
            rejected_ideas: vec![],
            budget: TurnBudget::default(),
            yield_ref: None,
            stage: Stage::default(),
            current_step: Some("design a costed backtest".into()),
        }
    }

    /// Established findings come first, because that is what a later turn must
    /// not spend a trial re-deriving.
    #[test]
    fn established_findings_are_rendered_before_suggestive_ones() {
        let text = render(&[summary(1)]);
        let est = text.find("f_1").expect("the established finding renders");
        let sug = text.find("f_2").expect("the suggestive finding renders");
        assert!(
            est < sug,
            "the suggestive finding is rendered first, so a truncation would drop the \
             established one:\n{text}"
        );
    }

    /// Every finding carries its id. An id is what makes a claim checkable, and a
    /// carry without one hands the model a claim it can only take on trust.
    #[test]
    fn every_rendered_claim_carries_its_id() {
        let text = render(&[summary(1)]);
        for id in ["f_1", "f_2", "t_9"] {
            assert!(text.contains(id), "`{id}` is missing from:\n{text}");
        }
        assert!(text.contains("2 trial(s) spent"));
        assert!(text.contains("n_eff now 7.50"));
    }

    /// Nothing before turn one means no header, not an empty header.
    #[test]
    fn a_first_turn_carries_nothing() {
        assert!(render(&[]).is_empty());
    }

    /// Truncation removes whole lines and says so.
    ///
    /// Cutting mid-line would truncate an id, and half an id looks citeable and
    /// resolves to nothing — which is worse than the claim being absent.
    #[test]
    fn truncation_drops_whole_lines_and_reports_it() {
        let many: Vec<TurnSummary> = (1..80).map(summary).collect();
        let text = render(&many);
        assert!(
            text.len() <= RENDER_BUDGET_CHARS + 200,
            "the carry is {} chars against a {RENDER_BUDGET_CHARS}-char budget",
            text.len()
        );
        assert!(
            text.contains("omitted to fit the carry budget"),
            "a silently short carry teaches the model the earlier turns did less than they did"
        );
        // Whatever survived, no line is a fragment of another.
        for line in text.lines() {
            assert!(
                !line.ends_with("f_") && !line.ends_with('['),
                "a line was cut mid-id: {line:?}"
            );
        }
    }
}
