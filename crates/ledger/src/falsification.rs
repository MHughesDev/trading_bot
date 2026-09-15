//! Kills and evaluator defects — the falsifier's output types
//! (EXTERNAL-REVIEW-2026-09 §4; ADR-P6-05, ADR-P6-06).
//!
//! ## Why the agent's primary job is falsification
//!
//! The argument is reward density, not philosophy. "Find alpha" produces a
//! usable learning signal roughly twice a year and each one resolves on a
//! ninety-day lag; "prove this is noise" produces one every few minutes, and it
//! is usually right, because most strategies are noise. Twenty-four noisy labels
//! a year is not a small dataset — it is not a dataset. Anything the agent can
//! actually learn has to be scored on the dense signal.
//!
//! ## The degenerate policy, and the type that blocks it
//!
//! An agent rewarded for killing will kill everything. The defence is not a
//! quota, it is a definition: a [`Kill`] requires a [`Reproduction`], and a
//! [`Reproduction`] requires a deterministic replay in which a stated
//! expectation is observed to fail. "Sharpe is only 0.4" cannot be written down
//! in these types. "Returns invert when signals are shifted one bar" can.
//!
//! [`Kill`] and [`EvaluatorDefect`] therefore have **no public constructor**.
//! The only route is through a `Reproduction`, and the only route to one of
//! those is [`Reproduction::replayed`], which will not accept an expectation
//! equal to its observation. A report nobody can reproduce scores
//! [`Score::ZERO`] — not a small number, zero — and the type makes that the
//! default rather than a policy somebody remembers to apply.
//!
//! ## Why an evaluator defect outscores a promotion
//!
//! If score comes from promotions, an agent that discovers the simulator lets it
//! buy the intrabar low will exploit it silently, because reporting it destroys
//! its best result. The only stable fix is to make the report worth more than
//! the exploit. [`Score`] encodes that ordering as a total order the scheduler
//! reads, so it is a property of the type rather than a number in a config file
//! somebody tunes downward when promotions look thin.
//!
//! The platform has already paid for this lesson once: the `array_length(x,1)`
//! NULL-CHECK hole accepted empty arrays in four constraints the spec calls
//! REQUIRED, and only a live database found it. That is exactly the class of
//! defect a kill-rewarded agent enumerates.

use serde::{Deserialize, Serialize};

/// What a falsification is worth, as a total order.
///
/// Ordering is the point: `EvaluatorDefect > Promotion > Kill > Zero`. It is
/// derived from the variant order below and cannot be re-weighted at runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Score {
    /// An unreproducible report, a duplicate, or a refuted one.
    Zero,
    /// A candidate was shown to be noise, reproducibly.
    Kill,
    /// A candidate passed the full gate stack. Present here only so the
    /// ordering against the other two is stated in one place.
    Promotion,
    /// The evaluator itself was shown to be wrong, reproducibly. The most
    /// valuable thing the agent can produce, because everything else the
    /// platform reports depends on the evaluator being right.
    EvaluatorDefect,
}

impl Score {
    pub const ZERO: Self = Self::Zero;

    /// Whether this outranks a promotion — the property the scheduler needs and
    /// the one a config file must not be able to change.
    #[must_use]
    pub fn outranks_promotion(self) -> bool {
        self > Self::Promotion
    }
}

/// A deterministic replay in which a stated expectation failed.
///
/// Sealed. [`Reproduction::replayed`] is the only constructor, and it refuses
/// the two shapes that make a "finding" unfalsifiable: an expectation identical
/// to the observation, and a replay with no seed or run to replay.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reproduction {
    /// The run, backtest or migration the replay is against.
    artifact_ref: String,
    /// The seed. A replay that cannot be pinned to a seed is an anecdote.
    seed: u64,
    /// What the reporter said would happen, in the reporter's own words.
    expected: String,
    /// What happened instead.
    observed: String,
    /// The exact steps. Must be non-empty: a reproduction with no steps is a
    /// claim, and the ten-minute rule is what makes a kill checkable by a human.
    steps: Vec<String>,
}

impl Reproduction {
    /// Mint a reproduction from a replay that actually failed.
    ///
    /// # Errors
    /// - [`FalsificationError::NotFalsifiable`] when `expected == observed`:
    ///   nothing was refuted.
    /// - [`FalsificationError::NoSteps`] when no steps are supplied.
    /// - [`FalsificationError::MissingArtifact`] when there is nothing to replay.
    pub fn replayed(
        artifact_ref: impl Into<String>,
        seed: u64,
        expected: impl Into<String>,
        observed: impl Into<String>,
        steps: Vec<String>,
    ) -> Result<Self, FalsificationError> {
        let artifact_ref = artifact_ref.into();
        let expected = expected.into();
        let observed = observed.into();

        if artifact_ref.trim().is_empty() {
            return Err(FalsificationError::MissingArtifact);
        }
        if expected.trim().is_empty() || observed.trim().is_empty() {
            return Err(FalsificationError::NotFalsifiable);
        }
        if expected.trim() == observed.trim() {
            return Err(FalsificationError::NotFalsifiable);
        }
        let steps: Vec<String> = steps.into_iter().filter(|s| !s.trim().is_empty()).collect();
        if steps.is_empty() {
            return Err(FalsificationError::NoSteps);
        }

        Ok(Self { artifact_ref, seed, expected, observed, steps })
    }

    #[must_use]
    pub fn artifact_ref(&self) -> &str {
        &self.artifact_ref
    }
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }
    #[must_use]
    pub fn expected(&self) -> &str {
        &self.expected
    }
    #[must_use]
    pub fn observed(&self) -> &str {
        &self.observed
    }
    #[must_use]
    pub fn steps(&self) -> &[String] {
        &self.steps
    }
}

/// Whether anyone has re-run the reproduction.
///
/// `Unverified` is the state a report is born in, and it scores zero. Verified
/// scores. Refuted scores zero *and is kept*, because a pattern of refuted
/// reports is itself the signal that a reporter is guessing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Unverified,
    Verified,
    Refuted,
}

/// A candidate shown to be noise, reproducibly.
///
/// No public constructor: the only route is [`Kill::reported`], which demands a
/// [`Reproduction`]. A low score is not a kill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Kill {
    target_trial_id: String,
    /// The defect class, from a closed vocabulary. Free text here would let
    /// "underperforms" back in through the side door.
    defect: KillClass,
    reproduction: Reproduction,
    verification: Verification,
}

/// The ways a candidate can be *wrong*, as opposed to merely unprofitable.
///
/// Closed on purpose. Every variant names a defect a human can confirm in ten
/// minutes; none of them is a performance threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillClass {
    /// The strategy trades on a bar it computes from, or otherwise reads the
    /// future. Returns invert under a one-bar shift.
    Lookahead,
    /// P&L concentrated in a handful of bars traceable to bad data — a wick
    /// with no supporting prints, a stale quote, a bad split.
    DataArtifact,
    /// Break-even cost is below the venue's fee floor: the edge is smaller than
    /// the cost of taking it.
    BelowCostFloor,
    /// The holding period sits below the measured tradeable frontier.
    BelowTradeableFrontier,
    /// The result depends on a simulator behaviour no venue offers — fills at
    /// the intrabar low, unlimited size, no queue.
    FillModelExploit,
    /// The candidate is a restatement of one already in the ledger; its apparent
    /// novelty is a naming difference.
    DuplicateOfExisting,
    /// The metric is undefined or degenerate for this return series — a Sharpe
    /// over three trades, a ratio with a zero denominator.
    InvalidMetric,
}

impl Kill {
    /// Report a kill. The reproduction is not optional.
    #[must_use]
    pub fn reported(
        target_trial_id: impl Into<String>,
        defect: KillClass,
        reproduction: Reproduction,
    ) -> Self {
        Self {
            target_trial_id: target_trial_id.into(),
            defect,
            reproduction,
            verification: Verification::Unverified,
        }
    }

    /// Record the outcome of someone re-running the reproduction.
    #[must_use]
    pub fn verified(mut self, outcome: Verification) -> Self {
        self.verification = outcome;
        self
    }

    /// What this is worth. Unverified and refuted are both zero.
    #[must_use]
    pub fn score(&self) -> Score {
        match self.verification {
            Verification::Verified => Score::Kill,
            Verification::Unverified | Verification::Refuted => Score::Zero,
        }
    }

    #[must_use]
    pub fn target_trial_id(&self) -> &str {
        &self.target_trial_id
    }
    #[must_use]
    pub const fn defect(&self) -> KillClass {
        self.defect
    }
    #[must_use]
    pub const fn reproduction(&self) -> &Reproduction {
        &self.reproduction
    }
    #[must_use]
    pub const fn verification(&self) -> Verification {
        self.verification
    }
}

/// The evaluator itself shown to be wrong, reproducibly.
///
/// Scores above a promotion — see the module docs for why that ordering is the
/// only stable one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EvaluatorDefect {
    /// Which part of the platform is wrong: a gate, the simulator, a migration,
    /// a statistic.
    component: String,
    /// What the component claims to guarantee, quoted from spec or docs. A
    /// defect is a gap between a claim and a behaviour; without the claim there
    /// is only a behaviour.
    claimed_guarantee: String,
    reproduction: Reproduction,
    verification: Verification,
    /// Set once a regression test exists. A verified defect with no test is a
    /// defect that comes back.
    regression_test: Option<String>,
}

impl EvaluatorDefect {
    /// Report a defect in the evaluator.
    #[must_use]
    pub fn reported(
        component: impl Into<String>,
        claimed_guarantee: impl Into<String>,
        reproduction: Reproduction,
    ) -> Self {
        Self {
            component: component.into(),
            claimed_guarantee: claimed_guarantee.into(),
            reproduction,
            verification: Verification::Unverified,
            regression_test: None,
        }
    }

    #[must_use]
    pub fn verified(mut self, outcome: Verification) -> Self {
        self.verification = outcome;
        self
    }

    /// Name the regression test that now covers it.
    #[must_use]
    pub fn covered_by(mut self, test: impl Into<String>) -> Self {
        self.regression_test = Some(test.into());
        self
    }

    /// What this is worth.
    #[must_use]
    pub fn score(&self) -> Score {
        match self.verification {
            Verification::Verified => Score::EvaluatorDefect,
            Verification::Unverified | Verification::Refuted => Score::Zero,
        }
    }

    /// Whether this defect is fully discharged: verified *and* covered by a
    /// test. Verification alone closes the question and not the hole.
    #[must_use]
    pub fn is_discharged(&self) -> bool {
        self.verification == Verification::Verified && self.regression_test.is_some()
    }

    #[must_use]
    pub fn component(&self) -> &str {
        &self.component
    }
    #[must_use]
    pub fn claimed_guarantee(&self) -> &str {
        &self.claimed_guarantee
    }
    #[must_use]
    pub const fn reproduction(&self) -> &Reproduction {
        &self.reproduction
    }
    #[must_use]
    pub const fn verification(&self) -> Verification {
        self.verification
    }
    #[must_use]
    pub fn regression_test(&self) -> Option<&str> {
        self.regression_test.as_deref()
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum FalsificationError {
    #[error(
        "a reproduction must show an expectation failing: `expected` and `observed` are the same, \
         so nothing was refuted. A low score is not a kill."
    )]
    NotFalsifiable,
    #[error(
        "a reproduction must list its steps: a report a human cannot re-run in ten minutes scores \
         zero, and one with no steps cannot be re-run at all"
    )]
    NoSteps,
    #[error("a reproduction must name the run, backtest or migration it replays")]
    MissingArtifact,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> Reproduction {
        Reproduction::replayed(
            "run_01H8XYZ",
            42,
            "shifting every signal back one bar leaves the Sharpe roughly unchanged",
            "the Sharpe inverts from +2.9 to -2.7",
            vec![
                "re-run run_01H8XYZ with signal_shift=1".into(),
                "compare net_sharpe across the two results".into(),
            ],
        )
        .unwrap()
    }

    // ── the ordering that keeps the agent honest ─────────────────────────────

    /// The property the whole module exists for: an agent scored on promotions
    /// hides simulator bugs, because reporting one destroys its best result.
    #[test]
    fn a_verified_evaluator_defect_outscores_a_promotion() {
        let d = EvaluatorDefect::reported("sim.fill_model", "fills never cross the bar", good())
            .verified(Verification::Verified);
        assert!(d.score().outranks_promotion());
        assert!(d.score() > Score::Promotion);
        assert!(Score::Promotion > Score::Kill);
        assert!(Score::Kill > Score::Zero);
    }

    #[test]
    fn the_score_order_is_total_and_fixed() {
        let mut all = vec![Score::Promotion, Score::Zero, Score::EvaluatorDefect, Score::Kill];
        all.sort();
        assert_eq!(all, vec![Score::Zero, Score::Kill, Score::Promotion, Score::EvaluatorDefect]);
    }

    // ── the degenerate policy is blocked at the constructor ──────────────────

    /// "Sharpe is only 0.4" must be unwritable. The constructor refuses a
    /// reproduction whose expectation matches its observation, which is what a
    /// bare performance complaint reduces to.
    #[test]
    fn a_report_that_refutes_nothing_cannot_be_constructed() {
        let err = Reproduction::replayed(
            "run_1",
            1,
            "sharpe is 0.4",
            "sharpe is 0.4",
            vec!["look at it".into()],
        )
        .unwrap_err();
        assert_eq!(err, FalsificationError::NotFalsifiable);
    }

    #[test]
    fn a_reproduction_without_steps_is_refused() {
        assert_eq!(
            Reproduction::replayed("run_1", 1, "a", "b", vec![]).unwrap_err(),
            FalsificationError::NoSteps
        );
        assert_eq!(
            Reproduction::replayed("run_1", 1, "a", "b", vec!["  ".to_string(), String::new()]).unwrap_err(),
            FalsificationError::NoSteps
        );
    }

    #[test]
    fn a_reproduction_with_nothing_to_replay_is_refused() {
        assert_eq!(
            Reproduction::replayed("", 1, "a", "b", vec!["x".into()]).unwrap_err(),
            FalsificationError::MissingArtifact
        );
    }

    #[test]
    fn an_empty_expectation_or_observation_is_refused() {
        assert!(Reproduction::replayed("r", 1, "", "b", vec!["x".into()]).is_err());
        assert!(Reproduction::replayed("r", 1, "a", "  ", vec!["x".into()]).is_err());
    }

    // ── unverified scores zero, not a little ─────────────────────────────────

    #[test]
    fn an_unreproduced_report_scores_zero() {
        let k = Kill::reported("trial_9", KillClass::Lookahead, good());
        assert_eq!(k.score(), Score::ZERO);
        assert_eq!(k.verification(), Verification::Unverified);

        let d = EvaluatorDefect::reported("gates", "every gate records a verdict", good());
        assert_eq!(d.score(), Score::ZERO);
    }

    #[test]
    fn a_refuted_report_scores_zero_and_is_kept() {
        let k = Kill::reported("trial_9", KillClass::DataArtifact, good())
            .verified(Verification::Refuted);
        assert_eq!(k.score(), Score::ZERO);
        // Kept, because a pattern of refuted reports is itself the signal.
        assert_eq!(k.verification(), Verification::Refuted);
        assert_eq!(k.target_trial_id(), "trial_9");
    }

    #[test]
    fn a_verified_kill_scores_a_kill() {
        let k = Kill::reported("trial_9", KillClass::Lookahead, good())
            .verified(Verification::Verified);
        assert_eq!(k.score(), Score::Kill);
        assert!(!k.score().outranks_promotion());
    }

    // ── discharge ────────────────────────────────────────────────────────────

    /// Verification closes the question. Only a regression test closes the hole.
    #[test]
    fn a_verified_defect_is_not_discharged_until_a_test_covers_it() {
        let d = EvaluatorDefect::reported("migrations", "REQUIRED arrays are non-empty", good())
            .verified(Verification::Verified);
        assert!(!d.is_discharged());
        assert!(d.regression_test().is_none());

        let d = d.covered_by("no_check_requires_a_non_empty_array_with_array_length");
        assert!(d.is_discharged());
        assert_eq!(
            d.regression_test(),
            Some("no_check_requires_a_non_empty_array_with_array_length")
        );
    }

    #[test]
    fn a_test_on_an_unverified_defect_does_not_discharge_it() {
        let d = EvaluatorDefect::reported("sim", "no intrabar fills", good()).covered_by("t");
        assert!(!d.is_discharged());
    }

    // ── the vocabulary is closed ─────────────────────────────────────────────

    /// Every kill class names a defect, never a threshold. Pinned so that
    /// `Underperforms` cannot be added without this test being deleted first.
    #[test]
    fn every_kill_class_round_trips_and_names_a_defect_not_a_threshold() {
        for c in [
            KillClass::Lookahead,
            KillClass::DataArtifact,
            KillClass::BelowCostFloor,
            KillClass::BelowTradeableFrontier,
            KillClass::FillModelExploit,
            KillClass::DuplicateOfExisting,
            KillClass::InvalidMetric,
        ] {
            let s = serde_json::to_string(&c).unwrap();
            assert_eq!(serde_json::from_str::<KillClass>(&s).unwrap(), c);
            for banned in ["sharpe", "return", "underperform", "score", "worse"] {
                assert!(!s.to_lowercase().contains(banned), "{s} names a threshold, not a defect");
            }
        }
    }

    #[test]
    fn reports_round_trip_through_json() {
        let k = Kill::reported("t", KillClass::FillModelExploit, good())
            .verified(Verification::Verified);
        let s = serde_json::to_string(&k).unwrap();
        assert_eq!(serde_json::from_str::<Kill>(&s).unwrap(), k);

        let d = EvaluatorDefect::reported("c", "g", good()).covered_by("t");
        let s = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<EvaluatorDefect>(&s).unwrap(), d);
    }

    #[test]
    fn the_reproduction_keeps_everything_a_human_needs_to_rerun_it() {
        let r = good();
        assert_eq!(r.artifact_ref(), "run_01H8XYZ");
        assert_eq!(r.seed(), 42);
        assert_eq!(r.steps().len(), 2);
        assert!(r.expected().contains("unchanged"));
        assert!(r.observed().contains("inverts"));
    }
}
