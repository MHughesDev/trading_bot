//! A Layer 2 version of a Layer 1 idea cannot pass Gate 3 unreconciled (Set M M-4.4; FEAT-004 §7,
//! ST-10).
//!
//! A research idea written as free-form Python earns a typed, deployable version only by
//! reconciling: both are run on one window under one policy and the positions agree
//! (`layer_reconciliation::reconcile`, recorded in `strategy_reconciliation`, migration 0205).
//! Without this gate that record is a table nothing reads. With it, a Layer 2 revision whose
//! recorded parent is a Layer 1 idea is refused at Gate 3 and every gate after it until a
//! **passed** reconciliation for exactly that pair stands.
//!
//! # Two places it bites
//!
//! * [`ReconciliationRequirement::satisfied`] is checked by the funnel runner
//!   (`GateRunner::with_reconciliation`): entering Significance or the Vault is refused.
//! * [`apply_to_stack`] post-processes the sixteen-gate stack: every gate from 3 on is
//!   refused, naming the reason. Gates 1 and 2 (pre-registration, leakage) are about the
//!   hypothesis and the data and are unaffected.
//!
//! # The standing verdict
//!
//! A pair may have several records (different thresholds are different rows). The **newest**
//! record decides: a pass followed by a later failure is a failure, and a failure followed by a
//! later pass is a pass. Taking "any pass" would let a candidate keep re-reconciling under
//! looser thresholds until one passed.

use chrono::{DateTime, Utc};

use super::evaluators::GateOutcome;
use super::stack::StackVerdict;

/// One row of `strategy_reconciliation`, as the gate reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconRecord {
    pub layer1_revision: String,
    pub layer2_revision: String,
    pub passed: bool,
    pub created_at: DateTime<Utc>,
}

/// What a Layer 2 revision owes before Gate 3.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReconciliationRequirement {
    pub layer2_revision: String,
    /// The Layer 1 revision the Layer 2 one was derived from (its recorded parent).
    pub layer1_parent: String,
    pub records: Vec<ReconRecord>,
}

impl ReconciliationRequirement {
    /// # Errors
    /// The reason Gate 3 and later are refused: no reconciliation, or the standing one failed.
    pub fn satisfied(&self) -> Result<(), String> {
        let newest = self
            .records
            .iter()
            .filter(|r| r.layer1_revision == self.layer1_parent && r.layer2_revision == self.layer2_revision)
            .max_by_key(|r| r.created_at);
        match newest {
            None => Err(format!(
                "layer 2 revision {} is derived from layer 1 idea {} and has no reconciliation: the \
                 two were never run side by side, so the typed version is not known to be the idea",
                self.layer2_revision, self.layer1_parent
            )),
            Some(r) if !r.passed => Err(format!(
                "the standing reconciliation of {} against {} failed",
                self.layer2_revision, self.layer1_parent
            )),
            Some(_) => Ok(()),
        }
    }
}

/// Refuse every gate from 3 on when the requirement is not met. Gates 1 and 2 are untouched.
#[must_use]
pub fn apply_to_stack(mut verdict: StackVerdict, req: Option<&ReconciliationRequirement>) -> StackVerdict {
    let Some(req) = req else { return verdict };
    let Err(reason) = req.satisfied() else { return verdict };
    for o in &mut verdict.outcomes {
        if o.gate_no >= 3 {
            let was = std::mem::take(&mut o.detail);
            *o = GateOutcome {
                gate_no: o.gate_no,
                gate_name: o.gate_name,
                passed: false,
                statistic: o.statistic,
                threshold: o.threshold,
                detail: format!("refused: {reason}. (the gate itself read: {was})"),
                inconclusive: false,
            };
        }
    }
    verdict.blocked_at = verdict.outcomes.iter().find(|o| !o.passed).map(|o| o.gate_no);
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn rec(l1: &str, l2: &str, passed: bool, minute: u32) -> ReconRecord {
        ReconRecord {
            layer1_revision: l1.into(),
            layer2_revision: l2.into(),
            passed,
            created_at: Utc.with_ymd_and_hms(2026, 10, 4, 12, minute, 0).unwrap(),
        }
    }

    fn req(records: Vec<ReconRecord>) -> ReconciliationRequirement {
        ReconciliationRequirement { layer2_revision: "L2".into(), layer1_parent: "L1".into(), records }
    }

    #[test]
    fn a_layer_2_version_with_no_reconciliation_is_refused_with_the_reason() {
        let e = req(vec![]).satisfied().unwrap_err();
        assert!(e.contains("no reconciliation") && e.contains("L1") && e.contains("L2"), "{e}");
        // A record for some other pair is not this pair's.
        assert!(req(vec![rec("L1", "OTHER", true, 0), rec("X", "L2", true, 0)]).satisfied().is_err());
    }

    #[test]
    fn a_passed_one_proceeds_and_a_failed_one_stays_refused() {
        assert!(req(vec![rec("L1", "L2", true, 0)]).satisfied().is_ok());
        let e = req(vec![rec("L1", "L2", false, 0)]).satisfied().unwrap_err();
        assert!(e.contains("failed"), "{e}");
    }

    #[test]
    fn the_newest_record_decides_so_a_candidate_cannot_shop_thresholds_until_one_passes() {
        assert!(req(vec![rec("L1", "L2", false, 0), rec("L1", "L2", true, 5)]).satisfied().is_ok());
        assert!(req(vec![rec("L1", "L2", true, 0), rec("L1", "L2", false, 5)]).satisfied().is_err());
    }

    fn passing_stack() -> StackVerdict {
        let outcomes: Vec<GateOutcome> =
            (1..=16).map(|n| GateOutcome::structural_pass(n, "g", format!("gate {n} ok"))).collect();
        StackVerdict { blocked_at: None, outcomes }
    }

    #[test]
    fn the_stack_refuses_gate_3_and_every_later_gate_but_not_1_and_2() {
        let v = apply_to_stack(passing_stack(), Some(&req(vec![])));
        assert_eq!(v.blocked_at, Some(3));
        assert!(v.outcomes.iter().filter(|o| o.gate_no <= 2).all(|o| o.passed));
        assert!(v.outcomes.iter().filter(|o| o.gate_no >= 3).all(|o| !o.passed && o.detail.starts_with("refused:")));
        assert!(!v.passed());
        assert_eq!(v.outcomes.len(), 16, "the stack is still sixteen long");
    }

    #[test]
    fn with_a_passed_reconciliation_or_no_requirement_the_stack_is_untouched() {
        let ok = apply_to_stack(passing_stack(), Some(&req(vec![rec("L1", "L2", true, 0)])));
        assert!(ok.passed());
        assert!(apply_to_stack(passing_stack(), None).passed(), "a revision with no Layer 1 parent owes nothing");
    }
}
