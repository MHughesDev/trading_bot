//! Gate 16 judges the step (Set T T-3.2; SPEC §12.3 Gate 16, HANDOFF §5.3).
//!
//! Gate 16 used to be a function of the other fifteen verdicts and a profile
//! flag: it could say "the stack passed" and had nothing to say about whether the
//! strategy had *behaved* since. This is the missing input: the completed review
//! period's forward observations (Phase 0), the divergence record (Phase 2) and
//! the profile's paper thresholds.
//!
//! Three outcomes, never two:
//!
//! * [`StepVerdict::Pass`] — the period was observed, nothing diverged, the
//!   forward process measurements clear the profile, and a profile pass is on
//!   record;
//! * [`StepVerdict::Fail`] — the period was observed and something did not clear;
//! * [`StepVerdict::NotComputable`] — the period is not over, or it was not
//!   observed enough to judge. **Not a pass.** A period that cannot be judged has
//!   not been measured, and ADR-P2-27 does not let anything sit unmeasured at its
//!   current size, so the review treats it as a failure of the period while the
//!   verdict keeps the honest name (D-T1: no evidence is not good evidence).
//!
//! The judge is a pure function of [`StepInputs`] and [`PeriodEvidence`], and both
//! are stored whole with the review. Re-running [`judge`] over the stored JSON
//! reproduces the stored outcome: that is what "reproducible" means here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::capital::{self, AllowedFraction};

/// The profile's paper thresholds and the observation floor for a period.
/// No `Default`: a floor nobody declared is a floor of zero, which judges an
/// unobserved period as clean.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepThresholds {
    /// Fewest `observed` forward bars a period needs to be judged at all.
    pub min_observed_bars: u32,
    pub paper_signal_match_gte: f64,
    pub paper_slippage_ratio_lte: f64,
    pub paper_turnover_tolerance: f64,
    pub paper_model_rejects_max: u32,
}

/// What the judge is told about the deployment, besides the period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepInputs {
    pub profile_id: String,
    /// From the gate-profile row, not the caller's opinion of the profile.
    pub authorises_capital: bool,
    pub thresholds: StepThresholds,
    /// The deployment's capital rung, or `None` when it is not on the capital
    /// ladder at all (a `paper_v1` deployment never is).
    pub current_rung: Option<AllowedFraction>,
}

/// The completed (or not) review period's evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PeriodEvidence {
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub evaluated_at: DateTime<Utc>,
    /// Forward observations with `source = 'observed'` in the period. Backfilled
    /// rows are never counted here (D-T1).
    pub observed_bars: u32,
    /// Divergence halts raised in the period (Phase 2's record).
    pub divergence_halts: u32,
    pub signal_match: Option<f64>,
    pub slippage_ratio: Option<f64>,
    pub turnover_deviation: Option<f64>,
    pub model_rejects: u32,
    /// Gates 1..=15 passed under this profile, on record.
    pub profile_passed: bool,
    /// Every gate verdict on record, `(gate_no, passed)`; used for the capital
    /// authority when the profile can authorise one.
    pub gate_verdicts: Vec<(i32, bool)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum StepVerdict {
    Pass,
    Fail { reasons: Vec<String> },
    NotComputable { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepJudgement {
    pub verdict: StepVerdict,
    /// Whether the capital rung may rise one step on this evidence.
    pub step_up_allowed: bool,
    /// Why not, when it may not (and the period passed).
    pub step_up_refusal: Option<String>,
}

impl StepJudgement {
    #[must_use]
    pub fn passed(&self) -> bool {
        matches!(self.verdict, StepVerdict::Pass)
    }

    /// The reason a review row records for a verdict that is not a pass.
    #[must_use]
    pub fn not_computable_reason(&self) -> Option<&str> {
        match &self.verdict {
            StepVerdict::NotComputable { reason } => Some(reason),
            _ => None,
        }
    }

    /// One human sentence for the review row.
    #[must_use]
    pub fn reason(&self) -> String {
        match &self.verdict {
            StepVerdict::Pass => match (self.step_up_allowed, &self.step_up_refusal) {
                (true, _) => "period passed; Gate 16 permits one rung".to_owned(),
                (false, Some(why)) => format!("period passed; holds: {why}"),
                (false, None) => "period passed; holds".to_owned(),
            },
            StepVerdict::Fail { reasons } => format!("period failed: {}", reasons.join("; ")),
            StepVerdict::NotComputable { reason } => {
                format!("Gate 16 not computable: {reason}")
            }
        }
    }
}

/// Judge one review period.
#[must_use]
pub fn judge(inputs: &StepInputs, ev: &PeriodEvidence) -> StepJudgement {
    let t = &inputs.thresholds;
    if ev.evaluated_at < ev.period_end {
        return not_computable(format!(
            "the period ends at {}; it was evaluated at {}",
            ev.period_end.to_rfc3339(),
            ev.evaluated_at.to_rfc3339()
        ));
    }
    if ev.observed_bars < t.min_observed_bars {
        return not_computable(format!(
            "{} observed forward bars in the period; at least {} are needed to judge it",
            ev.observed_bars, t.min_observed_bars
        ));
    }
    let mut reasons = Vec::new();
    if ev.divergence_halts > 0 {
        reasons.push(format!(
            "{} divergence halt(s) raised in the period",
            ev.divergence_halts
        ));
    }
    match ev.signal_match {
        Some(m) if m >= t.paper_signal_match_gte => {}
        Some(m) => reasons.push(format!(
            "signal match {m:.4} < {:.4}",
            t.paper_signal_match_gte
        )),
        None => return not_computable("signal match was not measured".to_owned()),
    }
    match ev.slippage_ratio {
        Some(r) if r <= t.paper_slippage_ratio_lte => {}
        Some(r) => reasons.push(format!(
            "slippage ratio {r:.4} > {:.4}",
            t.paper_slippage_ratio_lte
        )),
        None => return not_computable("slippage ratio was not measured".to_owned()),
    }
    match ev.turnover_deviation {
        Some(d) if d <= t.paper_turnover_tolerance => {}
        Some(d) => reasons.push(format!(
            "turnover deviation {d:.4} > {:.4}",
            t.paper_turnover_tolerance
        )),
        None => return not_computable("turnover was not measured".to_owned()),
    }
    if ev.model_rejects > t.paper_model_rejects_max {
        reasons.push(format!(
            "{} model-attributable rejects > {}",
            ev.model_rejects, t.paper_model_rejects_max
        ));
    }
    if !ev.profile_passed {
        reasons.push(format!(
            "no pass under `{}` is on record for this deployment",
            inputs.profile_id
        ));
    }
    if !reasons.is_empty() {
        return StepJudgement {
            verdict: StepVerdict::Fail { reasons },
            step_up_allowed: false,
            step_up_refusal: None,
        };
    }
    let (step_up_allowed, step_up_refusal) = step_up(inputs, ev);
    StepJudgement {
        verdict: StepVerdict::Pass,
        step_up_allowed,
        step_up_refusal,
    }
}

fn not_computable(reason: String) -> StepJudgement {
    StepJudgement {
        verdict: StepVerdict::NotComputable { reason },
        step_up_allowed: false,
        step_up_refusal: None,
    }
}

fn step_up(inputs: &StepInputs, ev: &PeriodEvidence) -> (bool, Option<String>) {
    let Some(rung) = inputs.current_rung else {
        return (
            false,
            Some(format!("`{}` is not on the capital ladder", inputs.profile_id)),
        );
    };
    if rung == AllowedFraction::Full {
        return (false, Some("already at the top rung".to_owned()));
    }
    // Gate 16 itself gates on the other fifteen, as the stack does.
    let mut verdicts: Vec<(i32, bool)> = ev
        .gate_verdicts
        .iter()
        .copied()
        .filter(|(n, _)| *n != 16)
        .collect();
    verdicts.push((16, true));
    match capital::authorise(&inputs.profile_id, inputs.authorises_capital, &verdicts) {
        Ok(_) => (true, None),
        Err(why) => (false, Some(why.to_string())),
    }
}

/// The hash of everything the judge was given. Stored with the review; the
/// review's evidence JSON re-hashes to it.
#[must_use]
pub fn evidence_hash(inputs: &StepInputs, ev: &PeriodEvidence) -> String {
    hex::encode(Sha256::digest(
        json!({ "inputs": inputs, "evidence": ev })
            .to_string()
            .as_bytes(),
    ))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::TimeZone;

    pub(crate) fn thresholds() -> StepThresholds {
        StepThresholds {
            min_observed_bars: 20,
            paper_signal_match_gte: 0.9,
            paper_slippage_ratio_lte: 1.5,
            paper_turnover_tolerance: 0.25,
            paper_model_rejects_max: 0,
        }
    }

    pub(crate) fn inputs(
        profile: &str,
        capital: bool,
        rung: Option<AllowedFraction>,
    ) -> StepInputs {
        StepInputs {
            profile_id: profile.into(),
            authorises_capital: capital,
            thresholds: thresholds(),
            current_rung: rung,
        }
    }

    pub(crate) fn evidence() -> PeriodEvidence {
        let end = Utc.with_ymd_and_hms(2026, 10, 15, 0, 0, 0).unwrap();
        PeriodEvidence {
            period_start: end - chrono::Duration::days(14),
            period_end: end,
            evaluated_at: end + chrono::Duration::minutes(1),
            observed_bars: 300,
            divergence_halts: 0,
            signal_match: Some(0.97),
            slippage_ratio: Some(1.1),
            turnover_deviation: Some(0.05),
            model_rejects: 0,
            profile_passed: true,
            gate_verdicts: (1..=15).map(|g| (g, true)).collect(),
        }
    }

    #[test]
    fn a_complete_clean_period_passes_and_a_capital_profile_may_climb() {
        let j = judge(
            &inputs("strict_v1", true, Some(AllowedFraction::Zero)),
            &evidence(),
        );
        assert!(j.passed());
        assert!(j.step_up_allowed, "{j:?}");
    }

    #[test]
    fn an_incomplete_period_is_not_computable_rather_than_a_pass() {
        let mut ev = evidence();
        ev.evaluated_at = ev.period_end - chrono::Duration::hours(1);
        let j = judge(&inputs("strict_v1", true, Some(AllowedFraction::Zero)), &ev);
        assert!(matches!(j.verdict, StepVerdict::NotComputable { .. }));
        assert!(!j.passed() && !j.step_up_allowed);
        assert!(j.not_computable_reason().unwrap().contains("ends at"));
    }

    #[test]
    fn an_unobserved_period_is_not_computable_with_the_counts_in_the_reason() {
        let mut ev = evidence();
        ev.observed_bars = 3;
        let j = judge(&inputs("strict_v1", true, Some(AllowedFraction::Zero)), &ev);
        let why = j.not_computable_reason().expect("not computable");
        assert!(why.contains('3') && why.contains("20"), "{why}");
    }

    #[test]
    fn each_failure_is_named_and_none_of_them_climbs() {
        type Break = fn(&mut PeriodEvidence);
        let cases: [(Break, &str); 5] = [
            (|e| e.divergence_halts = 2, "divergence halt"),
            (|e| e.signal_match = Some(0.5), "signal match"),
            (|e| e.slippage_ratio = Some(9.0), "slippage"),
            (|e| e.model_rejects = 1, "rejects"),
            (|e| e.profile_passed = false, "no pass under"),
        ];
        for (brk, needle) in cases {
            let mut ev = evidence();
            brk(&mut ev);
            let j = judge(&inputs("strict_v1", true, Some(AllowedFraction::Zero)), &ev);
            match &j.verdict {
                StepVerdict::Fail { reasons } => {
                    assert!(
                        reasons.iter().any(|r| r.contains(needle)),
                        "{needle}: {reasons:?}"
                    );
                }
                other => panic!("{needle}: {other:?}"),
            }
            assert!(!j.step_up_allowed);
        }
    }

    #[test]
    fn a_paper_v1_period_can_pass_and_can_never_climb() {
        let j = judge(&inputs("paper_v1", false, None), &evidence());
        assert!(j.passed());
        assert!(!j.step_up_allowed);
        assert!(j
            .step_up_refusal
            .unwrap()
            .contains("not on the capital ladder"));
        // Even handed a rung, the profile's flag refuses it.
        let j = judge(
            &inputs("paper_v1", false, Some(AllowedFraction::Zero)),
            &evidence(),
        );
        assert!(j.passed() && !j.step_up_allowed);
        assert!(j
            .step_up_refusal
            .unwrap()
            .contains("does not authorise capital"));
    }

    #[test]
    fn a_failed_earlier_gate_blocks_the_climb_though_the_period_passed() {
        let mut ev = evidence();
        ev.gate_verdicts[6] = (7, false);
        let j = judge(&inputs("strict_v1", true, Some(AllowedFraction::Ten)), &ev);
        assert!(j.passed());
        assert!(!j.step_up_allowed);
    }

    /// Reproducible: the stored inputs and evidence, round-tripped through JSON,
    /// give the same judgement and the same hash.
    #[test]
    fn the_judgement_reproduces_from_the_stored_evidence() {
        let i = inputs("strict_v1", true, Some(AllowedFraction::Ten));
        let e = evidence();
        let stored = serde_json::to_string(&json!({ "inputs": &i, "evidence": &e })).unwrap();
        let back: serde_json::Value = serde_json::from_str(&stored).unwrap();
        let i2: StepInputs = serde_json::from_value(back["inputs"].clone()).unwrap();
        let e2: PeriodEvidence = serde_json::from_value(back["evidence"].clone()).unwrap();
        assert_eq!(judge(&i, &e), judge(&i2, &e2));
        assert_eq!(evidence_hash(&i, &e), evidence_hash(&i2, &e2));
    }
}
