//! The procedure null and the sensitivity curve
//! (EXTERNAL-REVIEW-2026-09 §1 and §5.8; ADR-P6-10).
//!
//! ## What is being tested
//!
//! Not a strategy. The **procedure**: the proposer, its prompt and retrieval
//! memory, the search budget, both search levels, the gate stack and the
//! promotion rule. A candidate can be unseen while the process that generated it
//! has absorbed the evaluation period through thousands of rewards, and no
//! per-candidate statistic can see that.
//!
//! So the whole loop is run against worlds whose truth is known, and what is
//! measured is *how often it promotes*.
//!
//! - Against a **null world** — [`synthetic::Generator::GarchT`],
//!   `MertonJump`, `RegimeSwitch`, all with a conditional mean of exactly zero —
//!   every promotion is a false one. That gives the false-discovery floor:
//!   specificity.
//! - Against a **planted world** — `PlantedAr1`, `PlantedHourDrift`,
//!   `PlantedVolBreakout`, `PlantedCarry`, each carrying a `Truth` descriptor —
//!   a promotion that recovers the planted mechanism is a true one. That gives
//!   sensitivity, as a function of how faint the planted edge is.
//!
//! Together they are an ROC curve for the discovery procedure, which is the only
//! real measure of whether it works. A procedure that promotes nothing has a
//! perfect false-discovery floor and is worthless; one that promotes everything
//! finds every planted edge and is worse than worthless. Neither number means
//! anything alone, and [`Roc`] is the type that refuses to report one without
//! the other.
//!
//! ## What the numbers are allowed to claim
//!
//! An exceedance p-value from `B` null runs is valid under the exchangeability
//! its generator provides and says nothing about a misspecified live market.
//! Zero false promotions in 1,000 null runs bounds the rate at about 0.3% **in
//! that simulator**. [`FalseDiscoveryFloor::upper_95`] computes that bound
//! rather than letting a caller read "0 / 1000" as zero, because a rate of zero
//! is the one value the experiment cannot establish.
//!
//! ## Resetting between worlds
//!
//! [`WorldRun`] carries a fresh seed per world and the procedure must rebuild
//! its search state, features and memory from primitive data each time. A run
//! that carries retrieval memory from the real world into a null world has
//! measured nothing: the memory is where the contamination lives.

use crate::synthetic::Generator;
use serde::{Deserialize, Serialize};

/// One world the procedure was run against.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldRun {
    pub generator: Generator,
    /// The draw. Two runs of one generator at different seeds are two worlds.
    pub seed: u64,
    /// How many candidates the procedure promoted in this world.
    pub promotions: usize,
    /// Of those promotions, how many recovered the world's planted mechanism.
    /// Always 0 in a null world, where there is no mechanism to recover.
    pub true_recoveries: usize,
    /// Strength of the planted mechanism, in whatever units the generator uses.
    /// `None` for a null world.
    pub planted_strength: Option<f64>,
}

impl WorldRun {
    /// Whether this world had an exploitable mechanism.
    ///
    /// Read from the generator rather than from `planted_strength`, so that a
    /// caller who forgets to set the strength cannot silently turn a planted
    /// world into a null one and inflate the false-discovery floor.
    #[must_use]
    pub const fn is_null_world(&self) -> bool {
        matches!(
            self.generator,
            Generator::GarchT | Generator::MertonJump | Generator::RegimeSwitch
        )
    }

    /// Promotions that were not recoveries. In a null world this is all of them.
    #[must_use]
    pub const fn false_promotions(&self) -> usize {
        self.promotions.saturating_sub(self.true_recoveries)
    }
}

/// How often the procedure promotes when there is nothing to find.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FalseDiscoveryFloor {
    /// Null worlds run.
    pub worlds: usize,
    /// Null worlds in which at least one candidate was promoted.
    pub worlds_with_a_promotion: usize,
    /// Total false promotions, which can exceed `worlds_with_a_promotion` when a
    /// single null world yields several.
    pub total_false_promotions: usize,
}

impl FalseDiscoveryFloor {
    /// Measure the floor over null worlds. Planted worlds in `runs` are ignored.
    ///
    /// `None` when no null world was run — which is the honest answer, and is
    /// not the same as a floor of zero.
    #[must_use]
    pub fn measure(runs: &[WorldRun]) -> Option<Self> {
        let nulls: Vec<&WorldRun> = runs.iter().filter(|r| r.is_null_world()).collect();
        if nulls.is_empty() {
            return None;
        }
        Some(Self {
            worlds: nulls.len(),
            worlds_with_a_promotion: nulls.iter().filter(|r| r.promotions > 0).count(),
            total_false_promotions: nulls.iter().map(|r| r.promotions).sum(),
        })
    }

    /// Point estimate: the fraction of null worlds that produced a promotion.
    #[must_use]
    pub fn rate(&self) -> f64 {
        self.worlds_with_a_promotion as f64 / self.worlds as f64
    }

    /// One-sided 95% upper bound on that rate.
    ///
    /// With zero observed events this is the rule of three, `3/n` — the reason
    /// 1,000 clean null runs bound the rate at about 0.3% rather than at zero.
    /// With events observed it is the Clopper–Pearson upper limit, computed by
    /// bisection on the binomial tail, so the bound is exact rather than normal-
    /// approximate. The normal approximation is worst exactly here, at small
    /// counts, which is where every useful answer sits.
    #[must_use]
    pub fn upper_95(&self) -> f64 {
        let n = self.worlds;
        let k = self.worlds_with_a_promotion;
        if k == 0 {
            return (3.0 / n as f64).min(1.0);
        }
        if k >= n {
            return 1.0;
        }
        // P(X <= k-1 | p) = 0.05, solved for p. Monotone decreasing in p.
        let tail_at = |p: f64| -> f64 {
            let mut total = 0.0;
            let mut term = (1.0 - p).powi(i32::try_from(n).unwrap_or(i32::MAX));
            for i in 0..k {
                total += term;
                term = term * (n - i) as f64 / (i + 1) as f64 * p / (1.0 - p);
            }
            total
        };
        let (mut lo, mut hi) = (k as f64 / n as f64, 1.0);
        for _ in 0..80 {
            // `f64::midpoint` is 1.85+; the workspace MSRV is 1.82.
            let mid = (lo + hi) / 2.0;
            if tail_at(mid) > 0.05 {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    }

    /// Whether the floor is low enough to make any positive result interpretable
    /// at `alpha`. Uses the upper bound, not the point estimate: a floor of
    /// "0/20" reads as zero and bounds at 15%.
    #[must_use]
    pub fn is_interpretable_at(&self, alpha: f64) -> bool {
        self.upper_95() <= alpha
    }
}

/// Detection rate at one planted strength.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensitivityPoint {
    pub generator: Generator,
    pub strength: f64,
    pub worlds: usize,
    /// Worlds in which the procedure recovered the planted mechanism.
    pub recovered: usize,
    /// Promotions across those worlds that were *not* the planted mechanism —
    /// the false positives standing beside each true one. A procedure that finds
    /// the plant alongside nine spurious friends has not found it.
    pub spurious: usize,
}

impl SensitivityPoint {
    #[must_use]
    pub fn power(&self) -> f64 {
        self.recovered as f64 / self.worlds as f64
    }

    /// True recoveries as a fraction of everything promoted. `None` when
    /// nothing was promoted at all.
    #[must_use]
    pub fn precision(&self) -> Option<f64> {
        let total = self.recovered + self.spurious;
        (total > 0).then(|| self.recovered as f64 / total as f64)
    }
}

/// Specificity and sensitivity, together.
///
/// Constructed only through [`Roc::measure`], which needs both arms. Reporting
/// one without the other is the failure this type exists to prevent: a procedure
/// that promotes nothing has a perfect floor and no value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Roc {
    pub floor: FalseDiscoveryFloor,
    /// One point per `(generator, strength)`, ascending by strength.
    pub sensitivity: Vec<SensitivityPoint>,
}

impl Roc {
    /// Build the curve from a complete set of runs.
    ///
    /// # Errors
    /// - [`ProcedureNullError::NoNullWorlds`] when nothing measured specificity.
    /// - [`ProcedureNullError::NoPlantedWorlds`] when nothing measured sensitivity.
    pub fn measure(runs: &[WorldRun]) -> Result<Self, ProcedureNullError> {
        let floor = FalseDiscoveryFloor::measure(runs).ok_or(ProcedureNullError::NoNullWorlds)?;

        let mut buckets: std::collections::BTreeMap<(Generator, u64), Vec<&WorldRun>> =
            std::collections::BTreeMap::new();
        for r in runs.iter().filter(|r| !r.is_null_world()) {
            // Bucket by strength via its bit pattern so that f64 can key a map
            // without an epsilon that would silently merge two planted levels.
            let key = (
                r.generator,
                r.planted_strength.unwrap_or(f64::NAN).to_bits(),
            );
            buckets.entry(key).or_default().push(r);
        }
        if buckets.is_empty() {
            return Err(ProcedureNullError::NoPlantedWorlds);
        }

        let mut sensitivity: Vec<SensitivityPoint> = buckets
            .into_iter()
            .map(|((generator, bits), rs)| SensitivityPoint {
                generator,
                strength: f64::from_bits(bits),
                worlds: rs.len(),
                recovered: rs.iter().filter(|r| r.true_recoveries > 0).count(),
                spurious: rs.iter().map(|r| r.false_promotions()).sum(),
            })
            .collect();
        sensitivity.sort_by(|a, b| a.strength.total_cmp(&b.strength));

        Ok(Self { floor, sensitivity })
    }

    /// The faintest planted strength the procedure recovers at least
    /// `min_power` of the time **while its false-discovery floor is
    /// interpretable at `alpha`**.
    ///
    /// Both conditions, deliberately. A procedure that promotes everything
    /// recovers every plant and has detected nothing, and returning a detection
    /// limit for it would be the most misleading number this module could
    /// produce.
    #[must_use]
    pub fn detection_limit(&self, min_power: f64, alpha: f64) -> Option<f64> {
        self.floor
            .is_interpretable_at(alpha)
            .then(|| {
                self.sensitivity
                    .iter()
                    .find(|p| p.power() >= min_power)
                    .map(|p| p.strength)
            })
            .flatten()
    }
}

/// Monte Carlo exceedance p-value with the `+1` correction:
/// `(1 + #{Z_b ≥ Z_real}) / (B + 1)`.
///
/// The correction is not a rounding convenience. Without it a statistic that no
/// null draw exceeded reports `p = 0`, which claims more than `B` draws can
/// support; with it the floor is `1/(B+1)`, which is exactly what `B` draws can
/// support.
///
/// `None` when no null statistics were supplied.
#[must_use]
pub fn monte_carlo_p(observed: f64, null_draws: &[f64]) -> Option<f64> {
    if null_draws.is_empty() {
        return None;
    }
    let exceed = null_draws.iter().filter(|z| **z >= observed).count();
    Some((1 + exceed) as f64 / (null_draws.len() + 1) as f64)
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProcedureNullError {
    #[error(
        "no null worlds were run: without them there is no false-discovery floor, and a \
         detection rate on planted data alone cannot distinguish a procedure that finds edges \
         from one that promotes everything"
    )]
    NoNullWorlds,
    #[error(
        "no planted worlds were run: a clean false-discovery floor is also what a procedure that \
         promotes nothing produces, and the two are indistinguishable without a sensitivity arm"
    )]
    NoPlantedWorlds,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn null_world(seed: u64, promotions: usize) -> WorldRun {
        WorldRun {
            generator: Generator::GarchT,
            seed,
            promotions,
            true_recoveries: 0,
            planted_strength: None,
        }
    }

    fn planted(seed: u64, strength: f64, recovered: usize, promotions: usize) -> WorldRun {
        WorldRun {
            generator: Generator::PlantedAr1,
            seed,
            promotions,
            true_recoveries: recovered,
            planted_strength: Some(strength),
        }
    }

    // ── which worlds are null ────────────────────────────────────────────────

    /// Read from the generator, not from a field a caller can forget to set. A
    /// planted world misclassified as null deflates the false-discovery floor,
    /// which is the direction that flatters the procedure.
    #[test]
    fn null_worlds_are_identified_by_generator_not_by_a_missing_strength() {
        for g in [
            Generator::GarchT,
            Generator::MertonJump,
            Generator::RegimeSwitch,
        ] {
            assert!(WorldRun {
                generator: g,
                ..null_world(1, 0)
            }
            .is_null_world());
        }
        for g in [
            Generator::PlantedAr1,
            Generator::PlantedHourDrift,
            Generator::PlantedVolBreakout,
            Generator::PlantedCarry,
        ] {
            let mut r = planted(1, 0.1, 0, 0);
            r.generator = g;
            r.planted_strength = None; // caller forgot
            assert!(!r.is_null_world(), "{g:?} must stay a planted world");
        }
    }

    // ── the floor ────────────────────────────────────────────────────────────

    #[test]
    fn a_clean_run_of_a_thousand_null_worlds_bounds_the_rate_near_three_per_thousand() {
        let runs: Vec<WorldRun> = (0..1000).map(|i| null_world(i, 0)).collect();
        let floor = FalseDiscoveryFloor::measure(&runs).unwrap();
        assert!(floor.rate().abs() < f64::EPSILON);
        assert!((floor.upper_95() - 0.003).abs() < 1e-9, "rule of three");
        assert!(floor.is_interpretable_at(0.01));
    }

    /// The number that stops "0 out of 20" from reading as zero.
    #[test]
    fn twenty_clean_null_worlds_bound_the_rate_at_fifteen_percent() {
        let runs: Vec<WorldRun> = (0..20).map(|i| null_world(i, 0)).collect();
        let floor = FalseDiscoveryFloor::measure(&runs).unwrap();
        assert!(floor.rate().abs() < f64::EPSILON);
        assert!((floor.upper_95() - 0.15).abs() < 1e-9);
        assert!(
            !floor.is_interpretable_at(0.05),
            "a 15% bound interprets nothing at 5%"
        );
    }

    #[test]
    fn observed_false_promotions_give_a_clopper_pearson_bound_above_the_point_estimate() {
        let mut runs: Vec<WorldRun> = (0..100).map(|i| null_world(i, 0)).collect();
        for r in runs.iter_mut().take(5) {
            r.promotions = 1;
        }
        let floor = FalseDiscoveryFloor::measure(&runs).unwrap();
        assert!((floor.rate() - 0.05).abs() < 1e-9);
        let u = floor.upper_95();
        assert!(
            u > 0.05 && u < 0.12,
            "bound {u} should sit just above the estimate"
        );
    }

    /// One null world can yield several false promotions, and the two counts
    /// answer different questions.
    #[test]
    fn worlds_with_a_promotion_and_total_promotions_are_counted_separately() {
        let runs = vec![null_world(1, 3), null_world(2, 0), null_world(3, 1)];
        let floor = FalseDiscoveryFloor::measure(&runs).unwrap();
        assert_eq!(floor.worlds, 3);
        assert_eq!(floor.worlds_with_a_promotion, 2);
        assert_eq!(floor.total_false_promotions, 4);
    }

    #[test]
    fn a_floor_with_no_null_worlds_is_none_rather_than_zero() {
        assert!(FalseDiscoveryFloor::measure(&[]).is_none());
        assert!(FalseDiscoveryFloor::measure(&[planted(1, 0.1, 1, 1)]).is_none());
    }

    #[test]
    fn a_floor_of_one_is_returned_when_every_null_world_promoted() {
        let runs: Vec<WorldRun> = (0..10).map(|i| null_world(i, 1)).collect();
        let floor = FalseDiscoveryFloor::measure(&runs).unwrap();
        assert!((floor.rate() - 1.0).abs() < f64::EPSILON);
        assert!((floor.upper_95() - 1.0).abs() < f64::EPSILON);
    }

    // ── the ROC ──────────────────────────────────────────────────────────────

    /// Neither arm means anything alone, and the type refuses to be built from
    /// one of them.
    #[test]
    fn a_curve_needs_both_arms() {
        let nulls: Vec<WorldRun> = (0..5).map(|i| null_world(i, 0)).collect();
        assert_eq!(
            Roc::measure(&nulls).unwrap_err(),
            ProcedureNullError::NoPlantedWorlds
        );

        let planted_only = vec![planted(1, 0.1, 1, 1)];
        assert_eq!(
            Roc::measure(&planted_only).unwrap_err(),
            ProcedureNullError::NoNullWorlds
        );
    }

    #[test]
    fn sensitivity_points_are_grouped_by_strength_and_sorted() {
        let mut runs: Vec<WorldRun> = (0..50).map(|i| null_world(i, 0)).collect();
        for i in 0..10 {
            runs.push(planted(100 + i, 0.20, 1, 1));
            runs.push(planted(200 + i, 0.05, u8::from(i < 3).into(), 1));
        }
        let roc = Roc::measure(&runs).unwrap();
        assert_eq!(roc.sensitivity.len(), 2);
        assert!(
            (roc.sensitivity[0].strength - 0.05).abs() < 1e-12,
            "ascending by strength"
        );
        assert!((roc.sensitivity[0].power() - 0.3).abs() < 1e-9);
        assert!((roc.sensitivity[1].power() - 1.0).abs() < 1e-9);
    }

    /// The detection limit is the faintest edge found reliably — but only if the
    /// floor is clean enough for "found" to mean anything.
    #[test]
    fn the_detection_limit_is_the_faintest_strength_found_reliably() {
        let mut runs: Vec<WorldRun> = (0..1000).map(|i| null_world(i, 0)).collect();
        for i in 0..20 {
            runs.push(planted(1_000 + i, 0.02, u8::from(i < 2).into(), 1)); // 10% power
            runs.push(planted(2_000 + i, 0.10, u8::from(i < 18).into(), 1)); // 90% power
        }
        let roc = Roc::measure(&runs).unwrap();
        assert_eq!(roc.detection_limit(0.8, 0.01), Some(0.10));
        assert_eq!(roc.detection_limit(0.05, 0.01), Some(0.02));
    }

    /// The most misleading number this module could produce: a procedure that
    /// promotes everything recovers every plant. Its detection limit is None.
    #[test]
    fn a_procedure_that_promotes_everything_has_no_detection_limit() {
        let mut runs: Vec<WorldRun> = (0..100).map(|i| null_world(i, 1)).collect();
        for i in 0..20 {
            runs.push(planted(1_000 + i, 0.01, 1, 5));
        }
        let roc = Roc::measure(&runs).unwrap();
        assert!(
            (roc.sensitivity[0].power() - 1.0).abs() < 1e-9,
            "it finds every plant"
        );
        assert_eq!(
            roc.detection_limit(0.8, 0.05),
            None,
            "and has detected nothing"
        );
    }

    /// Finding the plant alongside nine spurious friends is not finding it.
    #[test]
    fn precision_counts_the_false_positives_standing_beside_each_true_one() {
        let mut runs: Vec<WorldRun> = (0..50).map(|i| null_world(i, 0)).collect();
        runs.push(planted(999, 0.1, 1, 10));
        let roc = Roc::measure(&runs).unwrap();
        let p = &roc.sensitivity[0];
        assert_eq!(p.recovered, 1);
        assert_eq!(p.spurious, 9);
        assert!((p.precision().unwrap() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn precision_is_none_when_nothing_was_promoted() {
        let mut runs: Vec<WorldRun> = (0..50).map(|i| null_world(i, 0)).collect();
        runs.push(planted(999, 0.1, 0, 0));
        let roc = Roc::measure(&runs).unwrap();
        assert!(roc.sensitivity[0].precision().is_none());
    }

    // ── the exceedance estimator ─────────────────────────────────────────────

    /// The `+1` is not a rounding convenience: without it an unexceeded
    /// statistic reports `p = 0`, which claims more than B draws can support.
    #[test]
    fn the_exceedance_p_value_floors_at_one_over_b_plus_one() {
        let nulls: Vec<f64> = (0..999).map(|i| f64::from(i) / 1000.0).collect();
        let p = monte_carlo_p(10.0, &nulls).unwrap();
        assert!((p - 1.0 / 1000.0).abs() < 1e-12);
        assert!(p > 0.0, "B draws cannot establish a rate of zero");
    }

    #[test]
    fn a_statistic_every_null_beats_gives_a_p_value_of_one() {
        let nulls = vec![1.0, 2.0, 3.0];
        assert!((monte_carlo_p(0.0, &nulls).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn ties_count_as_exceedances() {
        assert!((monte_carlo_p(1.0, &[1.0]).unwrap() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn no_null_draws_gives_no_p_value() {
        assert!(monte_carlo_p(1.0, &[]).is_none());
    }

    #[test]
    fn a_run_report_round_trips_through_json() {
        let mut runs: Vec<WorldRun> = (0..5).map(|i| null_world(i, 0)).collect();
        runs.push(planted(9, 0.1, 1, 1));
        let roc = Roc::measure(&runs).unwrap();
        let s = serde_json::to_string(&roc).unwrap();
        assert_eq!(serde_json::from_str::<Roc>(&s).unwrap(), roc);
    }
}
