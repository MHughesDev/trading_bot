//! The theme-posterior clause of the significance gate (Set Q Q-2.1; BS-007 11 EV-07).
//!
//! A candidate whose raw mean return is the best of a family has a mean biased
//! upward by exactly what selection rewarded. Deflating the Sharpe corrects the
//! *significance*; this corrects the *estimate*: it shrinks the candidate's mean
//! toward the family's (the "theme") by empirical Bayes and asks whether the
//! credible interval still excludes zero on the positive side. A candidate that
//! needs the winner's curse to be positive is not positive.
//!
//! This is the same arithmetic as the `posterior_alpha` template
//! (`apps/skill-runner/templates/attribution/posterior_alpha.py`), a normal-normal
//! model with a method-of-moments between-candidate variance and Newey-West
//! standard errors, and `matches_the_template_on_a_fixed_family` pins the two
//! to one answer. The template is what an agent reads *before* spending trials;
//! this is what the gate applies *after* them, and it must not disagree.
//!
//! # What the clause does to a gate
//!
//! It only ever **tightens** one: it can turn a pass into a fail naming the
//! interval, and it cannot turn a fail or an inconclusive into a pass. When the
//! family is too small to have a theme (fewer than [`MIN_MEMBERS`] usable
//! members) the clause is *not applicable* and says so in the verdict's detail;
//! it is not a pass of the clause and it is not a failure of the gate.

use crate::stats::inverse_normal_cdf;

use super::evaluators::GateOutcome;

/// The credible level of the interval the gate reads.
pub const CREDIBLE: f64 = 0.95;
/// A member with fewer finite periods is left out of the family: its mean is not
/// estimated well enough to be shrunk or to shrink others.
pub const MIN_PERIODS: usize = 60;
/// Below three usable members the between-candidate spread is not estimable.
pub const MIN_MEMBERS: usize = 3;

/// One candidate's posterior.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemePosterior {
    pub n_candidates: usize,
    /// The theme mean the candidate was shrunk toward, per period.
    pub prior_mean: f64,
    pub between_sd: f64,
    pub raw_mean: f64,
    /// The weight on the theme: 1 is "collapsed onto it".
    pub shrinkage: f64,
    pub post_mean: f64,
    pub ci_lo: f64,
    pub ci_hi: f64,
}

impl ThemePosterior {
    /// The interval is entirely above zero.
    #[must_use]
    pub fn excludes_zero_positive(&self) -> bool {
        self.ci_lo > 0.0
    }
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PosteriorError {
    #[error(
        "{usable} of the family have {MIN_PERIODS} or more periods and shrinkage toward a family \
         needs {MIN_MEMBERS}; with fewer the between-candidate spread is not estimable"
    )]
    FamilyTooSmall { usable: usize },
    #[error("candidate {0} is not a usable member of the family (out of range or under {MIN_PERIODS} periods)")]
    CandidateNotUsable(usize),
    #[error("a member's standard error is not positive and finite, so it cannot be shrunk")]
    BadStandardError,
}

/// HAC standard error of the mean (Bartlett kernel, lag `floor(4 (n/100)^(2/9))`).
///
/// The same rule as `newey_west_se` in the template preamble. `None` under three
/// observations.
#[must_use]
pub fn newey_west_se(x: &[f64]) -> Option<f64> {
    let x: Vec<f64> = x.iter().copied().filter(|v| v.is_finite()).collect();
    let n = x.len();
    if n < 3 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let nf = n as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let lags = ((4.0 * (nf / 100.0).powf(2.0 / 9.0)).floor() as usize).min(n - 2);
    let mean = x.iter().sum::<f64>() / nf;
    let e: Vec<f64> = x.iter().map(|v| v - mean).collect();
    let gamma = |lag: usize| {
        e[..n - lag]
            .iter()
            .zip(&e[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / nf
    };
    let mut total = gamma(0);
    for lag in 1..=lags {
        #[allow(clippy::cast_precision_loss)]
        let w = 1.0 - lag as f64 / (lags as f64 + 1.0);
        total += 2.0 * w * gamma(lag);
    }
    Some((total.max(0.0) / nf).sqrt())
}

/// The candidate's posterior within its family.
///
/// `candidate` indexes `family` as the caller holds it; members under
/// [`MIN_PERIODS`] finite periods are dropped before anything is estimated.
///
/// # Errors
/// A family too small to have a theme, a candidate that is not usable, or a
/// member whose error is not positive.
pub fn theme_posterior(
    family: &[Vec<f64>],
    candidate: usize,
) -> Result<ThemePosterior, PosteriorError> {
    let mut means = Vec::new();
    let mut ses = Vec::new();
    let mut at = None;
    for (i, series) in family.iter().enumerate() {
        let finite: Vec<f64> = series.iter().copied().filter(|v| v.is_finite()).collect();
        if finite.len() < MIN_PERIODS {
            continue;
        }
        let Some(se) = newey_west_se(&finite) else {
            continue;
        };
        if i == candidate {
            at = Some(means.len());
        }
        #[allow(clippy::cast_precision_loss)]
        means.push(finite.iter().sum::<f64>() / finite.len() as f64);
        ses.push(se);
    }
    let usable = means.len();
    if usable < MIN_MEMBERS {
        return Err(PosteriorError::FamilyTooSmall { usable });
    }
    let i = at.ok_or(PosteriorError::CandidateNotUsable(candidate))?;
    if ses.iter().any(|s| !s.is_finite() || *s <= 0.0) {
        return Err(PosteriorError::BadStandardError);
    }

    #[allow(clippy::cast_precision_loss)]
    let m = usable as f64;
    let mean_of_means = means.iter().sum::<f64>() / m;
    let var_means = means
        .iter()
        .map(|x| (x - mean_of_means).powi(2))
        .sum::<f64>()
        / (m - 1.0);
    let mean_se2 = ses.iter().map(|s| s * s).sum::<f64>() / m;
    let tau2 = (var_means - mean_se2).max(0.0);
    let w: Vec<f64> = ses.iter().map(|s| 1.0 / (s * s + tau2)).collect();
    let wsum: f64 = w.iter().sum();
    let mu0 = w.iter().zip(&means).map(|(w, x)| w * x).sum::<f64>() / wsum;
    let var_mu0 = 1.0 / wsum;

    let b = ses[i] * ses[i] / (ses[i] * ses[i] + tau2);
    let post_mean = b * mu0 + (1.0 - b) * means[i];
    let post_var = (1.0 - b) * ses[i] * ses[i] + b * b * var_mu0;
    let half = inverse_normal_cdf(0.5 + CREDIBLE / 2.0) * post_var.sqrt();
    Ok(ThemePosterior {
        n_candidates: usable,
        prior_mean: mu0,
        between_sd: tau2.sqrt(),
        raw_mean: means[i],
        shrinkage: b,
        post_mean,
        ci_lo: post_mean - half,
        ci_hi: post_mean + half,
    })
}

/// What the clause says about a candidate.
#[derive(Clone, Debug, PartialEq)]
pub enum Clause {
    /// No theme to shrink toward; the reason. Neither a pass nor a failure.
    NotApplicable(String),
    Holds(ThemePosterior),
    Fails(ThemePosterior),
}

#[must_use]
pub fn clause(family: &[Vec<f64>], candidate: usize) -> Clause {
    match theme_posterior(family, candidate) {
        Ok(p) if p.excludes_zero_positive() => Clause::Holds(p),
        Ok(p) => Clause::Fails(p),
        Err(e) => Clause::NotApplicable(e.to_string()),
    }
}

/// Apply the clause to a gate's outcome (the deflated-Sharpe gate, in the stack).
///
/// Only ever tightens: a pass whose candidate's interval includes zero becomes a
/// failure naming the interval; a fail stays a fail and an inconclusive stays
/// inconclusive, and both gain the reading in their detail.
#[must_use]
pub fn apply(mut outcome: GateOutcome, family: &[Vec<f64>], candidate: usize) -> GateOutcome {
    match clause(family, candidate) {
        Clause::NotApplicable(why) => {
            outcome.detail = format!("{}; theme posterior not applied: {why}", outcome.detail);
        }
        Clause::Holds(p) => {
            outcome.detail = format!(
                "{}; theme posterior {:.6} [{:.6}, {:.6}] excludes zero (shrunk from raw {:.6} \
                 toward the family's {:.6})",
                outcome.detail, p.post_mean, p.ci_lo, p.ci_hi, p.raw_mean, p.prior_mean
            );
        }
        Clause::Fails(p) => {
            outcome.detail = format!(
                "{}; theme posterior {:.6} [{:.6}, {:.6}] includes zero or is negative (raw mean \
                 {:.6} shrunk toward the family's {:.6} across {} candidates): the edge does not \
                 survive the winner's curse",
                outcome.detail,
                p.post_mean,
                p.ci_lo,
                p.ci_hi,
                p.raw_mean,
                p.prior_mean,
                p.n_candidates
            );
            outcome.passed = false;
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic member: a sinusoid of period `~9` around a per-member mean, so a test
    /// can plant structure without a generator and the Python template can be fed the same.
    fn member(mean: f64, phase: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|t| {
                #[allow(clippy::cast_precision_loss)]
                let t = t as f64;
                mean + 0.01 * (0.7 * t + phase).sin() + 0.004 * (1.9 * t + 2.0 * phase).cos()
            })
            .collect()
    }

    fn family(means: &[f64], n: usize) -> Vec<Vec<f64>> {
        means
            .iter()
            .enumerate()
            .map(|(j, m)| {
                #[allow(clippy::cast_precision_loss)]
                member(*m, j as f64, n)
            })
            .collect()
    }

    #[test]
    fn matches_the_template_on_a_fixed_family() {
        // Golden values from `posterior_alpha.py` run on the same family (2026-10-04): means
        // 0, 0.0004, -0.0003, 0.0012 and 0.0001; 400 periods; candidate r3.
        let fam = family(&[0.0, 0.0004, -0.0003, 0.0012, 0.0001], 400);
        let p = theme_posterior(&fam, 3).unwrap();
        assert_eq!(p.n_candidates, 5);
        assert!(
            (p.raw_mean - 0.001_131_236_473).abs() < 1e-11,
            "raw {}",
            p.raw_mean
        );
        assert!(
            (p.prior_mean - GOLDEN.0).abs() < 1e-9,
            "prior {}",
            p.prior_mean
        );
        assert!(
            (p.post_mean - GOLDEN.1).abs() < 1e-9,
            "post {}",
            p.post_mean
        );
        assert!((p.ci_lo - GOLDEN.2).abs() < 1e-8, "lo {}", p.ci_lo);
        assert!((p.ci_hi - GOLDEN.3).abs() < 1e-8, "hi {}", p.ci_hi);
    }

    /// (prior_mean, post_mean, ci_lo, ci_hi) from the Python template.
    const GOLDEN: (f64, f64, f64, f64) = (
        0.000_273_759_759_902,
        0.000_746_582_903_559,
        0.000_167_690_616_553,
        0.001_325_475_190_57,
    );

    #[test]
    fn a_planted_winner_among_noise_is_pulled_toward_the_theme_and_never_past_it() {
        let mut fam = family(
            &[0.0, 0.0001, -0.0001, 0.00005, 0.0, -0.00005, 0.0001, 0.0],
            300,
        );
        fam[2] = member(0.0006, 2.0, 300);
        let p = theme_posterior(&fam, 2).unwrap();
        let (lo, hi) = if p.prior_mean < p.raw_mean {
            (p.prior_mean, p.raw_mean)
        } else {
            (p.raw_mean, p.prior_mean)
        };
        assert!(p.post_mean >= lo - 1e-15 && p.post_mean <= hi + 1e-15);
        assert!((p.post_mean - p.prior_mean).abs() <= (p.raw_mean - p.prior_mean).abs());
    }

    #[test]
    fn a_family_too_small_for_a_theme_is_not_applicable_rather_than_failing() {
        let fam = family(&[0.001, 0.0], 300);
        assert!(matches!(
            theme_posterior(&fam, 0),
            Err(PosteriorError::FamilyTooSmall { usable: 2 })
        ));
        assert!(matches!(clause(&fam, 0), Clause::NotApplicable(_)));
    }

    #[test]
    fn short_members_are_left_out_and_a_short_candidate_is_not_usable() {
        let mut fam = family(&[0.0, 0.0001, -0.0001, 0.0002], 300);
        fam.push(member(0.01, 0.0, MIN_PERIODS - 1));
        let p = theme_posterior(&fam, 0).unwrap();
        assert_eq!(
            p.n_candidates, 4,
            "the 59-period member did not enter the family"
        );
        assert!(matches!(
            theme_posterior(&fam, 4),
            Err(PosteriorError::CandidateNotUsable(4))
        ));
    }

    fn passing_gate() -> GateOutcome {
        GateOutcome::decided_pub(
            8,
            "deflated_sharpe",
            true,
            0.97,
            0.95,
            "deflated Sharpe ratio 0.9700 (needs ≥ 0.9500)",
        )
    }

    #[test]
    fn the_clause_can_fail_a_passing_gate_naming_the_interval_and_never_rescues_a_failing_one() {
        // Noise everywhere: the best member's shrunk interval includes zero.
        let fam = family(
            &[0.0001, 0.0, -0.0001, 0.00005, 0.0, -0.00005, 0.00002, 0.0],
            300,
        );
        let failed = apply(passing_gate(), &fam, 0);
        assert!(!failed.passed);
        assert!(failed.detail.contains("includes zero"), "{}", failed.detail);
        assert!(
            failed.detail.contains("0.9700"),
            "the original DSR reading is kept: {}",
            failed.detail
        );

        let mut already_failing = passing_gate();
        already_failing.passed = false;
        assert!(!apply(already_failing, &fam, 0).passed);

        let mut inconclusive = GateOutcome::inconclusive_pub(
            8,
            "deflated_sharpe",
            "no deflated Sharpe ratio was computed",
        );
        inconclusive = apply(inconclusive, &fam, 0);
        assert!(inconclusive.inconclusive && !inconclusive.passed);
    }

    #[test]
    fn a_not_applicable_clause_leaves_the_gate_as_it_was_and_says_so() {
        let fam = family(&[0.001, 0.0], 300);
        let out = apply(passing_gate(), &fam, 0);
        assert!(out.passed, "DSR alone decides when there is no theme");
        assert!(
            out.detail.contains("theme posterior not applied"),
            "{}",
            out.detail
        );
    }

    #[test]
    fn a_clear_edge_in_a_family_that_varies_keeps_its_interval_above_zero() {
        // Candidate 0 earns far more than the noise, with the others spread around zero.
        let fam = family(&[0.004, 0.0003, -0.0002, 0.0001, -0.0003, 0.0002], 600);
        assert!(matches!(clause(&fam, 0), Clause::Holds(_)));
        assert!(apply(passing_gate(), &fam, 0).passed);
    }
}
