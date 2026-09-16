//! Getting Gate 15's evidence from the fill stream into the ledger (T-0.2).
//!
//! The gate runs later, in a different process, from
//! [`ledger::TrialLedger::record_statistic`] rows — never from a job manifest,
//! because a statistic the submitter supplies is a statistic the submitter
//! chose (ADR-P2-31, migration 0057). Gate 15 is where that matters most: it is
//! the only gate whose evidence cannot be re-derived by running anything again,
//! so a self-reported `paper_signal_match` of 0.999 would be both undetectable
//! and fatal.
//!
//! Six statistics are written, not four. The four §12.6 measurements are what
//! the gate judges on; `paper_observed_days` and
//! `paper_observation_window_days` are the count and the window it judged over,
//! recorded because a gate passed on four observations and one passed on four
//! hundred are different claims.

use domain::forward::Gate15Evidence;
use ledger::{LedgerError, TrialLedger};
use uuid::Uuid;

/// Where these numbers came from, written into every statistic row.
pub const PRODUCED_BY: &str = "reconciliation:forward_observation";

/// Record one deployment's forward evidence against its trial.
///
/// `evidence` must have been built by
/// [`Gate15Evidence::from_observations`](domain::forward::Gate15Evidence::from_observations),
/// which is the only place the observed-knowledge filter is applied.
///
/// # Errors
/// Any [`LedgerError`] from the underlying recorder. Nothing is written
/// partially on purpose is *not* claimed here: `record_statistic` is one row per
/// call, so a failure part-way leaves the earlier rows written. That is the
/// right trade — the alternative is a transaction spanning the gate's inputs,
/// and a missing statistic makes the gate inconclusive, which is safe, while a
/// rolled-back one makes it silently stale.
pub fn record_gate15_statistics(
    led: &dyn TrialLedger,
    tenant_id: &str,
    trial_id: Uuid,
    evidence: &Gate15Evidence,
) -> Result<(), LedgerError> {
    let rows: [(&str, f64); 6] = [
        ("paper_signal_match", evidence.signal_match),
        ("paper_slippage_ratio", evidence.slippage_ratio),
        ("paper_turnover_ratio", evidence.turnover_ratio),
        (
            "paper_model_rejects",
            f64::from(evidence.model_attributable_rejects),
        ),
        ("paper_observed_days", f64::from(evidence.observed_days)),
        (
            "paper_observation_window_days",
            f64::from(evidence.window.days()),
        ),
    ];
    for (name, value) in rows {
        led.record_statistic(tenant_id, trial_id, name, value, PRODUCED_BY)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ledger::TRIAL_STATISTICS;

    /// Every name this module writes has to be in the closed vocabulary the
    /// database CHECKs (migration 0106). A misspelled name is a statistic the
    /// gate silently never finds, and a gate that never finds its input is
    /// inconclusive forever without anybody noticing.
    #[test]
    fn every_name_written_is_in_the_closed_vocabulary() {
        for name in [
            "paper_signal_match",
            "paper_slippage_ratio",
            "paper_turnover_ratio",
            "paper_model_rejects",
            "paper_observed_days",
            "paper_observation_window_days",
        ] {
            assert!(
                TRIAL_STATISTICS.contains(&name),
                "{name} is not in TRIAL_STATISTICS; the database CHECK will reject it"
            );
        }
    }
}
