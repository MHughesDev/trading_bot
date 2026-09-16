//! A refused gate becomes a negative result (AGENT-007 §3.2; Set S task S-4.4).
//!
//! # Why this lives in `crates/api`
//!
//! It needs both sides: `backtest::gates::GateVerdict` and
//! `harness::memory::negative`. Neither crate depends on the other and neither
//! should — the funnel does not need a memory store to run gates, and the memory
//! store does not need a simulator to hold entries. `crates/api` already has
//! both, so the join goes where the two are already visible rather than by
//! making one crate carry the other.
//!
//! # The class is derived from the verdict, never chosen
//!
//! §3.2's rule, and the reason for it: a model that may pick the class will pick
//! the one whose retry condition is "when the platform is bigger", and an idea
//! that can be re-opened by renaming its failure was never closed. So the
//! mapping below reads what the gate reported and nothing else.
//!
//! The interesting case is gate 0. It refuses for two unrelated reasons —
//! a lookahead leak, or a gross edge under the cost floor — and they are
//! different futures for the same idea: one is fixed by repairing the leak and
//! re-running, the other by costs falling or the edge growing. Telling them
//! apart needs the typed codes `integrity_scan` produces, which is why
//! `GateVerdict` now carries them: they were being formatted into a sentence and
//! discarded, leaving a caller to read prose to find out which future it was in.

use backtest::gates::{Gate, GateVerdict};
use harness::memory::negative::{FailureClass, NegativeSource};
use harness::memory::{DataWindow, Entry, Provenance};

/// The class a refused gate produced.
///
/// Returns `None` for a verdict that passed — a passing gate is not a negative
/// result, and a caller that asked anyway has made a mistake worth surfacing
/// rather than answering.
#[must_use]
pub fn class_of(verdict: &GateVerdict) -> Option<FailureClass> {
    if verdict.passed {
        return None;
    }
    Some(match verdict.gate {
        // Gate 0 refuses for two unrelated reasons; the codes say which.
        Gate::Integrity => {
            if verdict.reasons.iter().any(|c| c.starts_with("cost.")) {
                FailureClass::CostsDominate
            } else {
                // Every other integrity flag is a lookahead or a stamping
                // fault, and `leakage` is the conservative reading when the
                // codes do not say: a result whose measurement may be
                // contaminated should not be re-openable by waiting.
                FailureClass::Leakage
            }
        }
        // Gate 1 is single-path sanity: the effect is not there on one honest
        // path. That is `weak_effect` rather than `no_better_than_null`, which
        // is gate 3's word — the null has not been run yet at this point, so
        // claiming the candidate failed against it would be a claim about a
        // comparison nobody made.
        Gate::SinglePath => FailureClass::WeakEffect,
        // Gate 2 is robustness: the result exists and does not survive
        // perturbation. `fragile_parameters` rather than `regime_specific`,
        // because the gate varies the configuration and the windows, not the
        // regime — regime-specificity is a claim about *when* it works, which
        // this gate does not test.
        Gate::Robustness => FailureClass::FragileParameters,
        // Gate 3 is significance, which is exactly the null comparison.
        Gate::Significance => FailureClass::NoBetterThanNull,
        // The vault is one look, ever. A candidate that fails it failed out of
        // sample, which is what `replication_failed` means.
        Gate::Vault => FailureClass::ReplicationFailed,
    })
}

/// The negative result a refused gate produces, or `None` if it passed.
///
/// `window` is the research slice the funnel ran over — the period the claim is
/// about, which the caller has and this does not.
#[must_use]
pub fn entry_for(
    verdict: &GateVerdict,
    ticket_id: &str,
    subject: &str,
    window: DataWindow,
    run_id: &str,
) -> Option<Entry> {
    let class = class_of(verdict)?;
    let source = NegativeSource::GateRefused {
        ticket_id: ticket_id.to_string(),
        gate: format!("{:?}", verdict.gate),
        class,
        // The gate's own summary. Not rewritten: it is the sentence the gate
        // chose to describe its refusal, and a paraphrase here would be a
        // second account of the same event with nothing tying the two together.
        detail: verdict.summary.clone(),
        window,
    };
    Some(harness::memory::negative::derive_negative_result(
        format!("neg_{ticket_id}_{:?}", verdict.gate).to_lowercase(),
        subject,
        &source,
        Provenance::new(run_id, "advance_funnel", "1", &verdict.profile_id),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn verdict(gate: Gate, passed: bool, reasons: &[&str]) -> GateVerdict {
        GateVerdict {
            gate,
            passed,
            summary: "a summary".into(),
            evidence: vec![],
            profile_id: "strict_v1".into(),
            reasons: reasons.iter().map(|s| (*s).to_string()).collect(),
            at: Utc::now(),
        }
    }

    /// A passing gate is not a negative result.
    #[test]
    fn a_gate_that_passed_produces_nothing() {
        assert_eq!(class_of(&verdict(Gate::SinglePath, true, &[])), None);
        assert!(entry_for(
            &verdict(Gate::SinglePath, true, &[]),
            "t_1",
            "ema_cross_v1",
            DataWindow::new("2026-04-01", "2026-06-30"),
            "run_1"
        )
        .is_none());
    }

    /// Gate 0's two futures are told apart by the codes, not by the prose.
    ///
    /// This is the case the typed reasons exist for: a leak is fixed by
    /// repairing it and re-running, and a cost failure by costs falling or the
    /// edge growing. Reading the same class for both would give one of them the
    /// other's retry condition.
    #[test]
    fn gate_zero_distinguishes_a_leak_from_a_cost_failure() {
        assert_eq!(
            class_of(&verdict(Gate::Integrity, false, &["cost.below_floor"])),
            Some(FailureClass::CostsDominate)
        );
        assert_eq!(
            class_of(&verdict(
                Gate::Integrity,
                false,
                &["lookahead.higher_tf_open"]
            )),
            Some(FailureClass::Leakage)
        );
        // No codes at all reads as leakage: a result whose measurement may be
        // contaminated should not be re-openable by waiting.
        assert_eq!(
            class_of(&verdict(Gate::Integrity, false, &[])),
            Some(FailureClass::Leakage)
        );
    }

    /// Each later gate has exactly one honest reading.
    #[test]
    fn every_gate_maps_to_the_class_it_actually_measured() {
        for (gate, expected) in [
            (Gate::SinglePath, FailureClass::WeakEffect),
            (Gate::Robustness, FailureClass::FragileParameters),
            (Gate::Significance, FailureClass::NoBetterThanNull),
            (Gate::Vault, FailureClass::ReplicationFailed),
        ] {
            assert_eq!(
                class_of(&verdict(gate, false, &[])),
                Some(expected),
                "{gate:?}"
            );
        }
    }

    /// The derived entry carries the class, the ticket and the gate's own words.
    #[test]
    fn the_entry_carries_the_ticket_the_class_and_the_gates_own_summary() {
        let mut v = verdict(Gate::Significance, false, &[]);
        v.summary = "p = 0.31 against the declared null over 512 draws".into();
        let e = entry_for(
            &v,
            "t_9",
            "ema_cross_v1",
            DataWindow::new("2026-04-01", "2026-06-30"),
            "run_4",
        )
        .expect("a failed gate produces one");

        assert_eq!(e.kind, harness::memory::Kind::NegativeResult);
        assert_eq!(
            e.extra["failure_class"], "no_better_than_null",
            "the class is the funnel's"
        );
        assert_eq!(e.extra["ticket_id"], "t_9");
        assert!(
            e.claim.contains("0.31") || e.extra["source"]["detail"] == v.summary,
            "the gate's own sentence must survive: a paraphrase would be a second account of \
             one event with nothing tying them together"
        );
        assert_eq!(e.data_window, DataWindow::new("2026-04-01", "2026-06-30"));
        assert_eq!(e.provenance.run_id, "run_4");
    }
}
