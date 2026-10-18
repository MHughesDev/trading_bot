//! The shadow random-search arm (Set R-b RB-3.1 to RB-3.3, EV-09; BS-007 11 §8, §9).
//!
//! A campaign's best result means something only against what random search with the
//! same budget finds on the same sealed metric. EV-09 makes that arm part of the
//! campaign record: it is created with the campaign, not run afterwards, so it cannot
//! be re-rolled until it flatters (ADR-RB-03).
//!
//! # No agent-chosen value reaches the arm
//!
//! [`draw`] takes a space, a budget and a seed, and nothing else. The seed is
//! [`arm_seed`] of the campaign id, which the platform assigns. The space is the
//! **declared** one ([`declared_space`]), with no narrowing: a narrowing is the agent's
//! judgement about where to look, and a baseline that inherited it would hide the
//! agent's best idea inside the thing it is being compared with (ADR-RB-08).
//!
//! # The comparison
//!
//! The agent's best score is ranked against the distribution of *the best of `budget`
//! random draws*, which is what the agent's own number is: a maximum. That
//! distribution is bootstrapped from the arm's scores with a fixed seed, so the same
//! campaign always reports the same percentile. [`compare_to_random`] refuses to
//! report without a completed arm of equal budget (RB-3.2), and [`gate_check`] is the
//! predicate Gate 3 calls so a campaign cannot reach a verdict without it (RB-3.3).

use std::collections::BTreeMap;

use backtest::run::ParamMap;
use serde::{Deserialize, Serialize};

use crate::sampler::{RandomSampler, Rng, Sampler};
use crate::space::{SearchSpace, SpaceError};

/// The platform's seed for a campaign's arm: FNV-1a over a fixed tag and the id.
/// Stable across runs and machines, and not a function of anything the agent sets.
#[must_use]
pub fn arm_seed(campaign_id: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in b"random_arm:v1:".iter().chain(campaign_id.as_bytes()) {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The declared search space: no narrowing, whatever the agent asked for.
///
/// # Errors
/// [`SpaceError`] when the definition has nothing to sweep.
pub fn declared_space(
    def: &domain::strategy_def::StrategyDefinition,
) -> Result<SearchSpace, SpaceError> {
    SearchSpace::from_definition(def, &BTreeMap::new())
}

/// `budget` feasible points drawn uniformly from `space`. Infeasible draws are
/// re-drawn (bounded) exactly as the agent's arm's are, so the two arms evaluate the
/// same kind of point.
#[must_use]
pub fn draw(space: &SearchSpace, budget: usize, seed: u64) -> Vec<ParamMap> {
    let mut sampler = RandomSampler::new(seed);
    let mut out = Vec::with_capacity(budget);
    let mut attempts = 0usize;
    while out.len() < budget && attempts < budget.saturating_mul(50).max(50) {
        for x in sampler.propose(space, 1) {
            attempts += 1;
            let p = space.decode(&x);
            if space.feasible(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// An arm's state as the campaign record holds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub campaign_id: String,
    pub budget: usize,
    pub seed: u64,
    /// Sealed-metric score of each completed trial, in draw order.
    pub scores: Vec<f64>,
}

impl Arm {
    /// An arm for a campaign with the agent arm's budget.
    #[must_use]
    pub fn new(campaign_id: &str, budget: usize) -> Self {
        Self {
            campaign_id: campaign_id.to_string(),
            budget,
            seed: arm_seed(campaign_id),
            scores: Vec::new(),
        }
    }

    #[must_use]
    pub fn complete(&self) -> bool {
        self.scores.len() >= self.budget && self.budget > 0
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ArmError {
    #[error("this campaign has no random arm; EV-09 requires one created with the campaign")]
    NoArm,
    #[error("the random arm has run {done} of {budget} trials; it must complete before the campaign can be reported")]
    Incomplete { done: usize, budget: usize },
    #[error("the arms' budgets differ (agent {agent}, random {random}); a comparison is only matched at equal budget")]
    UnequalBudget { agent: usize, random: usize },
}

/// Gate 3's predicate (RB-3.3): the arm exists and has completed. The error names the
/// arm, which is what the gate's refusal quotes.
///
/// # Errors
/// [`ArmError`].
pub fn gate_check(arm: Option<&Arm>) -> Result<(), ArmError> {
    let arm = arm.ok_or(ArmError::NoArm)?;
    if arm.complete() {
        Ok(())
    } else {
        Err(ArmError::Incomplete {
            done: arm.scores.len(),
            budget: arm.budget,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Quantiles {
    pub p05: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn quantiles(mut v: Vec<f64>) -> Quantiles {
    v.sort_by(f64::total_cmp);
    Quantiles {
        p05: quantile(&v, 0.05),
        p25: quantile(&v, 0.25),
        p50: quantile(&v, 0.5),
        p75: quantile(&v, 0.75),
        p95: quantile(&v, 0.95),
    }
}

/// The report: both arms side by side, and where the agent's best sits.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RandomArmReport {
    pub budget: usize,
    pub agent_best: f64,
    /// Distribution of the sealed metric over the agent's trials.
    pub agent_scores: Quantiles,
    /// Distribution of the sealed metric over the random arm's trials.
    pub random_scores: Quantiles,
    /// Distribution of the best of `budget` random draws (bootstrapped).
    pub random_best_of_budget: Quantiles,
    /// Share of random best-of-budget outcomes the agent's best beats, in `[0, 1]`.
    /// 0.5 is "no better than a coin"; the number to read is how far above it.
    pub percentile: f64,
}

const BOOTSTRAP: usize = 4000;

/// Rank the agent's best score in the random arm's best-of-budget distribution.
///
/// `agent_scores` are the agent arm's trial scores on the same sealed metric.
/// Higher is better; the caller negates a cost.
///
/// # Errors
/// [`ArmError`] when there is no arm, it is incomplete, or its budget differs.
pub fn compare_to_random(
    agent_scores: &[f64],
    arm: Option<&Arm>,
) -> Result<RandomArmReport, ArmError> {
    gate_check(arm)?;
    let arm = arm.ok_or(ArmError::NoArm)?;
    if agent_scores.len() != arm.budget {
        return Err(ArmError::UnequalBudget {
            agent: agent_scores.len(),
            random: arm.budget,
        });
    }
    let finite: Vec<f64> = arm
        .scores
        .iter()
        .copied()
        .filter(|s| s.is_finite())
        .collect();
    let agent_best = agent_scores
        .iter()
        .copied()
        .filter(|s| s.is_finite())
        .fold(f64::NEG_INFINITY, f64::max);
    // A *smoothed* bootstrap: each draw is a random arm score plus Gaussian noise at
    // Silverman's bandwidth. A plain bootstrap of a maximum can never exceed the arm's own
    // maximum, so under the null (an agent no better than random) "the agent's best beats
    // every bootstrapped best" would be true half the time and the percentile would read
    // 1.0 for a campaign with no edge at all.
    let n = finite.len().max(1) as f64;
    let mean = finite.iter().sum::<f64>() / n;
    let sd = (finite.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    let bandwidth = 1.06 * sd * n.powf(-0.2);
    let mut rng = Rng::new(arm.seed ^ 0xB007_57A9);
    let mut maxes = Vec::with_capacity(BOOTSTRAP);
    for _ in 0..BOOTSTRAP {
        let m = (0..arm.budget)
            .map(|_| finite[rng.below(finite.len().max(1))] + bandwidth * rng.next_normal())
            .fold(f64::NEG_INFINITY, f64::max);
        maxes.push(m);
    }
    let beaten = maxes.iter().filter(|m| **m < agent_best).count() as f64;
    let ties = maxes
        .iter()
        .filter(|m| (**m - agent_best).abs() < f64::EPSILON)
        .count() as f64;
    Ok(RandomArmReport {
        budget: arm.budget,
        agent_best,
        agent_scores: quantiles(
            agent_scores
                .iter()
                .copied()
                .filter(|s| s.is_finite())
                .collect(),
        ),
        random_scores: quantiles(finite),
        random_best_of_budget: quantiles(maxes.clone()),
        percentile: (beaten + 0.5 * ties) / maxes.len() as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn space(narrow: Option<(f64, f64)>) -> SearchSpace {
        let def = serde_json::from_value(json!({
            "strategy_id": "s", "definition_version": "1.0", "asset_class": "x",
            "parameters": {
                "a": { "type": "float", "default": 0.5, "min": 0.0, "max": 1.0 },
                "b": { "type": "float", "default": 0.5, "min": 0.0, "max": 1.0 }
            },
            "inputs": [], "nodes": [], "actions": []
        }))
        .unwrap();
        let mut n = BTreeMap::new();
        if let Some((lo, hi)) = narrow {
            n.insert(
                "a".to_string(),
                crate::space::Narrow {
                    min: Some(lo),
                    max: Some(hi),
                    ..Default::default()
                },
            );
        }
        SearchSpace::from_definition(&def, &n).unwrap()
    }

    #[test]
    fn two_arms_draw_equal_trial_counts_and_the_seed_is_the_platforms() {
        let s = space(None);
        let pts = draw(&s, 40, arm_seed("camp-1"));
        assert_eq!(pts.len(), 40);
        assert_ne!(arm_seed("camp-1"), arm_seed("camp-2"));
        assert_eq!(
            draw(&s, 40, arm_seed("camp-1")),
            pts,
            "reproducible from the campaign id"
        );
    }

    #[test]
    fn the_arm_ignores_the_agents_narrowing_because_it_draws_the_declared_space() {
        // The agent narrowed `a` to [0.9, 1.0]. The arm is built from the declared space,
        // so its draws span the whole range: some fall below 0.9.
        let declared = space(None);
        let narrowed = space(Some((0.9, 1.0)));
        let arm_pts = draw(&declared, 200, arm_seed("c"));
        let agent_pts = draw(&narrowed, 200, 7);
        let low = |pts: &[ParamMap]| {
            pts.iter()
                .filter(|p| p["a"].as_f64().unwrap() < 0.9)
                .count()
        };
        assert!(low(&arm_pts) > 100, "declared space spans [0,1]");
        assert_eq!(low(&agent_pts), 0, "the agent's arm stays in its narrowing");
    }

    #[test]
    fn a_campaign_with_no_arm_or_an_incomplete_one_cannot_report_or_reach_a_verdict() {
        assert_eq!(gate_check(None), Err(ArmError::NoArm));
        let mut arm = Arm::new("c", 10);
        arm.scores = vec![0.1; 4];
        assert_eq!(
            gate_check(Some(&arm)),
            Err(ArmError::Incomplete {
                done: 4,
                budget: 10
            })
        );
        assert_eq!(compare_to_random(&[0.0; 10], None), Err(ArmError::NoArm));
        assert!(compare_to_random(&[0.0; 10], Some(&arm)).is_err());
        arm.scores = vec![0.1; 10];
        assert!(gate_check(Some(&arm)).is_ok());
        assert_eq!(
            compare_to_random(&[0.0; 7], Some(&arm)),
            Err(ArmError::UnequalBudget {
                agent: 7,
                random: 10
            })
        );
    }

    #[test]
    fn an_agent_that_only_matches_random_ranks_near_the_middle_and_a_real_edge_ranks_high() {
        // Under the null the percentile is roughly uniform, so its average over many
        // replications is near one half (the property), and no single replication is pinned
        // to 1.0 by construction (the defect a plain bootstrap had).
        let mut rng = Rng::new(99);
        let mut total = 0.0;
        let reps = 40;
        let mut at_one = 0;
        for r in 0..reps {
            let mut arm = Arm::new(&format!("c{r}"), 50);
            arm.scores = (0..50).map(|_| rng.next_normal()).collect();
            let same: Vec<f64> = (0..50).map(|_| rng.next_normal()).collect();
            let p = compare_to_random(&same, Some(&arm)).unwrap().percentile;
            total += p;
            if p > 0.999 {
                at_one += 1;
            }
        }
        let mean = total / f64::from(reps);
        assert!((0.35..0.65).contains(&mean), "mean null percentile {mean}");
        assert!(
            at_one < reps / 4,
            "{at_one} of {reps} null campaigns read as beating all of random search"
        );

        // An agent whose best is far above anything random found, and one far below.
        let mut arm = Arm::new("c-edge", 50);
        arm.scores = (0..50).map(|_| rng.next_normal()).collect();
        let mut edge: Vec<f64> = (0..50).map(|_| rng.next_normal()).collect();
        edge[0] = 10.0;
        let c = compare_to_random(&edge, Some(&arm)).unwrap();
        assert!(c.percentile > 0.99, "{}", c.percentile);
        let weak = vec![-10.0; 50];
        assert!(compare_to_random(&weak, Some(&arm)).unwrap().percentile < 0.01);
        // The report prints both distributions.
        assert!(c.random_best_of_budget.p50 > c.random_scores.p50);
    }

    #[test]
    fn the_percentile_is_reproducible() {
        let mut rng = Rng::new(5);
        let mut arm = Arm::new("c", 30);
        arm.scores = (0..30).map(|_| rng.next_normal()).collect();
        let agent: Vec<f64> = (0..30).map(|_| rng.next_normal()).collect();
        assert_eq!(
            compare_to_random(&agent, Some(&arm)).unwrap().percentile,
            compare_to_random(&agent, Some(&arm)).unwrap().percentile
        );
    }
}
