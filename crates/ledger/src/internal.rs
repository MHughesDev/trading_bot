//! The cold-start ladder and the guards around training an internal model
//! (SPEC §13.1, checklist 4.1/4.13/4.14/4.15/4.16, ADR-P4-01…03).
//!
//! §13.1's ladder is `analytic/rule → shrunk → learned`, and ADR-P4-01 is the
//! observation that at this platform's scale the ladder **is** the product for
//! the foreseeable future: the reference's learned-tier thresholds (M4 ≈ 1 500
//! runs over ≥ 30 tasks, M5 ≥ 500 gated candidates with ≥ 100 passes, M8 ≥ 100
//! tasks) will not be met for a long time. So the rule tiers are built first and
//! each learned tier sits behind an explicit, written-down ledger-size trigger.
//!
//! ## Why the tier is a returned value and not a log line
//!
//! [`Answer`] carries the tier that produced it. Not "we also log the tier" —
//! the answer *is* the pair, so a caller cannot hold one without the other, and
//! the eventual rule-versus-learned comparison is free rather than an
//! archaeology project over old logs. §4.2's `decision_tier` column then records
//! a fact the code computed rather than a label somebody attached.
//!
//! ## The four guards
//!
//! A platform that learns from its own decisions can spiral: the policy's
//! outputs become the next policy's training data, the distribution narrows, and
//! every internal metric improves while the thing gets worse. Four guards, each
//! a refusal rather than a warning:
//!
//! 1. **Ranges, not windows** ([`TrainingRange`]). A training set is
//!    `[0, seq_max]` over the ledger — it accumulates and never forgets. A
//!    sliding window is how a model ends up trained only on what the current
//!    policy chose to try.
//! 2. **Ground truth required** ([`TrainingSet::build`]). Every set must contain
//!    at least one outcome observed in paper or live. A model trained purely on
//!    backtests has learned what the simulator does.
//! 3. **The seed holdout** ([`SeedHoldout`]). The first `K` trials per tenant,
//!    frozen once, never re-drawn — a prefix is reproducible from the ledger
//!    alone, where a random sample is a number somebody has to keep (ADR-P4-02).
//! 4. **The entropy floor** ([`EntropyFloor`]). Dispatch-propensity entropy over
//!    the last window must stay at or above `0.5·ln k`. Below it the policy has
//!    stopped exploring and a Tier-B promotion is blocked (ADR-P4-03).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{DecisionTier, LedgerError};

/// The size of the frozen seed holdout: the first `K` trials per tenant.
///
/// A **prefix**, not a random sample. A prefix is reproducible from the ledger
/// by anyone, cannot be re-drawn if the first draw was inconvenient, and is
/// exactly the data the platform had before it started learning from itself
/// (ADR-P4-02).
pub const SEED_HOLDOUT_SIZE: usize = 200;

/// The entropy floor's coefficient: `0.5 · ln k` (ADR-P4-03).
///
/// Relative to `ln k` rather than absolute, because an absolute floor is
/// meaningless for a small slate: 0.7 nats is near-maximal entropy over two
/// candidates and near-total collapse over fifty.
pub const ENTROPY_FLOOR_FRACTION: f64 = 0.5;

// ───────────────────────────────────────────────────────────────────────────────
// the ladder
// ───────────────────────────────────────────────────────────────────────────────

/// A rung of §13.1's ladder.
///
/// `Analytic` sits below [`DecisionTier::Rule`] and is not one of the three the
/// ledger records: a closed-form estimate (rows × features × steps over a
/// hardware class's throughput) is a rule with no table behind it, and it
/// records as `rule`. The distinction is kept here because it is the difference
/// between "we have a formula" and "we have a table somebody tuned", and the
/// second is the one that can rot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    Analytic,
    Rule,
    Shrunk,
    Learned,
}

impl Rung {
    pub const ALL: [Self; 4] = [Self::Analytic, Self::Rule, Self::Shrunk, Self::Learned];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Analytic => "analytic",
            Self::Rule => "rule",
            Self::Shrunk => "shrunk",
            Self::Learned => "learned",
        }
    }

    /// How §4.2 records this rung. Analytic and rule are both `rule` on the
    /// wire: the ledger's question is "did a fitted model decide this", and the
    /// answer for both is no.
    #[must_use]
    pub fn decision_tier(self) -> DecisionTier {
        match self {
            Self::Analytic | Self::Rule => DecisionTier::Rule,
            Self::Shrunk => DecisionTier::Shrunk,
            Self::Learned => DecisionTier::Learned,
        }
    }
}

/// An answer and the rung that produced it.
///
/// Inseparable on purpose. A number whose provenance is optional is a number
/// that will eventually be reported without it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer<T> {
    pub value: T,
    pub rung: Rung,
    /// What the rung was chosen on: the ledger evidence counted at decision
    /// time, and the trigger it was compared against.
    pub evidence: LadderEvidence,
}

impl<T> Answer<T> {
    #[must_use]
    pub fn decision_tier(&self) -> DecisionTier {
        self.rung.decision_tier()
    }
}

/// What the ladder counted, and what it needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LadderEvidence {
    pub observations: i64,
    pub required: i64,
    /// Secondary condition — distinct tasks, passes, groups — as the reference
    /// states each trigger. `None` when the trigger has only a count.
    pub groups: Option<i64>,
    pub groups_required: Option<i64>,
}

impl LadderEvidence {
    #[must_use]
    pub fn met(&self) -> bool {
        let count_ok = self.observations >= self.required;
        let groups_ok = match (self.groups, self.groups_required) {
            (Some(g), Some(need)) => g >= need,
            (None, Some(_)) => false, // a trigger with an uncounted condition is unmet
            _ => true,
        };
        count_ok && groups_ok
    }

    /// A one-line reason, for the decision row and for a human reading why the
    /// platform is still answering from a rule.
    #[must_use]
    pub fn describe(&self) -> String {
        match (self.groups, self.groups_required) {
            (Some(g), Some(need)) => format!(
                "{} of {} observations, {g} of {need} groups",
                self.observations, self.required
            ),
            _ => format!("{} of {} observations", self.observations, self.required),
        }
    }
}

/// A learned tier's ledger-size trigger, written down.
///
/// The thresholds are the reference's, stated per model rather than as one
/// global number, because they are not the same question: a cost model needs
/// runs per hardware class, a gate pre-screener needs *passes* and not merely
/// gated candidates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trigger {
    pub model: &'static str,
    pub observations: i64,
    pub groups: Option<i64>,
    /// What a "group" counts here — hardware classes, tasks, failure classes.
    pub group_label: &'static str,
}

/// §13.1's triggers, from the reference. One row per model that has a learned
/// tier at all.
pub const TRIGGERS: &[Trigger] = &[
    Trigger { model: "M1", observations: 200, groups: None, group_label: "flow family × hw class" },
    Trigger { model: "M2", observations: 300, groups: Some(50), group_label: "per failure class" },
    Trigger { model: "M3", observations: 1_000, groups: None, group_label: "curves" },
    Trigger { model: "M4", observations: 1_500, groups: Some(30), group_label: "tasks" },
    Trigger { model: "M5", observations: 500, groups: Some(100), group_label: "passes" },
    Trigger { model: "M8", observations: 100, groups: None, group_label: "tasks" },
    Trigger { model: "M9", observations: 3_000, groups: Some(100), group_label: "groups" },
    Trigger { model: "M10", observations: 1_000, groups: None, group_label: "planted cases" },
    Trigger { model: "M11", observations: 300, groups: None, group_label: "curves" },
    Trigger { model: "M13", observations: 3_000, groups: None, group_label: "labelled steps" },
];

/// The trigger for a model, if it has one.
#[must_use]
pub fn trigger_for(model: &str) -> Option<Trigger> {
    TRIGGERS.iter().copied().find(|t| t.model == model)
}

/// Decide which rung answers, given what the ledger holds.
///
/// `learned_available` is whether a trained, promoted model actually exists —
/// separate from whether the trigger is met, because a met trigger means the
/// data is there, not that anybody has trained on it yet. Reporting `learned`
/// when nothing is fitted is exactly the failure ADR-P5-01 names.
#[must_use]
pub fn choose_rung(
    model: &str,
    observations: i64,
    groups: Option<i64>,
    learned_available: bool,
    shrunk_available: bool,
) -> (Rung, LadderEvidence) {
    let trigger = trigger_for(model);
    let evidence = LadderEvidence {
        observations,
        required: trigger.map_or(i64::MAX, |t| t.observations),
        groups,
        groups_required: trigger.and_then(|t| t.groups),
    };
    let rung = if learned_available && evidence.met() {
        Rung::Learned
    } else if shrunk_available {
        Rung::Shrunk
    } else {
        Rung::Rule
    };
    (rung, evidence)
}

// ───────────────────────────────────────────────────────────────────────────────
// 4.13 — collapse guards
// ───────────────────────────────────────────────────────────────────────────────

/// A training set's extent over the ledger: `[0, seq_max]`.
///
/// There is no `from` field, and that is the guard. A window is how a model ends
/// up trained only on what the current policy chose to try recently — the
/// distribution narrows, every internal metric improves, and the thing gets
/// worse. Accumulating is slower and does not collapse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrainingRange {
    seq_max: i64,
}

impl TrainingRange {
    /// # Errors
    /// A negative sequence bound.
    pub fn up_to(seq_max: i64) -> Result<Self, LedgerError> {
        if seq_max < 0 {
            return Err(LedgerError::Invalid(format!(
                "a training range ends at a ledger sequence, and {seq_max} is not one"
            )));
        }
        Ok(Self { seq_max })
    }

    #[must_use]
    pub fn seq_max(self) -> i64 {
        self.seq_max
    }

    /// Always zero. Present as a method rather than a field so that "start the
    /// range later" is not something a caller can express.
    #[must_use]
    pub fn seq_min(self) -> i64 {
        0
    }
}

/// Where an outcome was observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeSource {
    Backtest,
    Paper,
    Live,
}

impl OutcomeSource {
    /// Whether this source is contact with the world rather than with the
    /// simulator.
    #[must_use]
    pub fn is_ground_truth(self) -> bool {
        matches!(self, Self::Paper | Self::Live)
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Backtest => "backtest",
            Self::Paper => "paper",
            Self::Live => "live",
        }
    }
}

/// Why a training set was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TrainingRefusal {
    #[error("the training set has no outcome observed in paper or live; a model trained only on backtests has learned what the simulator does")]
    NoGroundTruth,
    #[error("internal model training is frozen: {0}")]
    Frozen(String),
    #[error("the training set is empty")]
    Empty,
    #[error("the seed holdout is already frozen with {existing} trials; it is drawn once and never re-drawn")]
    HoldoutAlreadyFrozen { existing: usize },
    #[error("{overlap} of the training trials are in the frozen seed holdout")]
    HoldoutOverlap { overlap: usize },
}

/// A training set that passed the collapse guards.
///
/// Sealed: [`TrainingSet::build`] is the only constructor, and it cannot be
/// called without the range, the sources and the holdout to check against.
#[derive(Clone, Debug, PartialEq)]
pub struct TrainingSet {
    range: TrainingRange,
    trial_ids: Vec<Uuid>,
    ground_truth: usize,
}

impl TrainingSet {
    /// Build a training set, or say why not.
    ///
    /// # Errors
    /// An empty set, no ground-truth outcome, a frozen platform, or any overlap
    /// with the seed holdout.
    pub fn build(
        range: TrainingRange,
        trials: &[(Uuid, OutcomeSource)],
        holdout: &SeedHoldout,
        frozen: Option<&str>,
    ) -> Result<Self, TrainingRefusal> {
        if let Some(reason) = frozen {
            return Err(TrainingRefusal::Frozen(reason.to_string()));
        }
        if trials.is_empty() {
            return Err(TrainingRefusal::Empty);
        }
        let overlap = trials.iter().filter(|(id, _)| holdout.contains(*id)).count();
        if overlap > 0 {
            return Err(TrainingRefusal::HoldoutOverlap { overlap });
        }
        let ground_truth = trials.iter().filter(|(_, s)| s.is_ground_truth()).count();
        if ground_truth == 0 {
            return Err(TrainingRefusal::NoGroundTruth);
        }
        Ok(Self {
            range,
            trial_ids: trials.iter().map(|(id, _)| *id).collect(),
            ground_truth,
        })
    }

    #[must_use]
    pub fn range(&self) -> TrainingRange {
        self.range
    }
    #[must_use]
    pub fn trial_ids(&self) -> &[Uuid] {
        &self.trial_ids
    }
    #[must_use]
    pub fn ground_truth_count(&self) -> usize {
        self.ground_truth
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.trial_ids.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trial_ids.is_empty()
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// 4.15 — the seed holdout
// ───────────────────────────────────────────────────────────────────────────────

/// The first `K` trials of a tenant, frozen once (ADR-P4-02).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedHoldout {
    trial_ids: Vec<Uuid>,
}

impl SeedHoldout {
    /// An empty holdout — nothing has been frozen yet.
    #[must_use]
    pub fn unfrozen() -> Self {
        Self::default()
    }

    /// Freeze the prefix.
    ///
    /// `ordered` must be the tenant's trials in registration order; the holdout
    /// is the first [`SEED_HOLDOUT_SIZE`] of them. Freezing twice is refused
    /// rather than ignored: a second freeze with different data is the shape of
    /// a re-draw, and a re-draw is the thing a *frozen* holdout exists to
    /// prevent.
    ///
    /// # Errors
    /// Already frozen.
    pub fn freeze(&mut self, ordered: &[Uuid]) -> Result<usize, TrainingRefusal> {
        if !self.trial_ids.is_empty() {
            return Err(TrainingRefusal::HoldoutAlreadyFrozen { existing: self.trial_ids.len() });
        }
        self.trial_ids = ordered.iter().take(SEED_HOLDOUT_SIZE).copied().collect();
        Ok(self.trial_ids.len())
    }

    /// Reconstruct a frozen holdout from storage.
    #[must_use]
    pub fn from_stored(trial_ids: Vec<Uuid>) -> Self {
        Self { trial_ids }
    }

    #[must_use]
    pub fn contains(&self, trial_id: Uuid) -> bool {
        self.trial_ids.contains(&trial_id)
    }

    #[must_use]
    pub fn trial_ids(&self) -> &[Uuid] {
        &self.trial_ids
    }

    #[must_use]
    pub fn is_frozen(&self) -> bool {
        !self.trial_ids.is_empty()
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// 4.14 — the entropy floor
// ───────────────────────────────────────────────────────────────────────────────

/// The dispatch-propensity entropy SLO (ADR-P4-03).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntropyFloor {
    /// Measured entropy over the last window, in nats.
    pub observed: f64,
    /// Mean candidate-set size over that window.
    pub mean_candidates: f64,
}

impl EntropyFloor {
    /// `0.5 · ln k`. `None` when the slate was degenerate — one candidate has no
    /// entropy to measure, and reporting a floor of zero would make a collapsed
    /// policy look compliant.
    #[must_use]
    pub fn floor(&self) -> Option<f64> {
        (self.mean_candidates > 1.0).then(|| ENTROPY_FLOOR_FRACTION * self.mean_candidates.ln())
    }

    /// Whether the policy is still exploring enough for a Tier-B promotion.
    ///
    /// `None` — unmeasurable — is **not** a pass. A platform that cannot tell
    /// whether its policy has collapsed does not get to promote on the strength
    /// of not knowing.
    #[must_use]
    pub fn permits_promotion(&self) -> bool {
        self.floor().is_some_and(|f| self.observed >= f)
    }

    /// What to say about it.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.floor() {
            Some(f) => format!(
                "propensity entropy {:.3} nats against a floor of {f:.3} (0.5·ln {:.1})",
                self.observed, self.mean_candidates
            ),
            None => format!(
                "propensity entropy is not measurable over a mean slate of {:.1} candidates",
                self.mean_candidates
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holdout() -> SeedHoldout {
        SeedHoldout::unfrozen()
    }

    #[test]
    fn the_analytic_rung_records_as_a_rule() {
        assert_eq!(Rung::Analytic.decision_tier(), DecisionTier::Rule);
        assert_eq!(Rung::Rule.decision_tier(), DecisionTier::Rule);
        assert_eq!(Rung::Shrunk.decision_tier(), DecisionTier::Shrunk);
        assert_eq!(Rung::Learned.decision_tier(), DecisionTier::Learned);
    }

    /// A met trigger means the *data* exists. It does not mean anybody trained
    /// on it, and answering `learned` when nothing is fitted is the exact
    /// failure ADR-P5-01 names.
    #[test]
    fn a_met_trigger_is_not_a_trained_model() {
        let (rung, evidence) = choose_rung("M8", 500, None, false, false);
        assert!(evidence.met());
        assert_eq!(rung, Rung::Rule);

        let (rung, _) = choose_rung("M8", 500, None, true, false);
        assert_eq!(rung, Rung::Learned);
    }

    #[test]
    fn an_unmet_trigger_keeps_the_rule_tier_however_good_the_model_is() {
        let (rung, evidence) = choose_rung("M4", 1_400, Some(40), true, false);
        assert!(!evidence.met(), "{}", evidence.describe());
        assert_eq!(rung, Rung::Rule);
        assert!(evidence.describe().contains("1400 of 1500"), "{}", evidence.describe());
    }

    /// M5's second condition is *passes*, not gated candidates: five hundred
    /// gated candidates that all failed teach a pre-screener nothing about what
    /// a pass looks like.
    #[test]
    fn a_trigger_with_an_uncounted_secondary_condition_is_unmet() {
        let (rung, _) = choose_rung("M5", 900, None, true, true);
        assert_eq!(rung, Rung::Shrunk, "an uncounted condition must not read as satisfied");
        let (rung, _) = choose_rung("M5", 900, Some(120), true, true);
        assert_eq!(rung, Rung::Learned);
    }

    #[test]
    fn a_model_with_no_learned_tier_never_climbs() {
        assert!(trigger_for("M12").is_none());
        let (rung, evidence) = choose_rung("M12", 1_000_000, Some(1_000), true, false);
        assert_eq!(rung, Rung::Rule);
        assert!(!evidence.met());
    }

    /// A training set is a range from the beginning of the ledger. "Start later"
    /// is not an expressible request.
    #[test]
    fn a_training_range_always_starts_at_zero() {
        let r = TrainingRange::up_to(5_000).unwrap();
        assert_eq!(r.seq_min(), 0);
        assert_eq!(r.seq_max(), 5_000);
        assert!(TrainingRange::up_to(-1).is_err());
    }

    #[test]
    fn a_set_with_no_paper_or_live_outcome_is_refused() {
        let range = TrainingRange::up_to(10).unwrap();
        let only_backtests: Vec<(Uuid, OutcomeSource)> =
            (0..50).map(|_| (Uuid::new_v4(), OutcomeSource::Backtest)).collect();
        assert_eq!(
            TrainingSet::build(range, &only_backtests, &holdout(), None),
            Err(TrainingRefusal::NoGroundTruth)
        );

        let mut with_paper = only_backtests.clone();
        with_paper.push((Uuid::new_v4(), OutcomeSource::Paper));
        let set = TrainingSet::build(range, &with_paper, &holdout(), None).expect("built");
        assert_eq!(set.ground_truth_count(), 1);
        assert_eq!(set.len(), 51);
    }

    #[test]
    fn a_frozen_platform_trains_nothing() {
        let range = TrainingRange::up_to(10).unwrap();
        let trials = vec![(Uuid::new_v4(), OutcomeSource::Live)];
        let err = TrainingSet::build(range, &trials, &holdout(), Some("incident 4")).unwrap_err();
        assert!(matches!(err, TrainingRefusal::Frozen(r) if r == "incident 4"));
    }

    #[test]
    fn the_seed_holdout_is_a_prefix_frozen_once() {
        let all: Vec<Uuid> = (0..500).map(|_| Uuid::new_v4()).collect();
        let mut h = SeedHoldout::unfrozen();
        assert!(!h.is_frozen());
        assert_eq!(h.freeze(&all).unwrap(), SEED_HOLDOUT_SIZE);
        assert!(h.is_frozen());
        assert_eq!(h.trial_ids(), &all[..SEED_HOLDOUT_SIZE]);

        // A second freeze is a re-draw, which is what freezing prevents.
        assert!(matches!(
            h.freeze(&all).unwrap_err(),
            TrainingRefusal::HoldoutAlreadyFrozen { existing: SEED_HOLDOUT_SIZE }
        ));
    }

    /// A tenant with fewer than K trials freezes what it has, rather than
    /// waiting and then freezing a prefix of a ledger that has since learned
    /// from itself.
    #[test]
    fn a_short_ledger_freezes_what_it_has() {
        let all: Vec<Uuid> = (0..12).map(|_| Uuid::new_v4()).collect();
        let mut h = SeedHoldout::unfrozen();
        assert_eq!(h.freeze(&all).unwrap(), 12);
    }

    #[test]
    fn training_on_the_holdout_is_refused() {
        let all: Vec<Uuid> = (0..300).map(|_| Uuid::new_v4()).collect();
        let mut h = SeedHoldout::unfrozen();
        h.freeze(&all).unwrap();

        let range = TrainingRange::up_to(10).unwrap();
        let mut trials: Vec<(Uuid, OutcomeSource)> =
            all[250..].iter().map(|id| (*id, OutcomeSource::Live)).collect();
        TrainingSet::build(range, &trials, &h, None).expect("the tail is not the holdout");

        trials.push((all[3], OutcomeSource::Backtest));
        assert_eq!(
            TrainingSet::build(range, &trials, &h, None),
            Err(TrainingRefusal::HoldoutOverlap { overlap: 1 })
        );
    }

    #[test]
    fn the_entropy_floor_scales_with_the_slate() {
        let wide = EntropyFloor { observed: 1.5, mean_candidates: 20.0 };
        assert!((wide.floor().unwrap() - 0.5 * 20.0_f64.ln()).abs() < 1e-12);
        assert!(wide.permits_promotion(), "{}", wide.describe());

        // The same entropy over a small slate is near-maximal; over a large one
        // it is collapse. An absolute floor could not tell them apart.
        let collapsed = EntropyFloor { observed: 0.2, mean_candidates: 20.0 };
        assert!(!collapsed.permits_promotion(), "{}", collapsed.describe());
        let small = EntropyFloor { observed: 0.6, mean_candidates: 2.0 };
        assert!(small.permits_promotion(), "{}", small.describe());
    }

    /// Not being able to measure the entropy is not a pass.
    #[test]
    fn an_unmeasurable_floor_blocks_promotion() {
        let degenerate = EntropyFloor { observed: 0.0, mean_candidates: 1.0 };
        assert_eq!(degenerate.floor(), None);
        assert!(!degenerate.permits_promotion());
        assert!(degenerate.describe().contains("not measurable"), "{}", degenerate.describe());
    }

    #[test]
    fn every_trigger_names_what_its_groups_count() {
        for t in TRIGGERS {
            assert!(t.observations > 0, "{}", t.model);
            assert!(!t.group_label.is_empty(), "{} does not say what a group is", t.model);
        }
        // M5's is passes, not candidates — the distinction the reference calls
        // load-bearing.
        assert_eq!(trigger_for("M5").unwrap().group_label, "passes");
    }
}
