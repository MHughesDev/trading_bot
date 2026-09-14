//! Searcher selection, ASHA hardening and the early-stopping rule
//! (SPEC §11.1–§11.3, checklist 2.6/2.7/2.8/2.10, ADR-P2-08/09/10).
//!
//! Three things that are usually knobs and are rules here.
//!
//! ## 2.6 — which searcher runs is a function, not a setting
//!
//! [`select_searcher`] takes the budget, the dimensionality and the parallelism
//! and returns one arm. It is a pure function of facts the campaign already
//! declared, logged at `decision_tier = rule`, because "which optimiser did you
//! use" is otherwise answered by whoever set the config last and is
//! unreconstructable afterwards.
//!
//! Two arms are deliberately absent. GP-BO over *strategy* parameters is
//! deferred until a measured case needs it — strategy spaces here are few,
//! mixed and conditional, which is TPE's shape rather than a GP's. CMA-ES and
//! DEHB are N/A: a campaign on one box does not reach two hundred full-train
//! equivalents, and an arm nobody can run is an arm that rots (ADR-P2-08).
//!
//! ## 2.8 — the five ASHA mitigations are defaults, not switches
//!
//! Every one of them exists because plain ASHA's failure mode is specific and
//! expensive: it promotes the configuration that got lucky early. There is no
//! per-mitigation off switch, and that is the design — a switch is a thing
//! somebody turns off at 2 a.m. to make a sweep finish.
//!
//! The one that catches most of the damage is [`AshaLadder::needs_reseed`]: the
//! top three are re-run with fresh **platform-held** seeds before an incumbent
//! is crowned. It costs three runs and it is the difference between an
//! incumbent that is better and an incumbent that drew well.
//!
//! ## 2.10 — stopping is a rule until a posterior exists
//!
//! [`EarlyStopper`] answers from the median stopping rule today. When a
//! learning-curve posterior exists it answers from
//! `P(final > incumbent + δ | partial) < 0.05` instead — and the *interface does
//! not change*, which is what makes the eventual A/B free. A stop is terminal
//! and `right_asha`: a stopped trial is a censored observation, not a failure,
//! and counting it as a failure is how a search that works starts looking like
//! one that does not.

use ledger::internal::{choose_rung, Answer, Rung};
use serde::{Deserialize, Serialize};

// ───────────────────────────────────────────────────────────────────────────────
// 2.6 — searcher selection
// ───────────────────────────────────────────────────────────────────────────────

/// The shape of a search space, as far as the selection rule cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceShape {
    /// Every dimension is continuous and unconditional.
    Continuous,
    /// Mixed types, or dimensions that only exist when another takes a
    /// particular value. This is what a strategy parameter space looks like.
    MixedOrConditional,
}

/// The searchers this platform can actually run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Searcher {
    /// No search: replay the reference portfolio. With fewer than ten trials
    /// there is nothing to learn from, and a ten-sample optimiser is a random
    /// draw with extra steps.
    PortfolioReplay,
    /// Prior-weighted random over the declared ranges.
    PriorWeightedRandom,
    /// Tree-structured Parzen estimator. Handles mixed and conditional spaces,
    /// which is what strategy parameters are.
    Tpe,
    /// Gaussian-process Bayesian optimisation with the √D lengthscale prior
    /// (2.7). Model hyper-parameters only, in the Python sidecar.
    GpBo,
}

impl Searcher {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PortfolioReplay => "portfolio_replay",
            Self::PriorWeightedRandom => "prior_weighted_random",
            Self::Tpe => "tpe",
            Self::GpBo => "gp_bo",
        }
    }

    /// Whether this arm runs in the Python sidecar rather than in Rust.
    #[must_use]
    pub fn is_sidecar(self) -> bool {
        matches!(self, Self::GpBo)
    }
}

/// Below this budget there is nothing for an optimiser to learn from.
pub const MIN_BUDGET_FOR_SEARCH: u32 = 10;
/// Below this, a prior-weighted random draw is as good as a model.
pub const MIN_BUDGET_FOR_MODEL: u32 = 30;
/// Above this, §11.1's table calls for CMA-ES/DEHB — N/A at this scale, and the
/// revisit trigger is a campaign whose `max_trials` exceeds it (ADR-P2-08).
pub const CMAES_REVISIT_BUDGET: u32 = 200;
/// GP-BO only earns its keep above this dimensionality.
pub const MIN_DIMS_FOR_GP: u32 = 3;
/// At or above this parallelism, ASHA runs underneath whatever arm was chosen.
pub const MIN_PARALLELISM_FOR_ASHA: u32 = 8;

/// One selection, with the reason it was made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchPlan {
    pub searcher: Searcher,
    /// ASHA runs underneath when the parallelism is there to exploit it.
    pub asha: bool,
    /// Why this arm. Logged on the campaign's `propose` decision, so "which
    /// optimiser did you use and why" is answerable from the ledger.
    pub rationale: String,
    /// True when the budget is past the point §11.1 hands over to CMA-ES/DEHB,
    /// which this platform does not run. The plan is still valid — TPE keeps
    /// going — and the flag is the revisit trigger being visible rather than
    /// remembered.
    pub beyond_supported_budget: bool,
}

/// §11.1's table, as a function (ADR-P2-08).
///
/// `budget` is the campaign's trial budget, `dims` the number of free
/// dimensions, `parallelism` how many can run at once.
///
/// `strategy_space` says whether this is a strategy parameter space or a model
/// hyper-parameter space, and it is the one input that is not about size: GP-BO
/// is deferred for strategy parameters regardless of shape, because nobody has
/// measured a case here where it beats TPE and an unmeasured arm is a guess with
/// a citation.
#[must_use]
pub fn select_searcher(
    budget: u32,
    dims: u32,
    parallelism: u32,
    shape: SpaceShape,
    strategy_space: bool,
) -> Answer<SearchPlan> {
    // Selection is a rule and says so. There is no learned tier here yet, and
    // `choose_rung` with no trained model returns `Rule` whatever the ledger
    // holds — reporting `learned` because the data exists is the lie ADR-P5-01
    // names.
    let (rung, evidence) = choose_rung("M4", i64::from(budget), None, false, false);

    let asha = parallelism >= MIN_PARALLELISM_FOR_ASHA;
    let beyond = budget > CMAES_REVISIT_BUDGET;

    let (searcher, why) = if budget < MIN_BUDGET_FOR_SEARCH {
        (
            Searcher::PortfolioReplay,
            format!(
                "budget {budget} < {MIN_BUDGET_FOR_SEARCH}: too few trials for an optimiser to \
                 learn anything, so the reference portfolio is replayed rather than searched"
            ),
        )
    } else if budget < MIN_BUDGET_FOR_MODEL {
        (
            Searcher::PriorWeightedRandom,
            format!(
                "budget {budget} in [{MIN_BUDGET_FOR_SEARCH}, {MIN_BUDGET_FOR_MODEL}): \
                 prior-weighted random over the declared ranges; a model fitted on this many \
                 points is fitting noise"
            ),
        )
    } else if shape == SpaceShape::Continuous && dims > MIN_DIMS_FOR_GP && !strategy_space {
        (
            Searcher::GpBo,
            format!(
                "{dims} continuous model hyper-parameters at budget {budget}: GP-BO with the \
                 √D lengthscale prior (§11.1)"
            ),
        )
    } else if strategy_space && shape == SpaceShape::Continuous && dims > MIN_DIMS_FOR_GP {
        (
            Searcher::Tpe,
            format!(
                "{dims} continuous strategy parameters at budget {budget}: TPE. GP-BO over \
                 strategy parameters is deferred until a measured case needs it (ADR-P2-08)"
            ),
        )
    } else {
        (
            Searcher::Tpe,
            format!(
                "mixed or conditional space of {dims} dimensions at budget {budget}: TPE"
            ),
        )
    };

    let rationale = if beyond {
        format!(
            "{why}. Budget {budget} is past the {CMAES_REVISIT_BUDGET} at which §11.1 hands over \
             to CMA-ES/DEHB, which this deployment does not run — the arm stays TPE and this is \
             the revisit trigger"
        )
    } else {
        why
    };

    Answer {
        value: SearchPlan { searcher, asha, rationale, beyond_supported_budget: beyond },
        rung,
        evidence,
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// 2.8 — ASHA, hardened
// ───────────────────────────────────────────────────────────────────────────────

/// How a rung's score is taken from a candidate's observations.
///
/// Never a single last value, in either case. A backtest's last value is one
/// draw; a training curve's last value is one step, and the step after a lucky
/// batch looks like progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RungMetric {
    /// The sealed distribution's worst-case-robust statistic. What a backtest
    /// rung is scored on.
    WorstCaseRobust,
    /// Mean of the last three points. What a training curve's rung is scored on
    /// — enough to smooth a single noisy step, short enough to still be recent.
    LastThreeMean,
}

impl RungMetric {
    /// Score a candidate's observations at this rung.
    ///
    /// `None` when there is not enough to score. A rung that cannot be scored
    /// does not promote and does not stop — it waits, which is the only answer
    /// that is not a guess.
    #[must_use]
    pub fn score(self, observations: &[f64]) -> Option<f64> {
        let finite: Vec<f64> = observations.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        match self {
            Self::WorstCaseRobust => {
                // The 5th percentile, nearest-rank. "Robust" here means the
                // result survives its own bad tail, not that its average is good.
                let mut v = finite;
                v.sort_by(f64::total_cmp);
                #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let idx = ((v.len() as f64 * 0.05).ceil() as usize).saturating_sub(1);
                v.get(idx).copied()
            }
            Self::LastThreeMean => {
                let tail: Vec<f64> = finite.iter().rev().take(3).copied().collect();
                (!tail.is_empty()).then(|| tail.iter().sum::<f64>() / tail.len() as f64)
            }
        }
    }
}

/// One candidate's standing at a rung.
#[derive(Clone, Debug, PartialEq)]
pub struct RungEntry {
    pub candidate: String,
    pub score: f64,
    /// How many independent replicates produced `score`. The Wilcoxon pruner
    /// needs three; below that it does not run, and saying so is better than
    /// running a test that cannot reject.
    pub replicates: usize,
}

/// The ASHA ladder, with all five mitigations on and no way to turn one off.
#[derive(Clone, Debug)]
pub struct AshaLadder {
    metric: RungMetric,
    /// Promote the top `1/eta` of a rung.
    eta: u32,
    /// Rungs a candidate must complete before it is eligible to be stopped.
    /// From the ledger per task family; cold start is one full rung, because
    /// stopping something that has not finished a rung is stopping it on noise.
    grace_rungs: u32,
    /// PASHA's soft-ranking tolerance: the p90 of observed rank-swap gaps. Cold
    /// start is 0 — plain ASHA — recorded as the `rule` tier rather than as a
    /// tuned value nobody measured.
    epsilon: f64,
}

/// §11.2's default promotion ratio.
pub const DEFAULT_ETA: u32 = 3;
/// Replicates needed before the Wilcoxon pruner runs at all.
pub const WILCOXON_MIN_REPLICATES: usize = 3;
/// How many survivors are re-run on fresh platform seeds before an incumbent is
/// crowned.
pub const RESEED_TOP_N: usize = 3;

impl AshaLadder {
    /// A cold-start ladder: plain ASHA, one full rung of grace.
    #[must_use]
    pub fn cold_start(metric: RungMetric) -> Self {
        Self { metric, eta: DEFAULT_ETA, grace_rungs: 1, epsilon: 0.0 }
    }

    /// A ladder calibrated from the ledger.
    ///
    /// `epsilon` is the p90 of observed rank-swap gaps for this task family —
    /// how far apart two candidates have to be before their order is stable.
    /// Below it, ASHA's ranking is noise being treated as a decision.
    #[must_use]
    pub fn calibrated(metric: RungMetric, grace_rungs: u32, epsilon: f64) -> Self {
        Self {
            metric,
            eta: DEFAULT_ETA,
            grace_rungs: grace_rungs.max(1),
            epsilon: if epsilon.is_finite() && epsilon > 0.0 { epsilon } else { 0.0 },
        }
    }

    #[must_use]
    pub fn epsilon(&self) -> f64 {
        self.epsilon
    }

    #[must_use]
    pub fn metric(&self) -> RungMetric {
        self.metric
    }

    /// Which candidates survive this rung.
    ///
    /// Two mitigations are in here. The cut is the top `1/eta` **plus everyone
    /// within `epsilon` of the cut** (PASHA's soft ranking): a candidate that is
    /// indistinguishable from a survivor has not been shown to be worse, and
    /// cutting it is a decision made on noise. And nothing is cut before
    /// `grace_rungs` have completed.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn survivors(&self, rung: u32, entries: &[RungEntry]) -> Vec<String> {
        if rung < self.grace_rungs || entries.is_empty() {
            return entries.iter().map(|e| e.candidate.clone()).collect();
        }
        let mut sorted: Vec<&RungEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| b.score.total_cmp(&a.score));

        let keep = (entries.len() as f64 / f64::from(self.eta.max(1))).ceil().max(1.0) as usize;
        let cut = sorted[keep.min(sorted.len()) - 1].score;

        sorted
            .iter()
            .filter(|e| e.score >= cut - self.epsilon)
            .map(|e| e.candidate.clone())
            .collect()
    }

    /// Whether the Wilcoxon pruner may run on this pair.
    ///
    /// Three replicates each, minimum. With fewer, the signed-rank test cannot
    /// reject at any conventional level, and running it anyway produces a
    /// p-value that looks like evidence of no difference.
    #[must_use]
    pub fn wilcoxon_applicable(a: &RungEntry, b: &RungEntry) -> bool {
        a.replicates >= WILCOXON_MIN_REPLICATES && b.replicates >= WILCOXON_MIN_REPLICATES
    }

    /// The candidates that must be re-run on fresh platform-held seeds before
    /// one of them is crowned (§12.7, mitigation 5).
    ///
    /// This is the mitigation that catches most of the damage. ASHA's whole
    /// failure mode is promoting the configuration that got lucky early; re-running
    /// the top three on seeds the search never saw is three runs against exactly
    /// that. The seeds come from the campaign, which the candidate cannot read.
    #[must_use]
    pub fn needs_reseed(entries: &[RungEntry]) -> Vec<String> {
        let mut sorted: Vec<&RungEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
        sorted.iter().take(RESEED_TOP_N).map(|e| e.candidate.clone()).collect()
    }
}

// ───────────────────────────────────────────────────────────────────────────────
// 2.10 — early stopping
// ───────────────────────────────────────────────────────────────────────────────

/// What to do with a partially observed candidate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopDecision {
    Continue,
    /// Stop. **Terminal and `right_asha`** — a stopped trial is a censored
    /// observation, not a failure. Counting it as a failure is how a search that
    /// works starts looking like one that does not.
    Stop { reason: String },
}

impl StopDecision {
    #[must_use]
    pub fn stops(&self) -> bool {
        matches!(self, Self::Stop { .. })
    }

    /// The censoring a stop implies. One value, stated once.
    #[must_use]
    pub fn censoring(&self) -> Option<ledger::Censoring> {
        self.stops().then_some(ledger::Censoring::RightAsha)
    }
}

/// The early-stopping decision, with its tier (ADR-P2-10).
pub struct EarlyStopper {
    /// `P(final > incumbent + δ | partial)`, when a learning-curve posterior
    /// exists. `None` is the ordinary case today and selects the median rule.
    posterior: Option<f64>,
}

/// §11.3's threshold: below this posterior probability of beating the incumbent
/// by the declared effect size, stop.
pub const STOP_POSTERIOR: f64 = 0.05;

impl EarlyStopper {
    /// The day-one rule tier.
    #[must_use]
    pub fn median_rule() -> Self {
        Self { posterior: None }
    }

    /// The learned tier, once a curve posterior exists (4.4, promoted under
    /// 4.12). The interface is unchanged, which is what makes the eventual
    /// rule-versus-learned comparison free.
    #[must_use]
    pub fn with_posterior(p_beats_incumbent: f64) -> Self {
        Self { posterior: p_beats_incumbent.is_finite().then_some(p_beats_incumbent) }
    }

    /// Decide, and say at what tier.
    ///
    /// The median rule: stop if the candidate's current value is below the
    /// median of the values other candidates had at this same step. It is
    /// §13's day-one heuristic and it is deliberately crude — the point is that
    /// it is *cheap and unbiased*, not that it is accurate.
    #[must_use]
    pub fn decide(
        &self,
        current: f64,
        peers_at_this_step: &[f64],
        curves_observed: i64,
    ) -> Answer<StopDecision> {
        let (mut rung, evidence) = choose_rung("M3", curves_observed, None, self.posterior.is_some(), false);

        if let Some(p) = self.posterior {
            rung = Rung::Learned;
            let decision = if p < STOP_POSTERIOR {
                StopDecision::Stop {
                    reason: format!(
                        "P(final beats the incumbent by δ | partial) = {p:.3} < {STOP_POSTERIOR}"
                    ),
                }
            } else {
                StopDecision::Continue
            };
            return Answer { value: decision, rung, evidence };
        }

        let finite: Vec<f64> = peers_at_this_step.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.is_empty() || !current.is_finite() {
            // Nothing to compare against. Continuing costs compute; stopping
            // costs the answer, and only one of those is recoverable.
            return Answer { value: StopDecision::Continue, rung, evidence };
        }
        let mut sorted = finite;
        sorted.sort_by(f64::total_cmp);
        let mid = sorted.len() / 2;
        let median = if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        } else {
            sorted[mid]
        };

        let decision = if current < median {
            StopDecision::Stop {
                reason: format!(
                    "median rule: {current:.4} is below the {median:.4} its {} peers reached at \
                     this step",
                    sorted.len()
                ),
            }
        } else {
            StopDecision::Continue
        };
        Answer { value: decision, rung, evidence }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── 2.6 ─────────────────────────────────────────────────────────────────

    #[test]
    fn a_tiny_budget_does_not_search_at_all() {
        let p = select_searcher(6, 4, 1, SpaceShape::MixedOrConditional, true);
        assert_eq!(p.value.searcher, Searcher::PortfolioReplay);
        assert!(p.value.rationale.contains("too few trials"), "{}", p.value.rationale);
        assert!(!p.value.asha);
    }

    #[test]
    fn a_small_budget_draws_rather_than_models() {
        let p = select_searcher(20, 6, 2, SpaceShape::Continuous, false);
        assert_eq!(p.value.searcher, Searcher::PriorWeightedRandom);
        assert!(p.value.rationale.contains("fitting noise"), "{}", p.value.rationale);
    }

    #[test]
    fn a_mixed_space_gets_tpe() {
        let p = select_searcher(80, 5, 4, SpaceShape::MixedOrConditional, true);
        assert_eq!(p.value.searcher, Searcher::Tpe);
    }

    /// GP-BO is for model hyper-parameters. Over strategy parameters it is
    /// deferred until a measured case needs it, and the rationale says so
    /// rather than the arm silently not appearing.
    #[test]
    fn gp_bo_is_for_model_hyperparameters_not_strategy_parameters() {
        let model = select_searcher(80, 6, 4, SpaceShape::Continuous, false);
        assert_eq!(model.value.searcher, Searcher::GpBo);
        assert!(model.value.searcher.is_sidecar());

        let strategy = select_searcher(80, 6, 4, SpaceShape::Continuous, true);
        assert_eq!(strategy.value.searcher, Searcher::Tpe);
        assert!(strategy.value.rationale.contains("deferred"), "{}", strategy.value.rationale);
    }

    #[test]
    fn low_dimensional_continuous_spaces_do_not_need_a_gp() {
        let p = select_searcher(80, 2, 4, SpaceShape::Continuous, false);
        assert_eq!(p.value.searcher, Searcher::Tpe);
    }

    #[test]
    fn asha_joins_when_the_parallelism_is_there() {
        assert!(!select_searcher(80, 5, 7, SpaceShape::MixedOrConditional, true).value.asha);
        assert!(select_searcher(80, 5, 8, SpaceShape::MixedOrConditional, true).value.asha);
    }

    /// The arm this platform does not run is a *visible* revisit trigger rather
    /// than a silent fallback.
    #[test]
    fn a_budget_past_the_cmaes_handover_says_so() {
        let p = select_searcher(500, 5, 16, SpaceShape::MixedOrConditional, true);
        assert!(p.value.beyond_supported_budget);
        assert_eq!(p.value.searcher, Searcher::Tpe, "it still searches");
        assert!(p.value.rationale.contains("revisit trigger"), "{}", p.value.rationale);
    }

    #[test]
    fn selection_is_a_rule_and_says_so() {
        let p = select_searcher(80, 5, 4, SpaceShape::MixedOrConditional, true);
        assert_eq!(p.rung, Rung::Rule);
        assert_eq!(p.decision_tier(), ledger::DecisionTier::Rule);
    }

    // ── 2.8 ─────────────────────────────────────────────────────────────────

    /// Never a single last value. A backtest rung is scored on its own bad tail;
    /// a curve rung on three steps, not one.
    #[test]
    fn a_rung_is_never_scored_on_one_observation() {
        let obs = [0.9, 0.1, 0.5, 0.4, 0.6, 0.2, 0.8, 0.3, 0.7, 0.05];
        let robust = RungMetric::WorstCaseRobust.score(&obs).unwrap();
        assert!((robust - 0.05).abs() < 1e-12, "the worst tail, not the last value: {robust}");

        let curve = [1.0, 0.8, 0.6, 0.4, 0.2];
        let mean3 = RungMetric::LastThreeMean.score(&curve).unwrap();
        assert!((mean3 - 0.4).abs() < 1e-12, "{mean3}");
        assert!(mean3 > *curve.last().unwrap(), "one lucky step cannot carry the rung");
    }

    #[test]
    fn a_rung_with_nothing_to_score_waits() {
        assert_eq!(RungMetric::WorstCaseRobust.score(&[]), None);
        assert_eq!(RungMetric::LastThreeMean.score(&[f64::NAN]), None);
    }

    fn entries(scores: &[(&str, f64)], replicates: usize) -> Vec<RungEntry> {
        scores
            .iter()
            .map(|(c, s)| RungEntry { candidate: (*c).to_string(), score: *s, replicates })
            .collect()
    }

    /// Nothing is cut before the grace period. Stopping a candidate that has not
    /// finished a rung is stopping it on noise.
    #[test]
    fn the_grace_period_protects_an_unfinished_rung() {
        let l = AshaLadder::cold_start(RungMetric::WorstCaseRobust);
        let e = entries(&[("a", 1.0), ("b", 0.9), ("c", 0.1)], 1);
        assert_eq!(l.survivors(0, &e).len(), 3, "rung 0 is inside the grace period");
        assert_eq!(l.survivors(1, &e).len(), 1);
    }

    /// PASHA's soft ranking: a candidate indistinguishable from a survivor has
    /// not been shown to be worse, and cutting it is a decision made on noise.
    #[test]
    fn soft_ranking_keeps_candidates_that_are_not_distinguishably_worse() {
        let e = entries(&[("a", 1.00), ("b", 0.99), ("c", 0.98), ("d", 0.10)], 3);

        // eta = 3 over four candidates keeps the top two.
        let plain = AshaLadder::cold_start(RungMetric::WorstCaseRobust);
        assert_eq!(plain.survivors(2, &e), vec!["a".to_string(), "b".to_string()]);

        let soft = AshaLadder::calibrated(RungMetric::WorstCaseRobust, 1, 0.05);
        let kept = soft.survivors(2, &e);
        assert_eq!(kept.len(), 3, "a, b and c are within the swap gap: {kept:?}");
        assert!(!kept.contains(&"d".to_string()), "d is distinguishably worse");
    }

    #[test]
    fn a_cold_start_ladder_is_plain_asha_and_says_so() {
        let l = AshaLadder::cold_start(RungMetric::LastThreeMean);
        assert!((l.epsilon() - 0.0).abs() < f64::EPSILON);
        // A nonsensical calibration falls back to plain ASHA rather than to a
        // number nobody measured.
        assert!((AshaLadder::calibrated(RungMetric::LastThreeMean, 1, f64::NAN).epsilon()).abs() < f64::EPSILON);
    }

    /// With fewer than three replicates the signed-rank test cannot reject at any
    /// conventional level. Running it anyway yields a p-value that reads as
    /// evidence of no difference.
    #[test]
    fn the_wilcoxon_pruner_does_not_run_on_too_few_replicates() {
        let two = entries(&[("a", 1.0)], 2);
        let three = entries(&[("b", 1.0)], 3);
        assert!(!AshaLadder::wilcoxon_applicable(&two[0], &three[0]));
        assert!(AshaLadder::wilcoxon_applicable(&three[0], &three[0]));
    }

    /// The mitigation that catches most of the damage: the top three are re-run
    /// on seeds the search never saw, before one of them is crowned.
    #[test]
    fn the_top_three_are_reseeded_before_an_incumbent_is_crowned() {
        let e = entries(&[("a", 0.5), ("b", 0.9), ("c", 0.7), ("d", 0.8), ("e", 0.1)], 3);
        assert_eq!(AshaLadder::needs_reseed(&e), vec!["b".to_string(), "d".to_string(), "c".to_string()]);
        // A shorter field reseeds everything it has, best first, rather than
        // refusing.
        assert_eq!(AshaLadder::needs_reseed(&e[..2]), vec!["b".to_string(), "a".to_string()]);
    }

    // ── 2.10 ────────────────────────────────────────────────────────────────

    #[test]
    fn the_median_rule_stops_a_laggard_and_keeps_a_leader() {
        let s = EarlyStopper::median_rule();
        let peers = [0.5, 0.6, 0.7, 0.8, 0.9];
        let stop = s.decide(0.2, &peers, 40);
        assert!(stop.value.stops(), "{:?}", stop.value);
        assert_eq!(stop.rung, Rung::Rule);

        let keep = s.decide(0.85, &peers, 40);
        assert!(!keep.value.stops());
    }

    /// A stop is a censored observation, not a failure. This is the distinction
    /// M3/M4/M5 read, and getting it wrong makes a working search look broken.
    #[test]
    fn a_stop_is_right_asha_censored_never_failed() {
        let s = EarlyStopper::median_rule();
        let d = s.decide(0.1, &[0.5, 0.6, 0.7], 40).value;
        assert_eq!(d.censoring(), Some(ledger::Censoring::RightAsha));
        assert_ne!(d.censoring(), Some(ledger::Censoring::Failed));
        assert_eq!(StopDecision::Continue.censoring(), None);
    }

    /// Nothing to compare against is not a reason to stop. Continuing costs
    /// compute; stopping costs the answer, and only one of those is recoverable.
    #[test]
    fn no_peers_means_no_stop() {
        let s = EarlyStopper::median_rule();
        assert!(!s.decide(0.1, &[], 40).value.stops());
        assert!(!s.decide(f64::NAN, &[0.5, 0.6], 40).value.stops());
    }

    /// The learned tier answers through the same interface, which is what makes
    /// the eventual rule-versus-learned comparison a query rather than a
    /// rewrite.
    #[test]
    fn a_posterior_answers_through_the_same_interface() {
        let hopeless = EarlyStopper::with_posterior(0.01).decide(0.9, &[0.1], 5_000);
        assert!(hopeless.value.stops(), "a leader with no posterior support still stops");
        assert_eq!(hopeless.rung, Rung::Learned);

        let promising = EarlyStopper::with_posterior(0.4).decide(0.1, &[0.9], 5_000);
        assert!(!promising.value.stops(), "a laggard the posterior believes in continues");
        assert_eq!(promising.rung, Rung::Learned);
    }
}
