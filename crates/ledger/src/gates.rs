//! The durable record of a gate decision (SPEC §12.3; plan 2.14, 5.3, 5.5).
//!
//! A verdict is only meaningful with the bar it was judged against. "Passed" on
//! its own cannot be checked a year later, cannot be compared across a threshold
//! change, and cannot be aggregated into a pass rate that means anything — which
//! is why every row here carries its `profile_id`, the statistic that decided it,
//! and the threshold that statistic was compared to.
//!
//! This is deliberately separate from the funnel's own in-memory `GateLedger`
//! (`backtest::gates`). That one is the working record of one Experiment's run
//! through five stages; this is the tenant's permanent, append-only record of
//! sixteen numbered gates, and it is what `§16.2`'s "gate pass rate by profile"
//! and the gate-by-gate UI read.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::LedgerError;

/// One gate's decision, as recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateRecord {
    /// What was judged. At least one of `experiment_id` / `trial_id` is present;
    /// a verdict about nothing is not representable.
    pub experiment_id: Option<String>,
    pub trial_id: Option<Uuid>,
    pub campaign_id: Option<Uuid>,
    /// The versioned threshold set this verdict was judged under (INV-23).
    pub profile_id: String,
    /// 1..=16, per §12.3.
    pub gate_no: i32,
    pub gate_name: String,
    pub passed: bool,
    /// The number that decided it, and the bar it was compared against. Both
    /// optional because a few gates are structural (Gate 1 is a hash existing),
    /// but a gate that computes a statistic must record it.
    pub statistic: Option<f64>,
    pub threshold: Option<f64>,
    pub detail: String,
    /// Study, run or artifact ids constituting the evidence.
    pub evidence: Vec<String>,
    /// INV-3: a significance verdict (Gates 8 and 14) carries its `N_eff` and the
    /// trial count it was computed over, or the database refuses the row.
    pub n_eff: Option<f64>,
    pub trial_count_at_eval: Option<i64>,
}

impl GateRecord {
    /// A structural verdict: something either exists or it does not.
    #[must_use]
    pub fn structural(
        profile_id: impl Into<String>,
        gate_no: i32,
        gate_name: impl Into<String>,
        passed: bool,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            experiment_id: None,
            trial_id: None,
            campaign_id: None,
            profile_id: profile_id.into(),
            gate_no,
            gate_name: gate_name.into(),
            passed,
            statistic: None,
            threshold: None,
            detail: detail.into(),
            evidence: Vec::new(),
            n_eff: None,
            trial_count_at_eval: None,
        }
    }

    /// A measured verdict: a statistic against a threshold.
    #[must_use]
    pub fn measured(
        profile_id: impl Into<String>,
        gate_no: i32,
        gate_name: impl Into<String>,
        passed: bool,
        statistic: f64,
        threshold: f64,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            statistic: Some(statistic),
            threshold: Some(threshold),
            ..Self::structural(profile_id, gate_no, gate_name, passed, detail)
        }
    }

    #[must_use]
    pub fn for_experiment(mut self, experiment_id: impl Into<String>) -> Self {
        self.experiment_id = Some(experiment_id.into());
        self
    }

    #[must_use]
    pub fn for_trial(mut self, trial_id: Uuid) -> Self {
        self.trial_id = Some(trial_id);
        self
    }

    #[must_use]
    pub fn in_campaign(mut self, campaign_id: Uuid) -> Self {
        self.campaign_id = Some(campaign_id);
        self
    }

    #[must_use]
    pub fn with_evidence(mut self, evidence: Vec<String>) -> Self {
        self.evidence = evidence;
        self
    }

    /// Attach the significance context Gates 8 and 14 require (INV-3).
    #[must_use]
    pub fn with_significance(mut self, n_eff: f64, trial_count: i64) -> Self {
        self.n_eff = Some(n_eff);
        self.trial_count_at_eval = Some(trial_count);
        self
    }

    /// Mirrors `chk_gate_significance_never_naked`, so an invalid record is
    /// refused before it reaches the database as well as by it.
    ///
    /// # Errors
    /// A verdict about nothing, an out-of-range gate number, or a naked
    /// significance claim.
    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.experiment_id.is_none() && self.trial_id.is_none() {
            return Err(LedgerError::Invalid(
                "a gate verdict must name the experiment or the trial it judged".into(),
            ));
        }
        if !(1..=16).contains(&self.gate_no) {
            return Err(LedgerError::Invalid(format!(
                "gate_no {} is outside the sixteen gates of SPEC §12.3",
                self.gate_no
            )));
        }
        if matches!(self.gate_no, 8 | 14)
            && (self.n_eff.is_none() || self.trial_count_at_eval.is_none())
        {
            return Err(LedgerError::Invalid(format!(
                "gate {} is a significance claim and must carry its N_eff and trial count \
                 (INV-3: significance is never naked)",
                self.gate_no
            )));
        }
        Ok(())
    }
}

/// How a tenant's gate pass rate is moving (§16.2).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassRate {
    /// Verdicts on *whole candidates* — a candidate passes when no gate failed it.
    pub recent_decided: i64,
    pub recent_passed: i64,
    pub baseline_decided: i64,
    pub baseline_passed: i64,
}

impl PassRate {
    #[must_use]
    pub fn recent(&self) -> Option<f64> {
        (self.recent_decided > 0).then(|| self.recent_passed as f64 / self.recent_decided as f64)
    }

    #[must_use]
    pub fn baseline(&self) -> Option<f64> {
        (self.baseline_decided > 0)
            .then(|| self.baseline_passed as f64 / self.baseline_decided as f64)
    }

    /// §16.2's alarm: the recent rate drifted more than 2× from the baseline in
    /// either direction. `None` when either side has no verdicts — a rate with no
    /// denominator is not a drift, and reporting one would be inventing a number.
    #[must_use]
    pub fn drifted(&self) -> Option<bool> {
        let (r, b) = (self.recent()?, self.baseline()?);
        if b <= 0.0 {
            // A baseline of exactly zero has no ratio. Any passing at all is the
            // signal worth surfacing.
            return Some(r > 0.0);
        }
        let ratio = r / b;
        Some(!(0.5..=2.0).contains(&ratio))
    }
}

/// Writes and reads gate verdicts.
pub trait GateLog: Send + Sync {
    /// Record one verdict.
    ///
    /// # Errors
    /// An invalid record, or a backend failure.
    fn record_gate(&self, tenant_id: &str, record: &GateRecord) -> Result<Uuid, LedgerError>;

    /// Candidate-level pass rate under one profile, recent window vs everything
    /// before it.
    ///
    /// # Errors
    /// Backend failures.
    fn pass_rate(
        &self,
        tenant_id: &str,
        profile_id: &str,
        window_days: i64,
    ) -> Result<PassRate, LedgerError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_about_nothing_is_refused() {
        let r = GateRecord::structural("strict_v1", 1, "preregistration", true, "hash locked");
        assert!(r.validate().is_err());
        assert!(r.clone().for_experiment("exp-1").validate().is_ok());
        assert!(r.for_trial(Uuid::new_v4()).validate().is_ok());
    }

    #[test]
    fn a_gate_outside_the_sixteen_is_refused() {
        for bad in [0, 17, -1] {
            let r = GateRecord::structural("strict_v1", bad, "x", true, "d").for_experiment("e");
            assert!(r.validate().is_err(), "gate_no {bad} was accepted");
        }
    }

    /// INV-3, in the type as well as the schema: a significance verdict without
    /// its trial count is not a claim anyone can check.
    #[test]
    fn a_naked_significance_verdict_is_refused() {
        for gate in [8, 14] {
            let naked = GateRecord::measured("strict_v1", gate, "sig", true, 0.01, 0.05, "d")
                .for_experiment("e");
            assert!(naked.validate().is_err(), "gate {gate} accepted without N_eff");
            assert!(naked.with_significance(12.0, 300).validate().is_ok());
        }
    }

    #[test]
    fn a_pass_rate_with_no_denominator_reports_no_drift() {
        let empty = PassRate {
            recent_decided: 0,
            recent_passed: 0,
            baseline_decided: 40,
            baseline_passed: 4,
        };
        assert_eq!(empty.recent(), None);
        assert_eq!(empty.drifted(), None, "no verdicts is not a drift");
    }

    #[test]
    fn drift_fires_in_both_directions() {
        let base = |rd, rp, bd, bp| PassRate {
            recent_decided: rd,
            recent_passed: rp,
            baseline_decided: bd,
            baseline_passed: bp,
        };
        // 10% baseline, 10% recent — steady.
        assert_eq!(base(100, 10, 100, 10).drifted(), Some(false));
        // 10% baseline, 30% recent — three times as many candidates passing.
        assert_eq!(base(100, 30, 100, 10).drifted(), Some(true));
        // 20% baseline, 5% recent — a quarter as many.
        assert_eq!(base(100, 5, 100, 20).drifted(), Some(true));
        // Exactly 2× is not yet drift; just past it is.
        assert_eq!(base(100, 20, 100, 10).drifted(), Some(false));
        assert_eq!(base(100, 21, 100, 10).drifted(), Some(true));
    }

    /// A baseline where nothing ever passed has no ratio; any pass is the signal.
    #[test]
    fn a_zero_baseline_surfaces_the_first_passes() {
        let p = PassRate {
            recent_decided: 20,
            recent_passed: 2,
            baseline_decided: 100,
            baseline_passed: 0,
        };
        assert_eq!(p.drifted(), Some(true));
    }
}
