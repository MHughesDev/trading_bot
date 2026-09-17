//! What a battery concluded (migration `0135`; Set S task S-8.4).
//!
//! # Who writes one
//!
//! The workflow engine, at the end of a battery it ran itself, from the steps it
//! dispatched. Not a tool the model calls. The whole value of a gate that
//! demands a ticket is that the ticket cannot be produced by asserting it, so
//! there is deliberately no constructor here that takes a verdict and a subject
//! and nothing else — [`Record::validate`] refuses a verdict with no steps
//! behind it.
//!
//! # Why the steps are stored and not summarised
//!
//! AGENT-006 §6 says the leakage audit's evidence bundle is **permanent, not
//! transient**. A verdict cited three months later is worth what it can be taken
//! apart into: which checks fired, which were skipped and why, and which ids they
//! produced. A stored `clean` with nothing behind it is a claim, and this module
//! exists because claims were the problem.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::LedgerError;

/// A battery's conclusion, as the engine recorded it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// The family the battery minted before its first step was admitted
    /// (`0134`), which is how a verdict joins to the looks it rests on.
    pub family_id: String,
    pub tenant_id: String,
    pub workflow: String,
    /// One of the battery's own closed set.
    pub verdict: String,
    /// Why, in the battery's words.
    pub because: String,
    /// What it was run over — the experiment id.
    pub subject: String,
    pub run_id: Option<Uuid>,
    /// Every step, its status and its ids.
    pub steps: Value,
    /// Whether every step that was meant to run did.
    pub complete: bool,
}

impl Record {
    /// Structural checks that hold before any backend is involved.
    ///
    /// # Errors
    /// [`LedgerError::Invalid`] naming the field and why.
    pub fn validate(&self) -> Result<(), LedgerError> {
        let invalid = |m: &str| Err(LedgerError::Invalid(m.to_string()));
        if self.family_id.trim().is_empty() {
            return invalid(
                "a verdict carries the family its battery minted; without it the verdict cannot \
                 be joined to the looks it rests on",
            );
        }
        if self.workflow.trim().is_empty() {
            return invalid("a verdict names the battery that reached it");
        }
        if self.verdict.trim().is_empty() {
            return invalid("a verdict is a word from the battery's closed set");
        }
        if self.because.trim().is_empty() {
            return invalid(
                "a verdict gives its reason: a refusal that says only `fails` costs the reader \
                 the run that produced it",
            );
        }
        if self.subject.trim().is_empty() {
            return invalid("a verdict names what it was run over");
        }
        // The one check that is about the shape of evidence rather than the
        // shape of a row. A battery with no steps did not run, and a verdict
        // from a battery that did not run is exactly the assertion this table
        // exists to stop being possible.
        match self.steps.as_array() {
            Some(steps) if !steps.is_empty() => Ok(()),
            _ => invalid(
                "a verdict carries the steps that reached it. A verdict with no steps is a \
                 claim, and a gate that accepted one would be back to accepting arguments",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> Record {
        Record {
            family_id: "wf_selection_bias_verdict_7".into(),
            tenant_id: "t".into(),
            workflow: "selection_bias_verdict".into(),
            verdict: "passes".into(),
            because: "2 candidates survive the correction over 40 effective trials".into(),
            subject: "exp_1".into(),
            run_id: Some(Uuid::new_v4()),
            steps: json!([{ "label": "family-wise correction", "status": "ran" }]),
            complete: true,
        }
    }

    #[test]
    fn a_complete_record_validates() {
        record().validate().expect("complete");
    }

    /// The check this module exists for.
    #[test]
    fn a_verdict_with_no_steps_is_not_a_verdict() {
        for steps in [json!([]), json!(null), json!({}), json!("clean")] {
            let mut r = record();
            r.steps = steps.clone();
            let err = r.validate().expect_err("no steps behind it");
            assert!(format!("{err}").contains("claim"), "{steps}: {err}");
        }
    }

    /// Every other field is required too, and the message names it.
    #[test]
    fn every_field_a_reader_needs_is_required() {
        let mut r = record();
        r.family_id = "  ".into();
        assert!(format!("{}", r.validate().expect_err("no family")).contains("family"));

        let mut r = record();
        r.because = String::new();
        assert!(format!("{}", r.validate().expect_err("no reason")).contains("reason"));

        let mut r = record();
        r.subject = String::new();
        assert!(r.validate().is_err());

        let mut r = record();
        r.verdict = String::new();
        assert!(r.validate().is_err());
    }

    /// An incomplete battery still records a verdict — it just records that it
    /// was incomplete. Refusing to store it would lose the evidence that a
    /// backend was down when a gate was attempted.
    #[test]
    fn an_incomplete_battery_is_still_recorded() {
        let mut r = record();
        r.complete = false;
        r.verdict = "inconclusive".into();
        r.validate().expect("recorded, and marked incomplete");
    }
}
