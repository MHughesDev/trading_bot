//! `ContextPolicy` — the versioned object everything in Part II reads
//! (AGENT-005 §22; Set S task S-9.11).
//!
//! # Why this is an object and not a scatter of constants
//!
//! Selection weights, kind priors, per-kind half-lives, the degradation order,
//! the manifest budget, the step-value bounds and the compaction trigger are all
//! *tuning*. Left as constants they are untunable, untestable and — worse —
//! unrecorded: two runs that behaved differently would give no way to tell
//! whether the difference was the task or a value somebody changed.
//!
//! As one object with a version, recorded on every run and in every trajectory,
//! the whole of Part II becomes an offline experiment. The loop is sans-IO, so
//! two policies can be A/B'd over the **same recorded tapes at no model cost**.
//! That is the payoff of the sans-IO design and it is what turns context tuning
//! from a live experiment into an arithmetic one.
//!
//! # `v1` is the control, and it is Part I exactly
//!
//! [`ContextPolicy::v1`] is not "Part II with cautious numbers". It is a policy
//! whose selection mode is [`SelectionMode::Chronological`] and whose degradation
//! mode is [`DegradationMode::PartOne`] — that is, no selection and Part I's own
//! assembler. Under `v1` the assembled prompt is byte-identical to what this
//! platform sends today, which is the only definition of "control" that can be
//! *asserted* rather than argued.
//!
//! `v2` ships only by beating `v1` on **every** tier under AGENT-009 §4's
//! interval rules (S-9.12). A policy that helps the frontier and hurts the 7B
//! does not ship — AGENT-009 P4 — and on this phase that is the likely failure,
//! because relevance scoring has the most room to go wrong exactly where the
//! window is smallest.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::store::RecordKind;
use crate::state::GoalKind;

/// How the prompt's content is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    /// Part I: everything, in the order it arrived, trimmed from the front when
    /// it does not fit. The control.
    Chronological,
    /// Part II: scored, diversified, requirement-satisfying retrieval.
    Retrieval,
}

/// How a record that does not fit is made to fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationMode {
    /// Part I: `truncate_middle` on the assembled block. The control.
    PartOne,
    /// Part II: the declared ladder, and drop rather than cut at the bottom.
    Semantic,
}

/// How raw signals are made comparable before they are summed (§24.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Normalisation {
    /// Sum the raw signals. Wrong, and kept nameable so the ablation can say so:
    /// recency is `[0,1]` and citation count is unbounded, so the unbounded
    /// signal wins every tie and the weights become decoration.
    Raw,
    /// Percentile rank against the records currently in the store.
    PercentileRank,
}

/// The §15 signal weights.
///
/// Declared here and **fitted** in S-9.12 — the values below are the starting
/// point the spec's table implies, not a measurement, and `ADR-S9-03` records
/// that distinction so nobody later reads them as evidence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Weights {
    /// Subject match with the active step.
    pub subject: f32,
    /// Id overlap with the step's arguments or the hypothesis.
    pub id_overlap: f32,
    /// Kind prior for this goal kind.
    pub kind_prior: f32,
    /// Recency, decayed per kind.
    pub recency: f32,
    /// Times the model has already cited this record in the run.
    pub citations: f32,
    /// Records the active step's own tool produced before.
    pub same_tool: f32,
    /// A correction for the active or upcoming tool.
    pub correction: f32,
    /// A record the model has `expand`ed (§18's promotion).
    pub expansion: f32,
    /// Rendered on the previous call — the stickiness that stops cache thrash
    /// (§24.5).
    pub stickiness: f32,
    /// Subtracted for each earlier selected record sharing subject, kind,
    /// source or ids (§24.1).
    pub redundancy_penalty: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            subject: 1.0,
            id_overlap: 1.5,
            kind_prior: 0.8,
            recency: 0.6,
            citations: 0.7,
            same_tool: 0.5,
            correction: 1.2,
            expansion: 1.0,
            stickiness: 0.3,
            redundancy_penalty: 0.9,
        }
    }
}

/// The tuning object, versioned and recorded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextPolicy {
    /// Recorded on every run and in every trajectory. Two runs that behaved
    /// differently must be able to say whether the policy differed.
    pub version: String,
    pub selection: SelectionMode,
    pub degradation: DegradationMode,
    pub normalisation: Normalisation,
    pub weights: Weights,
    /// Per-`GoalKind` multipliers on each `RecordKind` — a lookup weights
    /// `ToolResult`; a research step weights `Finding` and `Correction`.
    pub kind_priors: BTreeMap<String, f32>,
    /// Steps after which a record of this kind has lost half its recency
    /// weight. One constant for every kind is wrong in one direction (§26.2):
    /// a `Correction` from step 2 is dead by step 30, and a `Finding` from
    /// step 2 may decide it.
    pub half_lives: BTreeMap<String, f32>,
    /// Share of the input budget the manifest may take before it groups harder.
    pub manifest_budget_share: f32,
    /// Bounds on the body-budget multiplier (§20).
    pub step_value_min: f32,
    pub step_value_max: f32,
    /// Fraction of the input budget at which selection engages at all.
    pub trigger: f32,
    /// See `degrade::DEGRADE_LOSS_FACTOR`.
    pub degrade_loss_factor: f32,
    /// Share of the body untrusted records may occupy (§27.2).
    pub untrusted_share: f32,
    /// Share of the body the harness's own furniture may occupy (§25.7).
    pub overhead_cap_share: f32,
    /// Admit and drop thresholds — two, not one, so a record hovering at the
    /// cut does not enter and leave on alternate calls (§24.5).
    pub admit_threshold: f32,
    pub drop_threshold: f32,
    /// Bodies beyond this count spill to the workspace; metadata stays
    /// resident so scoring and the manifest still see the whole store (§26.3).
    pub max_resident_records: usize,
}

/// Key for [`ContextPolicy::kind_priors`].
///
/// A `String` rather than a tuple key because this object is serialised into
/// every trajectory and JSON has no tuple keys — a map that cannot round-trip
/// through the trajectory is a policy that cannot be replayed, which defeats the
/// object.
#[must_use]
pub fn prior_key(goal: GoalKind, kind: RecordKind) -> String {
    format!("{}.{}", goal_str(goal), kind.as_str())
}

fn goal_str(g: GoalKind) -> &'static str {
    match g {
        GoalKind::Lookup => "lookup",
        GoalKind::Research => "research",
        GoalKind::Diagnose => "diagnose",
        GoalKind::Compare => "compare",
    }
}

/// Every `GoalKind`, so the priors table can be built exhaustively.
const GOAL_KINDS: &[GoalKind] = &[
    GoalKind::Lookup,
    GoalKind::Research,
    GoalKind::Diagnose,
    GoalKind::Compare,
];

impl ContextPolicy {
    /// Part I exactly: the control (§22).
    #[must_use]
    pub fn v1() -> Self {
        Self {
            version: "v1".into(),
            selection: SelectionMode::Chronological,
            degradation: DegradationMode::PartOne,
            normalisation: Normalisation::Raw,
            weights: Weights::default(),
            kind_priors: BTreeMap::new(),
            half_lives: BTreeMap::new(),
            manifest_budget_share: 0.0,
            step_value_min: 1.0,
            step_value_max: 1.0,
            trigger: super::COMPACTION_TRIGGER,
            degrade_loss_factor: super::degrade::DEGRADE_LOSS_FACTOR,
            untrusted_share: 1.0,
            overhead_cap_share: 1.0,
            admit_threshold: f32::NEG_INFINITY,
            drop_threshold: f32::NEG_INFINITY,
            max_resident_records: usize::MAX,
        }
    }

    /// Retrieval, with the weights the spec's table implies as a starting point.
    #[must_use]
    pub fn v2() -> Self {
        Self {
            version: "v2".into(),
            selection: SelectionMode::Retrieval,
            degradation: DegradationMode::Semantic,
            normalisation: Normalisation::PercentileRank,
            weights: Weights::default(),
            kind_priors: default_priors(),
            half_lives: default_half_lives(),
            manifest_budget_share: 0.08,
            step_value_min: 0.6,
            step_value_max: 1.4,
            trigger: super::COMPACTION_TRIGGER,
            degrade_loss_factor: super::degrade::DEGRADE_LOSS_FACTOR,
            untrusted_share: 0.25,
            overhead_cap_share: 0.12,
            admit_threshold: 0.35,
            drop_threshold: 0.20,
            max_resident_records: 400,
        }
    }

    /// The prior for this pairing; 1.0 when the policy declares none, so an
    /// unfitted policy is neutral rather than zero.
    #[must_use]
    pub fn kind_prior(&self, goal: GoalKind, kind: RecordKind) -> f32 {
        self.kind_priors
            .get(&prior_key(goal, kind))
            .copied()
            .unwrap_or(1.0)
    }

    /// Steps to half weight for this kind; a large number when undeclared, so a
    /// missing half-life means "does not decay" rather than "decays instantly".
    #[must_use]
    pub fn half_life(&self, kind: RecordKind) -> f32 {
        self.half_lives
            .get(kind.as_str())
            .copied()
            .unwrap_or(1_000.0)
    }

    /// Whether this policy engages Part II's machinery at all.
    #[must_use]
    pub fn is_retrieval(&self) -> bool {
        self.selection == SelectionMode::Retrieval
    }

    /// The two thresholds, ordered. A drop threshold above the admit threshold
    /// is not hysteresis, it is a record that can never be held.
    ///
    /// # Errors
    /// Returns the reason when the policy is not self-consistent.
    pub fn validate(&self) -> Result<(), String> {
        if self.version.trim().is_empty() {
            return Err("a policy with no version cannot be recorded on a run".into());
        }
        if self.is_retrieval() && self.drop_threshold > self.admit_threshold {
            return Err(format!(
                "drop_threshold {} above admit_threshold {}: a record could never be held",
                self.drop_threshold, self.admit_threshold
            ));
        }
        if !(0.0..=1.0).contains(&self.untrusted_share) {
            return Err("untrusted_share is a share of the body".into());
        }
        if !(0.0..=1.0).contains(&self.overhead_cap_share) {
            return Err("overhead_cap_share is a share of the body".into());
        }
        if self.step_value_min > self.step_value_max {
            return Err("step value bounds are inverted".into());
        }
        Ok(())
    }
}

fn default_priors() -> BTreeMap<String, f32> {
    let mut m = BTreeMap::new();
    for goal in GOAL_KINDS {
        for kind in RecordKind::ALL {
            let v = match (goal, kind) {
                // A lookup is answered by what a tool returned. Findings and
                // corrections from a research run are noise to it.
                (GoalKind::Lookup, RecordKind::ToolResult) => 1.4,
                (GoalKind::Lookup, RecordKind::Finding | RecordKind::Correction) => 0.7,
                // Research reasons over what it established and what it got
                // wrong.
                (GoalKind::Research, RecordKind::Finding) => 1.4,
                (GoalKind::Research, RecordKind::Correction) => 1.2,
                // Diagnosis is corrections and errors first: the run is looking
                // for what broke.
                (GoalKind::Diagnose, RecordKind::Correction) => 1.5,
                (GoalKind::Diagnose, RecordKind::ToolResult) => 1.1,
                // A comparison needs both sides' results side by side.
                (GoalKind::Compare, RecordKind::ToolResult) => 1.3,
                (GoalKind::Compare, RecordKind::Finding) => 1.2,
                _ => 1.0,
            };
            m.insert(prior_key(*goal, *kind), v);
        }
    }
    m
}

fn default_half_lives() -> BTreeMap<String, f32> {
    [
        // Dead by step 30, and actively misleading before that: a correction
        // for a tool the run has moved past is a warning about nothing.
        (RecordKind::Correction, 6.0),
        // A tool result's value falls with the step that asked for it.
        (RecordKind::ToolResult, 12.0),
        (RecordKind::Note, 8.0),
        (RecordKind::Decision, 20.0),
        (RecordKind::PlanChange, 20.0),
        // A finding from step 2 may decide step 30. This is the asymmetry §26.2
        // is about.
        (RecordKind::Finding, 200.0),
        // Memory arrived before the run started; it does not age within it.
        (RecordKind::Memory, 1_000.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.as_str().to_string(), v))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `v1` engages nothing. That is what makes it the control.
    #[test]
    fn v1_is_part_one_and_engages_no_part_two_machinery() {
        let p = ContextPolicy::v1();
        assert!(!p.is_retrieval());
        assert_eq!(p.degradation, DegradationMode::PartOne);
        assert!(
            (p.step_value_min - 1.0).abs() < f32::EPSILON
                && (p.step_value_max - 1.0).abs() < f32::EPSILON,
            "the control does not scale the body budget"
        );
        assert!(
            (p.untrusted_share - 1.0).abs() < f32::EPSILON,
            "and imposes no share Part I did not impose"
        );
        assert!(
            (p.trigger - super::super::COMPACTION_TRIGGER).abs() < f32::EPSILON,
            "on the same trigger Part I uses"
        );
    }

    /// Both shipped policies are self-consistent.
    #[test]
    fn the_shipped_policies_validate() {
        ContextPolicy::v1().validate().expect("v1");
        ContextPolicy::v2().validate().expect("v2");
    }

    /// An inconsistent policy is refused rather than behaving oddly.
    #[test]
    fn inverted_hysteresis_is_refused() {
        let mut p = ContextPolicy::v2();
        p.drop_threshold = 0.9;
        let err = p.validate().expect_err("a record could never be held");
        assert!(err.contains("never be held"), "{err}");
    }

    /// The priors table is exhaustive over both enums.
    ///
    /// Stated as a test because the failure is silent: a missing pairing falls
    /// back to 1.0, so a new `RecordKind` would be scored neutrally forever and
    /// nothing would say so.
    #[test]
    fn every_goal_and_record_kind_has_a_prior() {
        let p = ContextPolicy::v2();
        for goal in GOAL_KINDS {
            for kind in RecordKind::ALL {
                assert!(
                    p.kind_priors.contains_key(&prior_key(*goal, *kind)),
                    "{} has no prior",
                    prior_key(*goal, *kind)
                );
            }
        }
        for kind in RecordKind::ALL {
            assert!(
                p.half_lives.contains_key(kind.as_str()),
                "{} has no half-life",
                kind.as_str()
            );
        }
    }

    /// The asymmetry §26.2 exists for.
    #[test]
    fn a_finding_outlives_a_correction_by_an_order_of_magnitude() {
        let p = ContextPolicy::v2();
        assert!(
            p.half_life(RecordKind::Finding) > p.half_life(RecordKind::Correction) * 10.0,
            "one constant for both is wrong in one direction"
        );
    }

    /// The policy round-trips through JSON, because it rides in every trajectory.
    #[test]
    fn a_policy_round_trips_through_a_trajectory() {
        let p = ContextPolicy::v2();
        let json = serde_json::to_string(&p).expect("serialise");
        let back: ContextPolicy = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(p, back, "a policy that cannot replay cannot be A/B'd");
    }

    /// An undeclared prior is neutral, not zero.
    #[test]
    fn an_unfitted_policy_is_neutral_rather_than_silent() {
        let p = ContextPolicy::v1();
        assert!((p.kind_prior(GoalKind::Research, RecordKind::Finding) - 1.0).abs() < f32::EPSILON);
        assert!(p.half_life(RecordKind::Finding) > 100.0);
    }
}
