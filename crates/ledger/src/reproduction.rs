//! The reproduction ticket (AGENT-013 §6.9, §8.4; ADR-P2-17; Set S task S-10.8,
//! ADR-S10-18).
//!
//! # The problem this closes
//!
//! `lifecycle.reproduce` re-runs a stored result under its own receipt and asks
//! whether the platform still computes the same number. A re-run needs a Run
//! executor, an executor takes a [`crate::TrialTicket`] (INV-16: unreachable
//! without one), and until now the only way to get a ticket was to register a
//! trial. Registering one for a reproduction would charge every check of the
//! platform to the trial budget, change what `trials_counted`, the exploration
//! floor and `N_eff` mean, and put a duplicate of a registered look into the hash
//! chain. So nobody reproduced anything, and the receipt (`backtest::receipt`,
//! built 2026-09-17) had no caller.
//!
//! # The decision (ADR-S10-18)
//!
//! A **reproduction ticket** is a `TrialTicket` minted by the ledger for a trial
//! that already completed. It names that trial ([`crate::TrialTicket::reproduces`])
//! and has no trial row of its own, so no counter moves. It is a ticket and not a
//! bypass for the reason INV-16 gives: the compiler still refuses compute that was
//! not authorised by the ledger — the authorisation here is "this look exists and
//! completed".
//!
//! What it cannot do is as load-bearing as what it can: `transition`, `settle` and
//! `persist_returns` refuse it ([`crate::LedgerError::ReproductionIsNotATrial`]),
//! because each is a trial operation and a reproduction that could write one would
//! be a look in disguise. Its one terminal act is [`ReproductionRecord`], an
//! append-only row (`mlops.reproduction`, migration 0241).
//!
//! # Not a zero-weight trial
//!
//! The alternative considered and rejected is a real `mlops.trial` row flagged
//! weight 0. It needs every reader of the table — `trial_count`, `n_eff`, the
//! exploration fraction, the hash chain, `campaign_state` — to learn the flag, and
//! the one that forgot would count checks as looks. A separate table that no
//! existing reader names cannot be miscounted by one.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The outcomes a reproduction can record, mirroring
/// `backtest::receipt::Reproduction::as_str` and 0241's CHECK.
pub const REPRODUCTION_OUTCOMES: &[&str] = &["bit_identical", "drifted", "receipt_changed"];

/// What a reproduction found, ready to append.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReproductionRecord {
    pub run_id: String,
    /// One of [`REPRODUCTION_OUTCOMES`].
    pub outcome: String,
    /// The receipt the original carried. `None` means it predates receipts, which
    /// the record states through `receipt_missing` rather than leaving a reader to
    /// infer it from a NULL.
    pub receipt_stored: Option<Value>,
    pub receipt_rerun: Value,
    /// Named differences when the receipt moved; an empty array otherwise.
    pub differences: Value,
    /// Only drift carries both numbers.
    pub was_value: Option<f64>,
    pub now_value: Option<f64>,
    pub reproduced_by: String,
}

impl ReproductionRecord {
    /// Whether the original carried no receipt.
    #[must_use]
    pub fn receipt_missing(&self) -> bool {
        self.receipt_stored.is_none()
    }

    /// The rules the schema also enforces, so a caller learns the reason before a
    /// constraint violation it cannot parse.
    ///
    /// # Errors
    /// The first rule broken.
    pub fn validate(&self) -> Result<(), String> {
        if !REPRODUCTION_OUTCOMES.contains(&self.outcome.as_str()) {
            return Err(format!(
                "outcome `{}` is not one of {REPRODUCTION_OUTCOMES:?}",
                self.outcome
            ));
        }
        if self.run_id.trim().is_empty() || self.reproduced_by.trim().is_empty() {
            return Err("a reproduction names its run and who ran it".into());
        }
        if self.outcome == "drifted" && (self.was_value.is_none() || self.now_value.is_none()) {
            return Err("a drift names both numbers".into());
        }
        if self.outcome == "receipt_changed"
            && self.differences.as_array().is_none_or(Vec::is_empty)
        {
            return Err("a changed receipt names what changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(outcome: &str) -> ReproductionRecord {
        ReproductionRecord {
            run_id: "r1".into(),
            outcome: outcome.into(),
            receipt_stored: Some(json!({})),
            receipt_rerun: json!({}),
            differences: json!([]),
            was_value: None,
            now_value: None,
            reproduced_by: "tester".into(),
        }
    }

    #[test]
    fn a_bit_identical_record_validates_and_an_unknown_outcome_does_not() {
        assert!(rec("bit_identical").validate().is_ok());
        assert!(rec("close_enough").validate().is_err());
    }

    #[test]
    fn a_drift_must_name_both_numbers_and_a_changed_receipt_what_changed() {
        assert!(rec("drifted").validate().is_err());
        let mut d = rec("drifted");
        d.was_value = Some(1.0);
        d.now_value = Some(1.1);
        assert!(d.validate().is_ok());
        assert!(rec("receipt_changed").validate().is_err());
        let mut c = rec("receipt_changed");
        c.differences = json!([{"field": "calendar_version", "was": "1", "now": "2"}]);
        assert!(c.validate().is_ok());
    }

    #[test]
    fn an_original_without_a_receipt_says_so() {
        let mut r = rec("bit_identical");
        r.receipt_stored = None;
        assert!(r.receipt_missing());
    }
}
