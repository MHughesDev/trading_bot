//! A campaign candidate cannot pass Gate 3 without its random-search arm (Set R-b RB-3.3,
//! EV-09; BS-007 11 section 8).
//!
//! A campaign's results mean something only against what random search with the same budget
//! finds, so the arm is part of the campaign record and a candidate that reaches the gate
//! stack without a completed arm is refused at Gate 3 and every gate after it. Gates 1 and 2
//! (pre-registration, leakage) are about the hypothesis and the data and are unaffected.
//!
//! # Why this is data and not a call
//!
//! The arm lives in Postgres (`agent_memory.campaign_arm`) and its rule is
//! `research::random_arm::gate_check`. `research` depends on `backtest`, so `backtest` cannot
//! call it, and it must not depend on `api` either. The seam is the same one the
//! reconciliation gate uses: the layer that owns the database (`api::gate_worker`) asks
//! `knowledge_routes::arm_gate` and hands this module the **answer** as an
//! [`ArmRequirement`]; the one rule stays in one place and this module only applies it.
//! A request that cannot be answered (the database is down) is refused too: the caller
//! builds the requirement with [`ArmRequirement::unreadable`], never with "no requirement".

use super::evaluators::GateOutcome;
use super::stack::StackVerdict;

/// The answer to "has this campaign's random arm completed?", as the gate stack applies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmRequirement {
    pub campaign_id: String,
    /// `None` when the arm exists and has completed; otherwise the refusal, which names the
    /// arm ("no random arm" / "run N of M").
    pub refusal: Option<String>,
}

impl ArmRequirement {
    /// The arm completed.
    #[must_use]
    pub fn met(campaign_id: impl Into<String>) -> Self {
        Self {
            campaign_id: campaign_id.into(),
            refusal: None,
        }
    }

    /// The arm is missing or incomplete, for `reason` (the text of `research`'s `ArmError`).
    #[must_use]
    pub fn refused(campaign_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            campaign_id: campaign_id.into(),
            refusal: Some(reason.into()),
        }
    }

    /// The arm could not be read. A gate that could not look does not pass.
    #[must_use]
    pub fn unreadable(campaign_id: impl Into<String>, error: impl std::fmt::Display) -> Self {
        Self::refused(
            campaign_id,
            format!("the random arm could not be read: {error}"),
        )
    }

    /// # Errors
    /// The reason Gate 3 and later are refused.
    pub fn satisfied(&self) -> Result<(), String> {
        match &self.refusal {
            None => Ok(()),
            Some(why) => Err(format!("campaign {}: {why}", self.campaign_id)),
        }
    }
}

/// Refuse every gate from 3 on when the arm is not complete. Gates 1 and 2 are untouched.
#[must_use]
pub fn apply_to_stack(mut verdict: StackVerdict, req: Option<&ArmRequirement>) -> StackVerdict {
    let Some(req) = req else { return verdict };
    let Err(reason) = req.satisfied() else {
        return verdict;
    };
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
    verdict.blocked_at = verdict
        .outcomes
        .iter()
        .find(|o| !o.passed)
        .map(|o| o.gate_no);
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_stack() -> StackVerdict {
        let outcomes: Vec<GateOutcome> = (1..=16)
            .map(|n| GateOutcome::structural_pass(n, "g", format!("gate {n} ok")))
            .collect();
        StackVerdict {
            blocked_at: None,
            outcomes,
        }
    }

    #[test]
    fn a_completed_arm_changes_nothing_and_no_requirement_is_not_a_requirement() {
        let v = apply_to_stack(passing_stack(), Some(&ArmRequirement::met("c")));
        assert_eq!(v.blocked_at, None);
        assert!(v.outcomes.iter().all(|o| o.passed));
        assert_eq!(
            apply_to_stack(passing_stack(), None).blocked_at,
            None,
            "a non-campaign candidate has no arm"
        );
    }

    #[test]
    fn a_missing_or_incomplete_arm_refuses_gate_3_and_every_later_gate_naming_the_arm() {
        let req = ArmRequirement::refused("camp-1", "the random arm has run 4 of 10 trials");
        let v = apply_to_stack(passing_stack(), Some(&req));
        assert_eq!(
            v.blocked_at,
            Some(3),
            "gates 1 and 2 stand; the block is at 3"
        );
        for o in &v.outcomes {
            if o.gate_no < 3 {
                assert!(
                    o.passed,
                    "gate {} is about the hypothesis and the data",
                    o.gate_no
                );
            } else {
                assert!(!o.passed, "gate {} passed without the arm", o.gate_no);
                assert!(!o.inconclusive);
                assert!(
                    o.detail.contains("camp-1") && o.detail.contains("4 of 10"),
                    "{}",
                    o.detail
                );
                assert!(
                    o.detail.contains("the gate itself read"),
                    "what the gate said is kept: {}",
                    o.detail
                );
            }
        }
    }

    #[test]
    fn an_unreadable_arm_is_a_refusal_not_a_pass() {
        let req = ArmRequirement::unreadable("c", "connection refused");
        assert!(req.satisfied().unwrap_err().contains("could not be read"));
        assert_eq!(
            apply_to_stack(passing_stack(), Some(&req)).blocked_at,
            Some(3)
        );
    }
}
