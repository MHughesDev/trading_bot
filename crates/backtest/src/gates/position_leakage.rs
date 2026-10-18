//! Gate 0's check for Layer 1 research strategies (Set M M-1.7; FEAT-004 §2.3, ST-03).
//!
//! A `position_series` run is a free-form function's output, and the one way such a
//! function lies is by reading data it should not have had. The truncation test
//! (`position_series::truncation_test`, and its Python twin in
//! `apps/skill-runner/research_strategy.py`) asks whether the earlier answers survive
//! the later data not existing. This module is where its verdict becomes something the
//! gate stack can act on:
//!
//! * [`check`] turns a verdict into the stable-coded [`Flag`] Gate 0 attaches, beside
//!   `leakage.model_trained_past_decision`. Only a pass raises nothing; a failure names
//!   the first differing timestamp; a verdict that **was never produced** raises
//!   `leakage.position_truncation_not_run`, because an unchecked series is not a passed one.
//! * [`reject`] stamps a run with that flag and `RejectedIntegrity`.
//! * [`LAYER1_FAMILY_PREFIX`] is how an experiment says it is a Layer 1 experiment, and
//!   [`crate::gates::GateRunner`] reads it, so a plain `gate0` call on one is *refused*
//!   without a verdict rather than silently skipping the check.
//!
//! The truncation re-runs are compute, not trials: they re-run the same function on
//! prefixes of the same data and look at nothing out of sample, so the job that makes
//! them is not a counted kind (`JobKind::Layer1Run` counts the one run, never the cuts).

use crate::position_series::TruncationVerdict;
use crate::run::{Flag, RunResult, RunStatus};

pub use crate::position_series::{TRUNCATION_CODE as CODE, TRUNCATION_NOT_RUN_CODE as NOT_RUN_CODE};

/// An experiment whose `strategy_family` starts with this is a Layer 1 experiment: its
/// Gate 0 needs a truncation verdict.
pub const LAYER1_FAMILY_PREFIX: &str = "layer1:";

/// Whether an experiment's family names it a Layer 1 research strategy.
#[must_use]
pub fn is_layer1_family(strategy_family: &str) -> bool {
    strategy_family.starts_with(LAYER1_FAMILY_PREFIX)
}

/// The flag Gate 0 attaches. `None` for a pass only; an absent verdict is a flag.
#[must_use]
pub fn check(verdict: Option<&TruncationVerdict>) -> Option<Flag> {
    match verdict {
        Some(v) => v.gate_flag(),
        None => Some(Flag {
            code: NOT_RUN_CODE.into(),
            detail: "the truncation test did not run: no verdict was supplied for this Layer 1 \
                     experiment, and an unchecked position series is not a passed one"
                .into(),
        }),
    }
}

/// Stamp a run with the verdict. Returns whether the run was rejected.
///
/// A rejected run keeps its numbers (it is stored, and counted) but its status says
/// `RejectedIntegrity`, which is what every downstream reader refuses to promote.
pub fn reject(result: &mut RunResult, verdict: Option<&TruncationVerdict>) -> bool {
    match check(verdict) {
        Some(flag) => {
            result.status = RunStatus::RejectedIntegrity;
            result.integrity_flags.push(flag);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::position_series::TruncationFailure;

    #[test]
    fn only_a_pass_raises_no_flag_and_a_missing_verdict_is_a_flag() {
        assert!(check(Some(&TruncationVerdict::Pass { cuts_checked: 8 })).is_none());
        let missing = check(None).expect("no verdict is a flag");
        assert_eq!(missing.code, NOT_RUN_CODE);
        let not_computable = check(Some(&TruncationVerdict::NotComputable { reason: "no case".into() })).unwrap();
        assert_eq!(not_computable.code, NOT_RUN_CODE);
    }

    #[test]
    fn a_failure_names_the_first_differing_timestamp() {
        let v = TruncationVerdict::Fail(TruncationFailure {
            cut_ns: 5_000_000_000,
            first_differing_ns: 3_000_000_000,
            full: Some(0.5),
            truncated: None,
        });
        let flag = check(Some(&v)).unwrap();
        assert_eq!(flag.code, CODE);
        assert!(flag.detail.contains("1970-01-01T00:00:03"), "{}", flag.detail);
    }

    #[test]
    fn the_family_prefix_marks_a_layer1_experiment() {
        assert!(is_layer1_family("layer1:momo_v2"));
        assert!(!is_layer1_family("momo_v2"));
    }
}
