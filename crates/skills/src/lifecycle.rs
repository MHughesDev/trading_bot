//! What happens to a skill after it is admitted (Set R-a RA-3.4, RA-3.6, RA-3.7,
//! RA-4.5; Set R-b RB-2.1, RB-2.4, RB-2.5, RB-2.6; AGENT-003 §6.3, §6.4).
//!
//! Pure decisions over numbers the registry holds. The state is in Postgres
//! (`skill_registry.*`); the *rule* is here, so it is one definition the API, the
//! curator and the tests share, and every threshold is an argument rather than a
//! constant (AGENT-003 open questions 3 and 6 are configuration).
//!
//! # Contents
//!
//! | what | task |
//! |---|---|
//! | [`provisional_verdict`] | RA-4.5: provisional becomes admitted after N exposures |
//! | [`SequentialDesign`], [`replay_verdict`] | RA-3.4, RB-2.4: paired, sequential, pre-registered stopping rule |
//! | [`savings_condition`] | RA-3.6: T5, at least 20% cheaper at equal success |
//! | [`qualifies`], [`StopNudge`] | RA-3.7: proposal triggers T1 to T4, one nudge per session |
//! | [`check_promotion`] | RB-2.1: the six conditions for global |
//! | [`retire_decision`] | RB-2.6: archive on no activation or no contribution |
//! | [`check_model_change`] | RB-2.5 (SK-17): re-evaluate the library before a model default moves |

use std::collections::BTreeMap;

use crate::registry::Reason;

// ── RA-4.5 ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvisionalVerdict {
    /// Not enough exposures yet.
    Stays { exposures: u32, needed: u32 },
    /// Enough exposures, non-negative balance.
    Promote,
    /// Enough exposures and a negative helpfulness balance: stays provisional and is
    /// flagged for a human, because a skill people vote down is not admitted by waiting.
    Flagged { balance: i64 },
}

/// Provisional to admitted needs `n` exposures with a non-negative helpfulness
/// balance (ADR-RA-04: the default `n` is 20; the caller passes the configured one).
#[must_use]
pub fn provisional_verdict(exposures: u32, balance: i64, n: u32) -> ProvisionalVerdict {
    if exposures < n {
        ProvisionalVerdict::Stays {
            exposures,
            needed: n,
        }
    } else if balance < 0 {
        ProvisionalVerdict::Flagged { balance }
    } else {
        ProvisionalVerdict::Promote
    }
}

// ── RA-3.4 / RB-2.4: paired, sequential, pre-registered ─────────────────────

/// One matched task run with and without the skill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pair {
    /// The model actually activated the skill in the with-skill arm.
    pub activated: bool,
    pub with_success: bool,
    pub without_success: bool,
    pub with_cost: f64,
    pub without_cost: f64,
}

/// A sequential sign test on the discordant pairs, fixed *before* the first pair
/// (SK-16: "the design pre-registers its stopping rule").
///
/// Pairs where both arms agree carry no information about the skill and are ignored.
/// Of the rest, `p1` is the share the skill must win to count as helping; the
/// log-likelihood ratio against a fair coin is compared with Wald's two boundaries.
/// `max_pairs` is the stopping rule's hard cap: reaching it without crossing the
/// upper boundary is *no demonstrated contribution*, never "keep going".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequentialDesign {
    pub alpha: f64,
    pub beta: f64,
    pub p1: f64,
    pub min_pairs: usize,
    pub max_pairs: usize,
}

impl Default for SequentialDesign {
    fn default() -> Self {
        Self {
            alpha: 0.05,
            beta: 0.20,
            p1: 0.80,
            min_pairs: 3,
            max_pairs: 40,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// The skill changed outcomes in its favour at the design's error rates.
    Helps { pairs: usize, llr: f64 },
    /// The skill did not demonstrably help: the lower boundary, or the cap, was reached.
    NoContribution { pairs: usize, llr: f64 },
    /// Neither boundary and below the cap: run another pair.
    Continue { pairs: usize, llr: f64 },
}

impl SequentialDesign {
    /// Evaluate the pairs observed so far, in order.
    #[must_use]
    pub fn decide(&self, pairs: &[Pair]) -> Decision {
        let up = ((1.0 - self.beta) / self.alpha).ln();
        let down = (self.beta / (1.0 - self.alpha)).ln();
        let (win, lose) = ((self.p1 / 0.5).ln(), ((1.0 - self.p1) / 0.5).ln());
        let mut llr = 0.0;
        for (i, p) in pairs.iter().enumerate() {
            match (p.with_success, p.without_success) {
                (true, false) => llr += win,
                (false, true) => llr += lose,
                _ => {}
            }
            let n = i + 1;
            if n >= self.min_pairs {
                if llr >= up {
                    return Decision::Helps { pairs: n, llr };
                }
                if llr <= down {
                    return Decision::NoContribution { pairs: n, llr };
                }
            }
            if n >= self.max_pairs {
                return Decision::NoContribution { pairs: n, llr };
            }
        }
        Decision::Continue {
            pairs: pairs.len(),
            llr,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplayFailure {
    NoPairs,
    /// "A skill the model never activates fails": the with-skill arm never used it.
    NeverActivated,
    /// "A skill that changes nothing fails": every pair agreed.
    ChangedNothing,
    /// The sequential rule did not reach the helps boundary.
    NotDemonstrated(Decision),
    /// Fewer than the two variants the verification needs (RA-3.4).
    TooFewVariants { variants: usize },
}

/// The replay verdict for admission (RA-3.4): the source task plus at least two
/// variants on synthetic instruments, paired and sequential.
///
/// `variants` is how many distinct task variants the pairs cover.
///
/// # Errors
/// [`ReplayFailure`], naming which condition failed.
pub fn replay_verdict(
    pairs: &[Pair],
    variants: usize,
    design: &SequentialDesign,
) -> Result<Decision, ReplayFailure> {
    if pairs.is_empty() {
        return Err(ReplayFailure::NoPairs);
    }
    if variants < 3 {
        return Err(ReplayFailure::TooFewVariants { variants });
    }
    if !pairs.iter().any(|p| p.activated) {
        return Err(ReplayFailure::NeverActivated);
    }
    if pairs.iter().all(|p| p.with_success == p.without_success) {
        return Err(ReplayFailure::ChangedNothing);
    }
    match design.decide(pairs) {
        d @ Decision::Helps { .. } => Ok(d),
        d => Err(ReplayFailure::NotDemonstrated(d)),
    }
}

// ── RA-3.6: the savings condition (T5) ──────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum SavingsFailure {
    NoPairs,
    /// The with-skill arm succeeded less often: savings at lower success is not savings.
    LowerSuccess { with: f64, without: f64 },
    /// Cheaper by less than the required share.
    TooSmall { saved: f64, required: f64 },
}

/// Tokens or tool calls reduced by at least `required` (default 0.20) **at equal
/// success**. Returns the share saved.
///
/// # Errors
/// [`SavingsFailure`].
pub fn savings_condition(pairs: &[Pair], required: f64) -> Result<f64, SavingsFailure> {
    if pairs.is_empty() {
        return Err(SavingsFailure::NoPairs);
    }
    let n = pairs.len() as f64;
    let with = pairs.iter().filter(|p| p.with_success).count() as f64 / n;
    let without = pairs.iter().filter(|p| p.without_success).count() as f64 / n;
    if with < without {
        return Err(SavingsFailure::LowerSuccess { with, without });
    }
    let (cw, co): (f64, f64) = (
        pairs.iter().map(|p| p.with_cost).sum(),
        pairs.iter().map(|p| p.without_cost).sum(),
    );
    let saved = if co > 0.0 { (co - cw) / co } else { 0.0 };
    if saved + 1e-12 < required {
        return Err(SavingsFailure::TooSmall { saved, required });
    }
    Ok(saved)
}

// ── RA-3.7: triggers and the nudge ──────────────────────────────────────────

/// What the platform knows about a session when it asks "is there a skill here?".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TriggerEvidence {
    pub repeats_in_project: u32,
    pub repeats_across_projects: u32,
    pub tool_calls: u32,
    pub error_then_fix: bool,
    pub platform_verified: bool,
    pub rejection_pitfall: bool,
    pub human_requested: bool,
    // The refusals, which beat every trigger.
    pub speculative: bool,
    pub one_off: bool,
    pub single_tool_wrapper: bool,
    pub has_symbols_or_tuned_values: bool,
    pub only_untrusted_sources: bool,
}

/// Which trigger a session meets, or why none does.
///
/// T1 repetition (three times in a project, or twice across projects); T2 hard-won
/// (at least 15 tool calls, or an error then a fix) **and** platform-verified; T3 a
/// pitfall a rejection taught; T4 a human asked. Never speculative, one-off,
/// a single-tool wrapper, content with symbols or tuned values, or sourced only from
/// untrusted text.
///
/// # Errors
/// The list of reasons, so a refusal says what to change.
pub fn qualifies(e: &TriggerEvidence) -> Result<Reason, Vec<&'static str>> {
    let mut no = Vec::new();
    if e.speculative {
        no.push("speculative: no verified trajectory behind it");
    }
    if e.one_off {
        no.push("one-off: it happened once");
    }
    if e.single_tool_wrapper {
        no.push("a single-tool wrapper adds nothing over the tool");
    }
    if e.has_symbols_or_tuned_values {
        no.push("it contains symbols or tuned values: that is a finding, not a procedure");
    }
    if e.only_untrusted_sources {
        no.push("its only source was untrusted text");
    }
    if !no.is_empty() {
        return Err(no);
    }
    if e.human_requested {
        return Ok(Reason::T4);
    }
    if e.repeats_in_project >= 3 || e.repeats_across_projects >= 2 {
        return Ok(Reason::T1);
    }
    if (e.tool_calls >= 15 || e.error_then_fix) && e.platform_verified {
        return Ok(Reason::T2);
    }
    if e.rejection_pitfall {
        return Ok(Reason::T3);
    }
    Err(vec!["no trigger met: not repeated, not hard-won and verified, no pitfall, no request"])
}

/// The single Stop-hook nudge (SK-15): at most one per session. A second `Stop` in the
/// same session does not nudge again, however many triggers it meets.
#[derive(Clone, Debug, Default)]
pub struct StopNudge {
    fired: bool,
}

impl StopNudge {
    /// Returns the nudge text the first time a trigger is met, `None` afterwards.
    pub fn on_stop(&mut self, evidence: &TriggerEvidence) -> Option<String> {
        if self.fired {
            return None;
        }
        let reason = qualifies(evidence).ok()?;
        self.fired = true;
        Some(format!(
            "This session met proposal trigger {}. If the procedure is worth keeping, call \
             `propose_skill` citing this run; otherwise ignore this. You will not be asked again.",
            reason.as_str()
        ))
    }
}

// ── RB-2.1: promotion to global ─────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Condition {
    CrossProjectEvals,
    Sanitiser,
    OwnerConsent,
    SecurityScan,
    IndependentReviewer,
    NoTaint,
}

impl Condition {
    pub const ALL: [Condition; 6] = [
        Condition::CrossProjectEvals,
        Condition::Sanitiser,
        Condition::OwnerConsent,
        Condition::SecurityScan,
        Condition::IndependentReviewer,
        Condition::NoTaint,
    ];
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CrossProjectEvals => "cross_project_evals",
            Self::Sanitiser => "sanitiser",
            Self::OwnerConsent => "owner_consent",
            Self::SecurityScan => "security_scan",
            Self::IndependentReviewer => "independent_reviewer",
            Self::NoTaint => "no_taint",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == s)
    }
}

/// A recorded condition: who established it and whether it held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub passed: bool,
    pub by: String,
}

#[derive(Clone, Debug, Default)]
pub struct PromotionFile {
    pub author: String,
    pub recorded: BTreeMap<Condition, Recorded>,
    /// Any finding in the skill's provenance carries the `tainted` mark (KM-10).
    pub provenance_tainted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("promotion to global refused; unmet: {}", .0.iter().map(|(c, why)| format!("{} ({why})", c.as_str())).collect::<Vec<_>>().join(", "))]
pub struct PromotionRefused(pub Vec<(Condition, String)>);

/// All six conditions must be recorded **and** hold. An unrecorded one is refused as
/// readily as a failed one; the reviewer may not be the author; and tainted
/// provenance refuses the promotion whatever else was recorded.
///
/// # Errors
/// [`PromotionRefused`] naming every unmet condition.
pub fn check_promotion(f: &PromotionFile) -> Result<(), PromotionRefused> {
    let mut unmet = Vec::new();
    for c in Condition::ALL {
        match f.recorded.get(&c) {
            None => unmet.push((c, "not recorded".to_string())),
            Some(r) if !r.passed => unmet.push((c, "recorded as failed".to_string())),
            Some(r) => {
                if c == Condition::IndependentReviewer && r.by == f.author {
                    unmet.push((c, "the reviewer is the author".to_string()));
                }
            }
        }
    }
    if f.provenance_tainted && !unmet.iter().any(|(c, _)| *c == Condition::NoTaint) {
        unmet.push((
            Condition::NoTaint,
            "provenance includes a tainted finding".to_string(),
        ));
    }
    if unmet.is_empty() {
        Ok(())
    } else {
        Err(PromotionRefused(unmet))
    }
}

// ── RB-2.6: retirement ──────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Retirement {
    Keep,
    Archive(&'static str),
}

/// Archive after `n` exposures with no activation, or with a leave-one-out
/// contribution at most zero. Below `n` exposures nothing is concluded.
#[must_use]
pub fn retire_decision(
    exposures: u32,
    activations: u32,
    loo_contribution: Option<f64>,
    n: u32,
) -> Retirement {
    if exposures < n {
        return Retirement::Keep;
    }
    if activations == 0 {
        return Retirement::Archive("never activated in the exposure window");
    }
    if loo_contribution.is_some_and(|c| c <= 0.0) {
        return Retirement::Archive("leave-one-out contribution at most zero");
    }
    Retirement::Keep
}

// ── RB-2.5 (SK-17) ──────────────────────────────────────────────────────────

/// A completed run of the whole library on one model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReEvaluation {
    pub model: String,
    pub completed: bool,
    /// Skills whose tests failed under this model.
    pub failing: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ModelChangeRefused {
    #[error("no completed library re-evaluation exists for model {0:?}; run it before making this the default")]
    NotEvaluated(String),
}

/// A model default may change only once the library has been re-run on the new
/// model. Returns the skills to auto-disable (those failing under it).
///
/// # Errors
/// [`ModelChangeRefused`] when there is no completed record for `new_model`.
pub fn check_model_change(
    new_model: &str,
    records: &[ReEvaluation],
) -> Result<Vec<String>, ModelChangeRefused> {
    records
        .iter()
        .rev()
        .find(|r| r.model == new_model && r.completed)
        .map(|r| r.failing.clone())
        .ok_or_else(|| ModelChangeRefused::NotEvaluated(new_model.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(w: bool, wo: bool) -> Pair {
        Pair {
            activated: true,
            with_success: w,
            without_success: wo,
            with_cost: 80.0,
            without_cost: 100.0,
        }
    }

    #[test]
    fn provisional_stays_below_n_and_is_flagged_at_n_with_a_negative_balance() {
        assert_eq!(
            provisional_verdict(19, 5, 20),
            ProvisionalVerdict::Stays {
                exposures: 19,
                needed: 20
            }
        );
        assert_eq!(provisional_verdict(20, 0, 20), ProvisionalVerdict::Promote);
        assert_eq!(
            provisional_verdict(25, -1, 20),
            ProvisionalVerdict::Flagged { balance: -1 }
        );
        // The threshold is an argument.
        assert_eq!(provisional_verdict(5, 1, 5), ProvisionalVerdict::Promote);
    }

    #[test]
    fn a_skill_that_wins_every_discordant_pair_is_shown_to_help_and_stops() {
        let d = SequentialDesign::default();
        let pairs: Vec<Pair> = (0..10).map(|_| pair(true, false)).collect();
        match d.decide(&pairs) {
            Decision::Helps { pairs, .. } => assert!(pairs <= 6, "stops early, not at the cap: {pairs}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_skill_with_no_contribution_is_stopped_at_the_cap_or_the_lower_boundary() {
        let d = SequentialDesign::default();
        // Two wins then a loss, repeating: the LLR hovers near zero and the cap decides.
        let pairs: Vec<Pair> = (0..80).map(|i| pair(i % 3 != 2, i % 3 == 2)).collect();
        assert!(matches!(
            d.decide(&pairs),
            Decision::NoContribution { pairs: 40, .. }
        ));
        // All losses cross the lower boundary quickly.
        let losses: Vec<Pair> = (0..10).map(|_| pair(false, true)).collect();
        assert!(matches!(d.decide(&losses), Decision::NoContribution { pairs, .. } if pairs <= 4));
        // Too few pairs: keep going.
        assert!(matches!(d.decide(&[pair(true, false)]), Decision::Continue { .. }));
    }

    #[test]
    fn replay_fails_a_skill_never_activated_one_that_changes_nothing_and_too_few_variants() {
        let d = SequentialDesign::default();
        let wins: Vec<Pair> = (0..8).map(|_| pair(true, false)).collect();
        assert!(replay_verdict(&wins, 3, &d).is_ok());
        let mut idle = wins.clone();
        idle.iter_mut().for_each(|p| p.activated = false);
        assert_eq!(replay_verdict(&idle, 3, &d), Err(ReplayFailure::NeverActivated));
        let same: Vec<Pair> = (0..8).map(|_| pair(true, true)).collect();
        assert_eq!(replay_verdict(&same, 3, &d), Err(ReplayFailure::ChangedNothing));
        assert_eq!(
            replay_verdict(&wins, 2, &d),
            Err(ReplayFailure::TooFewVariants { variants: 2 })
        );
        assert_eq!(replay_verdict(&[], 3, &d), Err(ReplayFailure::NoPairs));
    }

    #[test]
    fn savings_of_ten_percent_fail_and_twenty_pass_only_at_equal_success() {
        let mk = |with_cost: f64, w: bool| {
            vec![
                Pair {
                    activated: true,
                    with_success: w,
                    without_success: true,
                    with_cost,
                    without_cost: 100.0,
                };
                4
            ]
        };
        assert!(matches!(
            savings_condition(&mk(90.0, true), 0.20),
            Err(SavingsFailure::TooSmall { .. })
        ));
        assert!((savings_condition(&mk(80.0, true), 0.20).unwrap() - 0.20).abs() < 1e-9);
        // Cheaper but worse is not savings.
        assert!(matches!(
            savings_condition(&mk(10.0, false), 0.20),
            Err(SavingsFailure::LowerSuccess { .. })
        ));
    }

    #[test]
    fn each_trigger_fires_alone_and_each_refusal_beats_every_trigger() {
        let t1 = TriggerEvidence {
            repeats_in_project: 3,
            ..Default::default()
        };
        assert_eq!(qualifies(&t1), Ok(Reason::T1));
        let t1b = TriggerEvidence {
            repeats_across_projects: 2,
            ..Default::default()
        };
        assert_eq!(qualifies(&t1b), Ok(Reason::T1));
        let t2 = TriggerEvidence {
            tool_calls: 15,
            platform_verified: true,
            ..Default::default()
        };
        assert_eq!(qualifies(&t2), Ok(Reason::T2));
        // Hard-won but unverified is not T2.
        let unverified = TriggerEvidence {
            tool_calls: 40,
            ..Default::default()
        };
        assert!(qualifies(&unverified).is_err());
        let t3 = TriggerEvidence {
            rejection_pitfall: true,
            ..Default::default()
        };
        assert_eq!(qualifies(&t3), Ok(Reason::T3));
        let t4 = TriggerEvidence {
            human_requested: true,
            ..Default::default()
        };
        assert_eq!(qualifies(&t4), Ok(Reason::T4));
        for spoil in [
            |e: &mut TriggerEvidence| e.speculative = true,
            |e: &mut TriggerEvidence| e.one_off = true,
            |e: &mut TriggerEvidence| e.single_tool_wrapper = true,
            |e: &mut TriggerEvidence| e.has_symbols_or_tuned_values = true,
            |e: &mut TriggerEvidence| e.only_untrusted_sources = true,
        ] {
            let mut e = TriggerEvidence {
                human_requested: true,
                repeats_in_project: 9,
                ..Default::default()
            };
            spoil(&mut e);
            assert!(qualifies(&e).is_err(), "{e:?}");
        }
    }

    #[test]
    fn a_session_nudges_once_and_a_second_stop_does_not() {
        let e = TriggerEvidence {
            human_requested: true,
            ..Default::default()
        };
        let mut n = StopNudge::default();
        assert!(n.on_stop(&TriggerEvidence::default()).is_none(), "no trigger, no nudge, and it is not spent");
        assert!(n.on_stop(&e).is_some());
        assert!(n.on_stop(&e).is_none());
    }

    fn full_file() -> PromotionFile {
        let mut f = PromotionFile {
            author: "agent".into(),
            ..Default::default()
        };
        for c in Condition::ALL {
            f.recorded.insert(
                c,
                Recorded {
                    passed: true,
                    by: if c == Condition::IndependentReviewer {
                        "mason".into()
                    } else {
                        "platform".into()
                    },
                },
            );
        }
        f
    }

    #[test]
    fn promotion_needs_all_six_and_names_the_missing_one() {
        assert!(check_promotion(&full_file()).is_ok());
        for c in Condition::ALL {
            let mut f = full_file();
            f.recorded.remove(&c);
            let e = check_promotion(&f).unwrap_err();
            assert_eq!(e.0.len(), 1, "{c:?}");
            assert_eq!(e.0[0].0, c);
        }
        let mut f = full_file();
        f.recorded.get_mut(&Condition::SecurityScan).unwrap().passed = false;
        assert!(check_promotion(&f).is_err());
    }

    #[test]
    fn the_reviewer_may_not_be_the_author_and_taint_refuses_with_the_other_five() {
        let mut f = full_file();
        f.recorded.get_mut(&Condition::IndependentReviewer).unwrap().by = "agent".into();
        let e = check_promotion(&f).unwrap_err();
        assert_eq!(e.0[0].0, Condition::IndependentReviewer);
        let mut f = full_file();
        f.provenance_tainted = true;
        let e = check_promotion(&f).unwrap_err();
        assert_eq!(e.0, vec![(Condition::NoTaint, "provenance includes a tainted finding".to_string())]);
    }

    #[test]
    fn retirement_waits_for_n_and_then_follows_activation_and_contribution() {
        assert_eq!(retire_decision(5, 0, None, 20), Retirement::Keep);
        assert!(matches!(retire_decision(20, 0, None, 20), Retirement::Archive(_)));
        assert!(matches!(retire_decision(20, 6, Some(0.0), 20), Retirement::Archive(_)));
        assert_eq!(retire_decision(20, 6, Some(0.1), 20), Retirement::Keep);
        assert_eq!(retire_decision(20, 6, None, 20), Retirement::Keep);
    }

    #[test]
    fn a_model_default_cannot_change_without_a_completed_reevaluation() {
        let recs = vec![
            ReEvaluation { model: "m2".into(), completed: false, failing: vec![] },
            ReEvaluation { model: "m3".into(), completed: true, failing: vec!["a".into()] },
        ];
        assert!(check_model_change("m2", &recs).is_err());
        assert!(check_model_change("m9", &recs).is_err());
        assert_eq!(check_model_change("m3", &recs).unwrap(), vec!["a".to_string()]);
    }
}
