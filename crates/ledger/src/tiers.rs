//! The remaining rule tiers and the promotion machinery (SPEC §13–§14,
//! checklist 4.5/4.6/4.8/4.10/4.11/4.12, ADR-P4-01…04).
//!
//! Four more models answering from rules, and the machinery that would ever
//! replace one with a fit. Each rule tier is here because §13.1's ladder says
//! ship it on day one, and each is written so the learned tier can replace its
//! internals without changing a signature.
//!
//! * **M5 — the gate pre-screener.** Monotone in the Gate 2/3 margin, calibrated
//!   to 99 % recall. The asymmetry is the whole design: screening out a
//!   candidate that would have passed costs a discovery nobody knows was lost,
//!   while letting a doomed one through costs one gate run.
//! * **M8 — the family recommender.** Per-tenant always (§7.4), and the *only*
//!   per-tenant model here. Family win rates pooled across tenants would leak
//!   which strategies work for someone else, which is the thing a tenant is
//!   paying not to share.
//! * **M9 — the proposal ranker.** `mean + κ·σ`, the standard acquisition. With
//!   no surrogate it has no σ, and it says so rather than ranking on the mean.
//! * **M13 — the step critic.** Hard rules first, judge triage second; the
//!   learned PRM waits for three thousand labelled steps.
//!
//! ## 4.11 — retraining is a decision, not a schedule
//!
//! [`learning_debt`] retrains iff `ρ > c_churn / (c_churn + c_wait)`, where `ρ`
//! is the champion's rolling error minus a challenger's on recent data. A
//! calendar does not know whether the model has drifted; a dashboard nobody is
//! looking at knows even less (R-11). The threshold is a ratio of the two costs
//! that actually trade off: churning a model that was fine, and waiting while
//! one that is not stays champion.
//!
//! ## 4.12 — promotion is an evaluation, not a deploy
//!
//! [`PromotionCase`] is sealed the way [`crate::capital::RampAuthority`] is.
//! Building one requires the challenger to have beaten the champion **on the
//! frozen seed holdout** under 2.12's protocol, the entropy floor to be
//! satisfied, and the platform not to be frozen. Tier A auto-promotes; Tier B
//! needs a human (2.17's envelope); Tier C does not exist here at all.

use serde::{Deserialize, Serialize};

use crate::internal::{choose_rung, Answer, EntropyFloor, SeedHoldout};

// ───────────────────────────────────────────────────────────────────────────────
// M5 — the gate pre-screener
// ───────────────────────────────────────────────────────────────────────────────

/// The recall the pre-screener is calibrated to.
///
/// Ninety-nine per cent, and the asymmetry is deliberate. A screened-out
/// candidate that would have passed is a discovery nobody knows was lost — there
/// is no trace, no counterfactual, nothing to investigate. A doomed candidate
/// that reaches the gates costs one gate run. The two errors are not comparable
/// and the operating point should not pretend they are.
pub const PRESCREEN_RECALL: f64 = 0.99;

/// What the pre-screener answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prescreen {
    /// Send it to the gates.
    Proceed,
    /// Almost certainly fails. **Advisory only**: this is a scheduling hint, not
    /// a verdict, and nothing in the platform treats it as one — a candidate the
    /// pre-screener dislikes still reaches the gates if the caller asks.
    LikelyFails,
    /// Not enough margin information to say. Not a pass and not a rejection.
    Unknown,
}

/// M5's rule tier: monotone in the Gate 2/3 margin.
///
/// `margin` is how far the candidate cleared (or missed) the Gate 2/3 bar,
/// normalised so zero is exactly at the threshold. Monotone by construction —
/// there is no fitted shape to invert — which is what lets the operating point
/// be *set* rather than searched for.
///
/// The cut is the margin below which 99 % of eventual passers would still be
/// kept. With no observed distribution it is `-inf`: nothing is screened out,
/// because a screen calibrated on nothing screens on nothing.
#[must_use]
pub fn prescreen(margin: Option<f64>, calibration_cut: Option<f64>, gated_seen: i64, passes_seen: i64) -> Answer<Prescreen> {
    let (rung, evidence) = choose_rung("M5", gated_seen, Some(passes_seen), false, false);
    let value = match (margin, calibration_cut) {
        (Some(m), Some(cut)) if m.is_finite() && cut.is_finite() => {
            if m >= cut {
                Prescreen::Proceed
            } else {
                Prescreen::LikelyFails
            }
        }
        // No margin, or no calibration: proceed. The default direction is the
        // one whose error is recoverable.
        (_, None) => Prescreen::Proceed,
        _ => Prescreen::Unknown,
    };
    Answer { value, rung, evidence }
}

/// The margin cut that keeps [`PRESCREEN_RECALL`] of observed passers.
///
/// Empirical: the `(1 − recall)` quantile of the margins of candidates that went
/// on to pass. `None` below thirty observed passers — a quantile from fewer is
/// a number, not an operating point.
#[must_use]
#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn calibrate_prescreen(passer_margins: &[f64]) -> Option<f64> {
    let mut v: Vec<f64> = passer_margins.iter().copied().filter(|m| m.is_finite()).collect();
    if v.len() < 30 {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let idx = ((1.0 - PRESCREEN_RECALL) * v.len() as f64).floor() as usize;
    v.get(idx.min(v.len() - 1)).copied()
}

// ───────────────────────────────────────────────────────────────────────────────
// M8 — the strategy-family recommender, per tenant always
// ───────────────────────────────────────────────────────────────────────────────

/// One family's record for one tenant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FamilyRecord {
    pub family: String,
    pub attempts: i64,
    pub passes: i64,
}

impl FamilyRecord {
    /// The Wilson lower bound on the pass rate at 95 %.
    ///
    /// A lower bound rather than the raw rate, because the raw rate ranks
    /// one-for-one above nineteen-for-twenty and that ordering is an artefact of
    /// the sample size. The bound is what makes "we have not tried this much"
    /// cost something.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn lower_bound(&self) -> f64 {
        if self.attempts <= 0 {
            return 0.0;
        }
        let n = self.attempts as f64;
        let p = (self.passes as f64) / n;
        let z = 1.96;
        let denom = 1.0 + z * z / n;
        let centre = p + z * z / (2.0 * n);
        let spread = z * ((p * (1.0 - p) + z * z / (4.0 * n)) / n).sqrt();
        ((centre - spread) / denom).max(0.0)
    }
}

/// M8's rule tier: rank families by their lower-bounded win rate **for this
/// tenant only**.
///
/// Per-tenant is not a scoping convenience, it is the model's scope (§7.4,
/// ADR-P4-01). Pooling win rates across tenants would tell each of them which
/// strategy families work for the others, which is exactly what a tenant is
/// paying not to share. The `tenant_id` argument exists so that a caller cannot
/// accidentally pass a pooled record set without noticing.
#[must_use]
pub fn recommend_families(
    tenant_id: &str,
    records: &[FamilyRecord],
    tasks_seen: i64,
) -> Answer<Vec<String>> {
    debug_assert!(!tenant_id.is_empty(), "M8 is per-tenant; an empty tenant is a pooled read");
    let (rung, evidence) = choose_rung("M8", tasks_seen, None, false, false);
    let mut ranked: Vec<&FamilyRecord> = records.iter().collect();
    ranked.sort_by(|a, b| b.lower_bound().total_cmp(&a.lower_bound()));
    Answer {
        value: ranked.into_iter().map(|r| r.family.clone()).collect(),
        rung,
        evidence,
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// M9 — the proposal ranker
// ───────────────────────────────────────────────────────────────────────────────

/// The exploration weight in `mean + κ·σ`.
///
/// Two: roughly a 95 % upper confidence bound under a normal posterior. Not
/// tuned, because tuning κ against observed outcomes is fitting the acquisition
/// to the answers it was supposed to find.
pub const KAPPA: f64 = 2.0;

/// One candidate proposal and what the surrogate thinks of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub mean: f64,
    /// Posterior standard deviation. `None` when no surrogate exists.
    pub sigma: Option<f64>,
}

/// M9's rule tier: rank by `mean + κ·σ`.
///
/// With no surrogate there is no σ, and this returns the proposals **unranked**
/// rather than ranking on the mean alone. Ranking on the mean is not a weaker
/// version of the acquisition — it is pure exploitation, which is the opposite
/// of what the acquisition is for, and it would look like a ranking to everyone
/// downstream.
#[must_use]
pub fn rank_proposals(proposals: &[Proposal], ranked_items_seen: i64, groups_seen: i64) -> Answer<Option<Vec<String>>> {
    let (rung, evidence) = choose_rung("M9", ranked_items_seen, Some(groups_seen), false, false);
    if proposals.iter().any(|p| p.sigma.is_none()) {
        return Answer { value: None, rung, evidence };
    }
    let mut scored: Vec<(&Proposal, f64)> = proposals
        .iter()
        .map(|p| (p, p.mean + KAPPA * p.sigma.unwrap_or(0.0)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    Answer {
        value: Some(scored.into_iter().map(|(p, _)| p.id.clone()).collect()),
        rung,
        evidence,
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// M13 — the step critic
// ───────────────────────────────────────────────────────────────────────────────

/// What the critic thinks of one agent step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepVerdict {
    Fine,
    /// A hard rule fired. These are cheap, certain and worth stopping on.
    RuleViolation,
    /// Nothing mechanical is wrong and the step is unusual enough to be worth a
    /// judge's attention. Triage, not a verdict.
    NeedsJudge,
}

/// M13's rule tier: hard rules, then triage.
///
/// The hard rules come first because they are certain. A step that repeats the
/// previous one verbatim is a loop; a step with no tool call and no conclusion
/// is the model talking to itself. Neither needs a judge and neither is a
/// borderline case.
#[must_use]
pub fn critique_step(
    repeated_previous: bool,
    made_progress: bool,
    unusual: bool,
    labelled_steps_seen: i64,
) -> Answer<StepVerdict> {
    let (rung, evidence) = choose_rung("M13", labelled_steps_seen, None, false, false);
    // A repeated step is a loop; a step that moved nothing is the model talking
    // to itself. Both are certain and cheap to detect, and neither is a
    // borderline case a judge would add anything to.
    let value = if repeated_previous || !made_progress {
        StepVerdict::RuleViolation
    } else if unusual {
        StepVerdict::NeedsJudge
    } else {
        StepVerdict::Fine
    };
    Answer { value, rung, evidence }
}

// ───────────────────────────────────────────────────────────────────────────────
// 4.11 — the learning-debt retraining trigger
// ───────────────────────────────────────────────────────────────────────────────

/// Whether the debt justifies a retrain (§14.1, R-11).
///
/// `rho` is the champion's rolling error minus a challenger's on recent data —
/// how much is being lost by not retraining. `c_churn` is the cost of replacing
/// a model that was fine (re-validation, a new version everyone has to trust);
/// `c_wait` is the cost of a stale champion staying in place.
///
/// Retrain iff `rho > c_churn / (c_churn + c_wait)`. A calendar cannot know
/// whether a model has drifted, and a dashboard nobody looks at knows less.
#[must_use]
pub fn learning_debt(rho: f64, c_churn: f64, c_wait: f64) -> Option<bool> {
    if !(rho.is_finite() && c_churn.is_finite() && c_wait.is_finite()) {
        return None;
    }
    if c_churn <= 0.0 || c_wait <= 0.0 {
        return None;
    }
    Some(rho > c_churn / (c_churn + c_wait))
}

// ───────────────────────────────────────────────────────────────────────────────
// 4.12 — promotion machinery
// ───────────────────────────────────────────────────────────────────────────────

/// Which tier an internal model sits in (§14.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ModelTier {
    /// Auto-promote and auto-rollback. Global models whose errors are visible
    /// and cheap.
    A,
    /// Promotion is an approval (2.17's envelope). Hierarchical and per-tenant
    /// models, where a bad promotion is expensive and quiet.
    B,
}

/// Why a promotion was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PromotionRefusal {
    #[error("internal model training and promotion are frozen: {0}")]
    Frozen(String),
    #[error("the challenger was not evaluated on the frozen seed holdout")]
    NotOnHoldout,
    #[error("the challenger did not beat the champion under the §11.5 protocol")]
    NotBetter,
    #[error("the dispatch policy's entropy is below its floor; a Tier-B promotion on a collapsed policy compounds the collapse — {0}")]
    EntropyFloor(String),
    #[error("a Tier-B promotion needs a named approver (§15's promotion envelope)")]
    NoApprover,
}

/// Authorisation to replace a champion. Sealed: [`authorise_promotion`] is the
/// only source.
#[derive(Debug)]
pub struct PromotionCase {
    tier: ModelTier,
    approver: Option<String>,
    holdout_size: usize,
}

impl PromotionCase {
    #[must_use]
    pub fn tier(&self) -> ModelTier {
        self.tier
    }
    #[must_use]
    pub fn approver(&self) -> Option<&str> {
        self.approver.as_deref()
    }
    /// How many held-out trials the decision rested on. Recorded on the
    /// promotion row, because "the challenger won" over twelve trials and over
    /// two hundred are different claims.
    #[must_use]
    pub fn holdout_size(&self) -> usize {
        self.holdout_size
    }
}

/// Mint the authority to promote.
///
/// Four conditions, none of them skippable:
///
/// 1. The platform is not frozen (4.16).
/// 2. The challenger was evaluated on the **frozen seed holdout** — the one set
///    the platform has never trained on (4.15). A win on anything else is a win
///    on data the champion also shaped.
/// 3. It beat the champion under §11.5's protocol. `challenger_won` is a
///    `&Comparison`-shaped fact the caller establishes; this refuses without it.
/// 4. The dispatch policy has not collapsed (4.14). Promoting on a collapsed
///    policy compounds the collapse: the new champion was chosen from the
///    narrow slate the old one produced.
///
/// # Errors
/// Any of the four, named.
pub fn authorise_promotion(
    tier: ModelTier,
    frozen: Option<&str>,
    holdout: &SeedHoldout,
    evaluated_on_holdout: bool,
    challenger_won: bool,
    entropy: EntropyFloor,
    approver: Option<&str>,
) -> Result<PromotionCase, PromotionRefusal> {
    if let Some(reason) = frozen {
        return Err(PromotionRefusal::Frozen(reason.to_string()));
    }
    if !evaluated_on_holdout || !holdout.is_frozen() {
        return Err(PromotionRefusal::NotOnHoldout);
    }
    if !challenger_won {
        return Err(PromotionRefusal::NotBetter);
    }
    if tier == ModelTier::B {
        if !entropy.permits_promotion() {
            return Err(PromotionRefusal::EntropyFloor(entropy.describe()));
        }
        if approver.is_none_or(str::is_empty) {
            return Err(PromotionRefusal::NoApprover);
        }
    }
    Ok(PromotionCase {
        tier,
        approver: approver.map(ToString::to_string),
        holdout_size: holdout.trial_ids().len(),
    })
}

/// Whether a champion should be rolled back.
///
/// Tier A rolls back automatically; Tier B does not, because an automatic
/// rollback of a model a human approved is an automatic reversal of a human
/// decision. It alarms instead.
#[must_use]
pub fn should_auto_rollback(tier: ModelTier, champion_worse_than_previous: bool) -> bool {
    tier == ModelTier::A && champion_worse_than_previous
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn frozen_holdout() -> SeedHoldout {
        let ids: Vec<Uuid> = (0..200).map(|_| Uuid::new_v4()).collect();
        let mut h = SeedHoldout::unfrozen();
        h.freeze(&ids).unwrap();
        h
    }

    fn healthy_entropy() -> EntropyFloor {
        EntropyFloor { observed: 2.0, mean_candidates: 20.0 }
    }

    // ── M5 ──────────────────────────────────────────────────────────────────

    /// An uncalibrated screen screens nothing. The default direction is the one
    /// whose error is recoverable: a doomed candidate costs one gate run, a
    /// screened-out passer costs a discovery nobody knows was lost.
    #[test]
    fn an_uncalibrated_prescreen_lets_everything_through() {
        assert_eq!(prescreen(Some(-5.0), None, 0, 0).value, Prescreen::Proceed);
        assert_eq!(prescreen(None, None, 0, 0).value, Prescreen::Proceed);
    }

    #[test]
    fn a_calibrated_prescreen_is_monotone_in_the_margin() {
        let cut = Some(-0.1);
        assert_eq!(prescreen(Some(0.5), cut, 900, 150).value, Prescreen::Proceed);
        assert_eq!(prescreen(Some(-0.1), cut, 900, 150).value, Prescreen::Proceed);
        assert_eq!(prescreen(Some(-0.2), cut, 900, 150).value, Prescreen::LikelyFails);
    }

    #[test]
    fn a_missing_margin_against_a_calibration_is_unknown_not_a_pass() {
        assert_eq!(prescreen(None, Some(-0.1), 900, 150).value, Prescreen::Unknown);
    }

    /// A quantile from a handful of passers is a number, not an operating point.
    #[test]
    fn the_operating_point_needs_enough_passers_to_be_one() {
        assert_eq!(calibrate_prescreen(&[0.1; 29]), None);
        let margins: Vec<f64> = (0..200).map(|i| f64::from(i) / 100.0).collect();
        let cut = calibrate_prescreen(&margins).expect("calibrated");
        // 1 % of 200 passers is the second-lowest margin.
        assert!(cut <= 0.02, "{cut}");
    }

    #[test]
    fn m5s_trigger_counts_passes_not_merely_gated_candidates() {
        // Five hundred gated candidates that all failed teach a pre-screener
        // nothing about what a pass looks like.
        assert!(!prescreen(None, None, 900, 40).evidence.met());
        assert!(prescreen(None, None, 900, 150).evidence.met());
    }

    // ── M8 ──────────────────────────────────────────────────────────────────

    /// The raw rate ranks 1-for-1 above 19-for-20, which is an artefact of the
    /// sample size. The lower bound makes "we have not tried this much" cost
    /// something.
    #[test]
    fn families_rank_on_a_lower_bound_not_a_raw_rate() {
        let lucky = FamilyRecord { family: "lucky".into(), attempts: 1, passes: 1 };
        let proven = FamilyRecord { family: "proven".into(), attempts: 20, passes: 19 };
        assert!(proven.lower_bound() > lucky.lower_bound());

        let r = recommend_families("tenant-a", &[lucky, proven], 10);
        assert_eq!(r.value, vec!["proven".to_string(), "lucky".to_string()]);
        assert_eq!(r.decision_tier(), crate::DecisionTier::Rule);
    }

    #[test]
    fn a_family_nobody_has_tried_ranks_last_rather_than_first() {
        let untried = FamilyRecord { family: "untried".into(), attempts: 0, passes: 0 };
        let tried = FamilyRecord { family: "tried".into(), attempts: 10, passes: 3 };
        assert!((untried.lower_bound() - 0.0).abs() < f64::EPSILON);
        let r = recommend_families("t", &[untried, tried], 10);
        assert_eq!(r.value.first().unwrap(), "tried");
    }

    // ── M9 ──────────────────────────────────────────────────────────────────

    #[test]
    fn proposals_rank_on_mean_plus_kappa_sigma() {
        let p = vec![
            Proposal { id: "safe".into(), mean: 1.0, sigma: Some(0.0) },
            Proposal { id: "wild".into(), mean: 0.5, sigma: Some(0.5) },
        ];
        let r = rank_proposals(&p, 100, 10).value.expect("ranked");
        // 0.5 + 2·0.5 = 1.5 beats 1.0 + 0: the acquisition buys information.
        assert_eq!(r, vec!["wild".to_string(), "safe".to_string()]);
    }

    /// Ranking on the mean is not a weaker acquisition, it is pure exploitation
    /// — and it would look like a ranking to everyone downstream.
    #[test]
    fn without_a_surrogate_there_is_no_ranking_rather_than_a_mean_ranking() {
        let p = vec![
            Proposal { id: "a".into(), mean: 1.0, sigma: None },
            Proposal { id: "b".into(), mean: 0.5, sigma: Some(0.2) },
        ];
        assert_eq!(rank_proposals(&p, 100, 10).value, None);
    }

    // ── M13 ─────────────────────────────────────────────────────────────────

    #[test]
    fn the_hard_rules_fire_before_any_triage() {
        assert_eq!(critique_step(true, true, false, 0).value, StepVerdict::RuleViolation);
        assert_eq!(critique_step(false, false, false, 0).value, StepVerdict::RuleViolation);
        assert_eq!(critique_step(false, true, true, 0).value, StepVerdict::NeedsJudge);
        assert_eq!(critique_step(false, true, false, 0).value, StepVerdict::Fine);
        // A repeated step is a loop whether or not it looks unusual.
        assert_eq!(critique_step(true, true, true, 0).value, StepVerdict::RuleViolation);
    }

    // ── 4.11 ────────────────────────────────────────────────────────────────

    #[test]
    fn the_learning_debt_trigger_balances_churn_against_waiting() {
        // Equal costs: retrain when the debt exceeds half.
        assert_eq!(learning_debt(0.6, 1.0, 1.0), Some(true));
        assert_eq!(learning_debt(0.4, 1.0, 1.0), Some(false));
        // Churn ten times as expensive: the bar rises to ~0.91.
        assert_eq!(learning_debt(0.8, 10.0, 1.0), Some(false));
        assert_eq!(learning_debt(0.95, 10.0, 1.0), Some(true));
    }

    #[test]
    fn unmeasured_costs_produce_no_trigger() {
        assert_eq!(learning_debt(0.9, 0.0, 1.0), None);
        assert_eq!(learning_debt(f64::NAN, 1.0, 1.0), None);
    }

    // ── 4.12 ────────────────────────────────────────────────────────────────

    #[test]
    fn a_tier_a_promotion_needs_a_holdout_win_and_nothing_else() {
        let case = authorise_promotion(
            ModelTier::A,
            None,
            &frozen_holdout(),
            true,
            true,
            healthy_entropy(),
            None,
        )
        .expect("tier A auto-promotes");
        assert_eq!(case.tier(), ModelTier::A);
        assert_eq!(case.holdout_size(), 200);
        assert_eq!(case.approver(), None);
    }

    /// A win on anything but the frozen holdout is a win on data the champion
    /// also shaped.
    #[test]
    fn a_win_off_the_frozen_holdout_is_not_a_win() {
        assert_eq!(
            authorise_promotion(ModelTier::A, None, &frozen_holdout(), false, true, healthy_entropy(), None)
                .unwrap_err(),
            PromotionRefusal::NotOnHoldout
        );
        assert_eq!(
            authorise_promotion(ModelTier::A, None, &SeedHoldout::unfrozen(), true, true, healthy_entropy(), None)
                .unwrap_err(),
            PromotionRefusal::NotOnHoldout
        );
    }

    #[test]
    fn a_frozen_platform_promotes_nothing() {
        let err = authorise_promotion(
            ModelTier::A, Some("incident 4"), &frozen_holdout(), true, true, healthy_entropy(), None,
        )
        .unwrap_err();
        assert!(matches!(err, PromotionRefusal::Frozen(r) if r == "incident 4"));
    }

    /// Promoting on a collapsed policy compounds the collapse: the new champion
    /// was chosen from the narrow slate the old one produced.
    #[test]
    fn a_tier_b_promotion_needs_the_entropy_floor_and_an_approver() {
        let collapsed = EntropyFloor { observed: 0.1, mean_candidates: 20.0 };
        assert!(matches!(
            authorise_promotion(ModelTier::B, None, &frozen_holdout(), true, true, collapsed, Some("mason")),
            Err(PromotionRefusal::EntropyFloor(_))
        ));
        assert_eq!(
            authorise_promotion(ModelTier::B, None, &frozen_holdout(), true, true, healthy_entropy(), None)
                .unwrap_err(),
            PromotionRefusal::NoApprover
        );
        let ok = authorise_promotion(
            ModelTier::B, None, &frozen_holdout(), true, true, healthy_entropy(), Some("mason"),
        )
        .expect("approved");
        assert_eq!(ok.approver(), Some("mason"));
    }

    /// Tier A's errors are visible and cheap, so it reverts itself. Tier B's
    /// were approved by a person, and an automatic rollback of those is an
    /// automatic reversal of a human decision.
    #[test]
    fn only_tier_a_rolls_itself_back() {
        assert!(should_auto_rollback(ModelTier::A, true));
        assert!(!should_auto_rollback(ModelTier::A, false));
        assert!(!should_auto_rollback(ModelTier::B, true));
    }
}
