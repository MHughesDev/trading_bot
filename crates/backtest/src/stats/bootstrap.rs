//! Gates 13 and 14: the stationary bootstrap and Romano–Wolf stepdown
//! (SPEC §12.3; plan 2.14, ADR-P2-17).
//!
//! Both are **statistics over return series the ledger already stores** (INV-18),
//! not Studies. Nothing here dispatches a Run, so nothing here adds to the trial
//! counter — a statistic computed from an existing look is not a new look, and
//! counting it as one would deflate the platform against work it never did.
//!
//! ## Why these two
//!
//! Gate 13 asks whether the result survives *a different ordering of the same
//! history*: path dependence is the failure a single realized path cannot show
//! you. Gate 14 asks which candidates are genuinely good given that you generated
//! a whole family of them — and it is the most defensible single gate in the
//! spec, because it uses the actual correlation structure of the family rather
//! than a guessed independent count. A 200-point sweep over one idea is correctly
//! not penalized as 200 independent tests.

use crate::rng::DetRng;

/// §12.3 Gate 13: "≥ 1000 resamples".
pub const MIN_BOOTSTRAP_RESAMPLES: usize = 1_000;

/// §12.3 Gate 14: "B ≥ 5000 bootstrap replications".
pub const MIN_STEPDOWN_RESAMPLES: usize = 5_000;

/// Below this many observations, neither the block-length estimate nor the
/// percentile has enough to stand on, and the gate must say so rather than
/// return a number.
const MIN_OBSERVATIONS: usize = 30;

/// Why a bootstrap gate could not produce a verdict.
///
/// Refusing is the point: a 5th-percentile Sharpe from twelve observations is a
/// number, and it is not evidence.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BootstrapError {
    #[error("need at least {MIN_OBSERVATIONS} observations for a stationary bootstrap, have {have}")]
    TooShort { have: usize },
    #[error("need at least {min} resamples for this gate, asked for {asked}")]
    TooFewResamples { min: usize, asked: usize },
    #[error("the return series is constant; a Sharpe ratio is undefined")]
    ZeroVariance,
    #[error("candidate {0} has a different number of observations from the family")]
    RaggedFamily(usize),
    #[error("a family test needs at least two candidates")]
    FamilyTooSmall,
}

// ───────────────────────────────────────────────────────────────────────────────
// the stationary bootstrap (Politis–Romano 1994)
// ───────────────────────────────────────────────────────────────────────────────

/// One resample's index path.
///
/// The stationary bootstrap glues together geometric-length blocks: at each step
/// it continues the current block with probability `1 − q` and starts a new one
/// at a uniformly random position with probability `q`. Expected block length is
/// `1/q`. Wrapping makes the resampled series stationary, which is what lets the
/// percentile be read as a distribution rather than an artefact of where the
/// blocks happened to fall.
#[must_use]
pub fn stationary_indices(n: usize, mean_block: f64, rng: &mut DetRng) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let q = (1.0 / mean_block.max(1.0)).clamp(f64::EPSILON, 1.0);
    let mut idx = Vec::with_capacity(n);
    let mut cur = rng.below(n);
    for _ in 0..n {
        idx.push(cur);
        if rng.next_f64() < q {
            cur = rng.below(n);
        } else {
            cur = (cur + 1) % n;
        }
    }
    idx
}

/// Politis–White (2004) automatic block length for the stationary bootstrap.
///
/// The alternative is a round number, and a round number is wrong in a specific
/// direction: too short destroys the autocorrelation the null is supposed to
/// preserve, which makes every result look more significant than it is. The
/// estimate is `b = (Ĝ/ĝ(0))^(2/3) · n^(1/3)`, with `Ĝ` and `ĝ(0)` flat-top
/// kernel estimates of the spectral density and its first derivative at zero.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    // The names here are Politis and White's: `n`, `m`, `M`, `b`, `lambda`,
    // `G` and `g(0)`. Renaming them to something clippy prefers would make the
    // code harder, not easier, to check against the paper.
    clippy::many_single_char_names
)]
pub fn politis_white_block(x: &[f64]) -> f64 {
    let n = x.len();
    if n < MIN_OBSERVATIONS {
        return 1.0;
    }
    let nf = n as f64;
    let mean = x.iter().sum::<f64>() / nf;
    let c0 = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / nf;
    if c0 <= 0.0 {
        return 1.0;
    }
    let rho = |k: usize| -> f64 {
        if k >= n {
            return 0.0;
        }
        let s: f64 = (0..n - k).map(|i| (x[i] - mean) * (x[i + k] - mean)).sum();
        s / nf / c0
    };

    // The lag beyond which the autocorrelation is indistinguishable from zero.
    // Politis–White: the smallest m whose next K_N lags are all insignificant.
    let k_n = (5.0 * (nf.log10()).sqrt()).ceil().max(5.0) as usize;
    let bound = 2.0 * (nf.log10() / nf).sqrt();
    let max_lag = (nf.sqrt().ceil() as usize + k_n).min(n - 1);
    let mut m = 0usize;
    for cand in 1..=max_lag {
        let all_small = (1..=k_n)
            .map(|j| cand + j)
            .take_while(|l| *l <= max_lag)
            .all(|l| rho(l).abs() < bound);
        if all_small {
            m = cand;
            break;
        }
    }
    if m == 0 {
        m = max_lag.min((nf.sqrt().ceil() as usize).max(1));
    }
    let big_m = (2 * m).min(n - 1);

    // Flat-top (trapezoidal) kernel: 1 on [0, 1/2], tapering to 0 at 1.
    let lambda = |t: f64| -> f64 {
        let a = t.abs();
        if a <= 0.5 {
            1.0
        } else if a <= 1.0 {
            2.0 * (1.0 - a)
        } else {
            0.0
        }
    };

    // `derivative` is Politis-White's Ĝ, `spectrum` their ĝ(0).
    let mut derivative = 0.0;
    let mut spectrum = rho(0); // = 1
    for k in 1..=big_m {
        let w = lambda(k as f64 / big_m as f64);
        let r = rho(k);
        spectrum += 2.0 * w * r;
        derivative += 2.0 * w * (k as f64) * r;
    }
    if spectrum.abs() < 1e-12 {
        return 1.0;
    }
    let b = (derivative.abs() / spectrum.abs()).powf(2.0 / 3.0) * nf.powf(1.0 / 3.0);
    // A block longer than the sample cannot be resampled from, and a block below
    // one is not a block.
    b.clamp(1.0, (nf / 3.0).max(1.0))
}

/// Gate 13's product: the bootstrap distribution of the Sharpe ratio and the
/// percentile the gate reads.
#[derive(Clone, Debug, PartialEq)]
pub struct BootstrapSharpe {
    /// Sharpe of the realized path.
    pub observed: f64,
    /// 5th percentile of the bootstrap distribution — what §12.3 gates on.
    pub p05: f64,
    pub median: f64,
    pub resamples: usize,
    /// The block length Politis–White chose, reported so the verdict can be
    /// re-derived and argued with.
    pub mean_block: f64,
}

/// Gate 13 — stationary-bootstrap Sharpe distribution over one return series.
///
/// # Errors
/// A series too short to bootstrap, too few resamples, or a constant series.
pub fn bootstrap_sharpe(
    returns: &[f64],
    resamples: usize,
    seed: u64,
) -> Result<BootstrapSharpe, BootstrapError> {
    if returns.len() < MIN_OBSERVATIONS {
        return Err(BootstrapError::TooShort { have: returns.len() });
    }
    if resamples < MIN_BOOTSTRAP_RESAMPLES {
        return Err(BootstrapError::TooFewResamples {
            min: MIN_BOOTSTRAP_RESAMPLES,
            asked: resamples,
        });
    }
    let observed = sharpe(returns).ok_or(BootstrapError::ZeroVariance)?;
    let mean_block = politis_white_block(returns);
    let mut rng = DetRng::new(seed);
    let mut draws: Vec<f64> = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let idx = stationary_indices(returns.len(), mean_block, &mut rng);
        let sample: Vec<f64> = idx.iter().map(|&i| returns[i]).collect();
        if let Some(s) = sharpe(&sample) {
            draws.push(s);
        }
    }
    if draws.len() < resamples / 2 {
        return Err(BootstrapError::ZeroVariance);
    }
    draws.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(BootstrapSharpe {
        observed,
        p05: percentile(&draws, 0.05),
        median: percentile(&draws, 0.50),
        resamples: draws.len(),
        mean_block,
    })
}

// ───────────────────────────────────────────────────────────────────────────────
// Gate 14 — Romano–Wolf stepdown
// ───────────────────────────────────────────────────────────────────────────────

/// One candidate's verdict in the family test.
#[derive(Clone, Debug, PartialEq)]
pub struct StepdownVerdict {
    /// Index into the family as supplied.
    pub index: usize,
    /// Studentized mean return: `mean / (sd / √n)`.
    pub t_stat: f64,
    /// The stepdown-adjusted p-value. Monotone in the rejection order by
    /// construction, so a candidate can never be reported as more significant
    /// than one rejected before it.
    pub adjusted_p: f64,
    pub rejected: bool,
}

/// The whole family's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Stepdown {
    pub verdicts: Vec<StepdownVerdict>,
    pub resamples: usize,
    pub mean_block: f64,
    pub alpha: f64,
}

impl Stepdown {
    /// The verdict for one candidate.
    #[must_use]
    pub fn for_index(&self, index: usize) -> Option<&StepdownVerdict> {
        self.verdicts.iter().find(|v| v.index == index)
    }

    #[must_use]
    pub fn rejected_count(&self) -> usize {
        self.verdicts.iter().filter(|v| v.rejected).count()
    }
}

/// Gate 14 — Romano–Wolf stepdown over a family of return series.
///
/// `family[i]` is candidate `i`'s out-of-sample return series; all must be the
/// same length and **aligned in time**, because the whole value of this test is
/// that it resamples the *same* time indices across every candidate and so
/// preserves their cross-sectional correlation. That is why a sweep of 200
/// correlated variants is not penalized like 200 independent tests.
///
/// The procedure (§6.5 of the financial-ML reference):
/// 1. studentize each candidate, sort descending;
/// 2. stationary-bootstrap the *joint* null of `max_k |t*_k|` over the active set,
///    centering each candidate on its own observed mean;
/// 3. reject the leader if it exceeds the `1−α` quantile of that max;
/// 4. drop the rejected, recompute the max over what remains, repeat;
/// 5. stop at the first non-rejection.
///
/// # Errors
/// A family smaller than two, ragged series, series too short, or too few
/// resamples.
pub fn romano_wolf(
    family: &[Vec<f64>],
    alpha: f64,
    resamples: usize,
    seed: u64,
) -> Result<Stepdown, BootstrapError> {
    if family.len() < 2 {
        return Err(BootstrapError::FamilyTooSmall);
    }
    if resamples < MIN_STEPDOWN_RESAMPLES {
        return Err(BootstrapError::TooFewResamples {
            min: MIN_STEPDOWN_RESAMPLES,
            asked: resamples,
        });
    }
    let n = family[0].len();
    if n < MIN_OBSERVATIONS {
        return Err(BootstrapError::TooShort { have: n });
    }
    for (i, f) in family.iter().enumerate() {
        if f.len() != n {
            return Err(BootstrapError::RaggedFamily(i));
        }
    }

    let k = family.len();
    let stats: Vec<f64> = family.iter().map(|f| t_stat(f).unwrap_or(0.0)).collect();
    let means: Vec<f64> = family.iter().map(|f| mean(f)).collect();

    // The block length is chosen from the family's average autocorrelation, so a
    // single unusually smooth candidate cannot set the block for everyone.
    let mean_block = {
        let pooled: Vec<f64> = (0..n)
            .map(|t| family.iter().map(|f| f[t]).sum::<f64>() / k as f64)
            .collect();
        politis_white_block(&pooled)
    };

    // One set of index paths, shared across candidates and across steps: the
    // cross-sectional correlation is the thing being preserved, and re-drawing
    // per candidate would destroy exactly that.
    let mut rng = DetRng::new(seed);
    let paths: Vec<Vec<usize>> = (0..resamples)
        .map(|_| stationary_indices(n, mean_block, &mut rng))
        .collect();

    // Centered bootstrap t-statistics: `boot[b][i]`.
    let mut boot: Vec<Vec<f64>> = Vec::with_capacity(resamples);
    for path in &paths {
        let mut row = Vec::with_capacity(k);
        for (i, f) in family.iter().enumerate() {
            let sample: Vec<f64> = path.iter().map(|&t| f[t]).collect();
            // Center on the observed mean so the resample is a draw from the null.
            let centered: Vec<f64> = sample.iter().map(|v| v - means[i]).collect();
            row.push(t_stat(&centered).unwrap_or(0.0));
        }
        boot.push(row);
    }

    // Descending by observed statistic — the stepdown order.
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by(|a, b| stats[*b].partial_cmp(&stats[*a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut active: Vec<usize> = order.clone();
    let mut verdicts: Vec<StepdownVerdict> = Vec::with_capacity(k);
    let mut prev_p = 0.0_f64;

    while !active.is_empty() {
        // The joint null of the maximum over what is still in contention.
        let maxima: Vec<f64> = boot
            .iter()
            .map(|row| {
                active
                    .iter()
                    .map(|&i| row[i].abs())
                    .fold(f64::NEG_INFINITY, f64::max)
            })
            .collect();
        let leader = active[0];
        let t = stats[leader];
        // p = P(max |t*| ≥ |t_observed|), the natural stepdown-adjusted p-value.
        #[allow(clippy::cast_precision_loss)]
        let p = {
            let exceed = maxima.iter().filter(|m| **m >= t.abs()).count();
            (exceed as f64 + 1.0) / (maxima.len() as f64 + 1.0)
        };
        // Monotone by construction: a candidate cannot be reported as more
        // significant than one that was rejected ahead of it.
        let adjusted_p = p.max(prev_p);
        prev_p = adjusted_p;
        let rejected = adjusted_p < alpha;

        verdicts.push(StepdownVerdict {
            index: leader,
            t_stat: t,
            adjusted_p,
            rejected,
        });
        active.remove(0);

        if !rejected {
            // Everything below the first non-rejection keeps its p-value: the
            // procedure stops rejecting, it does not stop reporting.
            for &i in &active {
                verdicts.push(StepdownVerdict {
                    index: i,
                    t_stat: stats[i],
                    adjusted_p,
                    rejected: false,
                });
            }
            break;
        }
    }

    Ok(Stepdown {
        verdicts,
        resamples,
        mean_block,
        alpha,
    })
}

// ───────────────────────────────────────────────────────────────────────────────
// small shared statistics
// ───────────────────────────────────────────────────────────────────────────────

fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f64>() / x.len() as f64
    }
}

/// Sample standard deviation, or `None` when the series has no *measurable*
/// dispersion.
///
/// The bound is relative, not `> 0.0`, because a constant series does not have a
/// sample standard deviation of exactly zero: summing two hundred copies of
/// `0.001` and dividing gives a mean that differs from each element by a few
/// ULPs, so the naive test passes and the Sharpe comes out at 1.5e15. Anything
/// within a few thousand ULPs of the data's own scale is representation noise,
/// and dividing by it produces a number with no meaning at all.
fn sd(x: &[f64]) -> Option<f64> {
    if x.len() < 2 {
        return None;
    }
    let m = mean(x);
    let v = x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len() as f64 - 1.0);
    let s = v.sqrt();
    let scale = x.iter().fold(0.0_f64, |a, b| a.max(b.abs())).max(1.0);
    (s > scale * 1e-12).then_some(s)
}

/// Per-observation Sharpe. Not annualized: Gate 13 compares it against zero, and
/// a scale factor would move the number without moving the decision.
fn sharpe(x: &[f64]) -> Option<f64> {
    sd(x).map(|s| mean(x) / s)
}

/// Studentized mean: `mean / (sd / √n)`.
fn t_stat(x: &[f64]) -> Option<f64> {
    sd(x).map(|s| mean(x) / (s / (x.len() as f64).sqrt()))
}

/// Linear-interpolated percentile of a sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let pos = p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let w = pos - lo as f64;
    sorted[lo] * (1.0 - w) + sorted[hi] * w
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic pseudo-random return series with a given drift and an
    /// optional AR(1) coefficient, so a test can plant autocorrelation and then
    /// assert the block-length estimator finds it.
    fn series(n: usize, drift: f64, ar1: f64, seed: u64) -> Vec<f64> {
        let mut rng = DetRng::new(seed);
        let mut prev = 0.0;
        (0..n)
            .map(|_| {
                // Box–Muller from two uniforms: a normal shock, deterministically.
                let (u1, u2) = (rng.next_f64().max(1e-12), rng.next_f64());
                let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
                let e = ar1 * prev + z * 0.01;
                prev = e;
                drift + e
            })
            .collect()
    }

    // ── the stationary bootstrap ─────────────────────────────────────────────

    #[test]
    fn a_resample_is_the_same_length_and_draws_only_real_observations() {
        let mut rng = DetRng::new(7);
        let idx = stationary_indices(120, 8.0, &mut rng);
        assert_eq!(idx.len(), 120);
        assert!(idx.iter().all(|i| *i < 120));
    }

    /// The blocks are contiguous runs, not independent draws — that is the whole
    /// difference from an iid bootstrap, and it is what preserves the
    /// autocorrelation a path-dependence check is about.
    #[test]
    fn resampling_preserves_contiguous_runs() {
        let mut rng = DetRng::new(11);
        let idx = stationary_indices(500, 20.0, &mut rng);
        let continued = idx
            .windows(2)
            .filter(|w| w[1] == (w[0] + 1) % 500)
            .count();
        // With a mean block of 20, roughly 95% of steps continue their block.
        assert!(
            continued > 400,
            "only {continued}/499 steps continued a block; blocks are not contiguous"
        );
    }

    /// A block length of 1 degenerates to the iid bootstrap.
    #[test]
    fn a_unit_block_is_the_iid_bootstrap() {
        let mut rng = DetRng::new(3);
        let idx = stationary_indices(200, 1.0, &mut rng);
        let continued = idx.windows(2).filter(|w| w[1] == (w[0] + 1) % 200).count();
        assert!(continued < 10, "a unit block should almost never continue");
    }

    /// Politis–White must find the autocorrelation: a strongly autocorrelated
    /// series needs a longer block than white noise, or the bootstrap destroys
    /// the very structure the gate is testing against.
    #[test]
    fn the_block_length_grows_with_autocorrelation() {
        let white = politis_white_block(&series(800, 0.0, 0.0, 1));
        let ar = politis_white_block(&series(800, 0.0, 0.85, 1));
        assert!(
            ar > white * 1.5,
            "autocorrelated series got block {ar:.2} vs white noise {white:.2}"
        );
        assert!(white >= 1.0 && ar <= 800.0 / 3.0);
    }

    #[test]
    fn a_short_series_cannot_be_bootstrapped() {
        let r = bootstrap_sharpe(&series(10, 0.0, 0.0, 1), 1_000, 1);
        assert!(matches!(r, Err(BootstrapError::TooShort { have: 10 })));
    }

    /// §12.3 says ≥ 1000 resamples. Asking for fewer is refused rather than
    /// quietly obliged: a percentile from 50 draws has a granularity coarser
    /// than the thing it is being compared against.
    #[test]
    fn too_few_resamples_is_refused() {
        let r = bootstrap_sharpe(&series(200, 0.0, 0.0, 1), 100, 1);
        assert!(matches!(r, Err(BootstrapError::TooFewResamples { .. })));
    }

    #[test]
    fn a_constant_series_has_no_sharpe() {
        let r = bootstrap_sharpe(&vec![0.001; 200], 1_000, 1);
        assert_eq!(r, Err(BootstrapError::ZeroVariance));
    }

    /// The gate's actual job: a genuinely profitable series clears a 5th
    /// percentile above zero, and a zero-drift one does not.
    #[test]
    fn gate_13_separates_drift_from_noise() {
        let strong = bootstrap_sharpe(&series(500, 0.004, 0.0, 2), 2_000, 9).expect("strong");
        assert!(strong.observed > 0.0);
        assert!(
            strong.p05 > 0.0,
            "a strong, persistent edge should clear the 5th percentile; got {:.3}",
            strong.p05
        );

        let noise = bootstrap_sharpe(&series(500, 0.0, 0.0, 3), 2_000, 9).expect("noise");
        assert!(
            noise.p05 <= 0.0,
            "zero-drift noise must not clear the gate; got {:.3}",
            noise.p05
        );
    }

    #[test]
    fn the_bootstrap_is_deterministic_given_a_seed() {
        let s = series(300, 0.001, 0.2, 5);
        let a = bootstrap_sharpe(&s, 1_000, 42).unwrap();
        let b = bootstrap_sharpe(&s, 1_000, 42).unwrap();
        assert_eq!(a, b);
        let c = bootstrap_sharpe(&s, 1_000, 43).unwrap();
        assert!(
            (a.p05 - c.p05).abs() > f64::EPSILON,
            "a different seed is a different draw"
        );
    }

    // ── Romano–Wolf ──────────────────────────────────────────────────────────

    #[test]
    fn a_family_of_one_is_not_a_family() {
        let r = romano_wolf(&[series(100, 0.0, 0.0, 1)], 0.05, 5_000, 1);
        assert_eq!(r, Err(BootstrapError::FamilyTooSmall));
    }

    #[test]
    fn a_ragged_family_is_refused() {
        let f = vec![series(100, 0.0, 0.0, 1), series(90, 0.0, 0.0, 2)];
        assert_eq!(romano_wolf(&f, 0.05, 5_000, 1), Err(BootstrapError::RaggedFamily(1)));
    }

    #[test]
    fn too_few_stepdown_resamples_is_refused() {
        let f = vec![series(100, 0.0, 0.0, 1), series(100, 0.0, 0.0, 2)];
        assert!(matches!(
            romano_wolf(&f, 0.05, 1_000, 1),
            Err(BootstrapError::TooFewResamples { min: 5_000, .. })
        ));
    }

    /// A family of pure noise must produce no rejections at α = 0.05. This is the
    /// test that makes the gate mean something: an agent generating variations
    /// until one looks good is exactly the family this is applied to.
    #[test]
    fn a_family_of_noise_yields_no_rejections() {
        let family: Vec<Vec<f64>> = (0..20).map(|i| series(250, 0.0, 0.0, 100 + i)).collect();
        let r = romano_wolf(&family, 0.05, 5_000, 7).expect("stepdown");
        assert_eq!(r.verdicts.len(), 20, "every candidate is reported");
        assert_eq!(
            r.rejected_count(),
            0,
            "no member of a pure-noise family may be declared good"
        );
    }

    /// And the converse: a candidate with a real edge inside a noisy family is
    /// found. Without this the test above would pass on a procedure that rejects
    /// nothing, ever.
    #[test]
    fn a_real_edge_inside_a_noisy_family_is_found() {
        let mut family: Vec<Vec<f64>> = (0..15).map(|i| series(250, 0.0, 0.0, 200 + i)).collect();
        family.push(series(250, 0.006, 0.0, 999));
        let winner = family.len() - 1;

        let r = romano_wolf(&family, 0.05, 5_000, 7).expect("stepdown");
        let v = r.for_index(winner).expect("the winner is reported");
        assert!(v.rejected, "the real edge was not detected; adjusted p = {}", v.adjusted_p);
        assert!(v.t_stat > 0.0);
        // And it did not drag the noise along with it.
        assert!(
            r.rejected_count() <= 2,
            "{} rejections in a family with one real edge",
            r.rejected_count()
        );
    }

    /// Adjusted p-values are monotone in the stepdown order: a candidate can
    /// never be reported as more significant than one rejected ahead of it.
    #[test]
    fn adjusted_p_values_are_monotone_in_the_stepdown_order() {
        let family: Vec<Vec<f64>> = (0..10)
            .map(|i: u64| series(250, i as f64 * 0.0005, 0.0, 300 + i))
            .collect();
        let r = romano_wolf(&family, 0.05, 5_000, 3).expect("stepdown");
        for w in r.verdicts.windows(2) {
            assert!(
                w[1].adjusted_p >= w[0].adjusted_p - 1e-12,
                "p-values fell from {} to {}",
                w[0].adjusted_p,
                w[1].adjusted_p
            );
            assert!(
                w[1].t_stat <= w[0].t_stat + 1e-12,
                "candidates are not in descending t order"
            );
        }
    }

    /// The family test is what stops a correlated sweep from being penalized as
    /// many independent tests: 30 near-copies of one idea should behave like
    /// roughly one test, not thirty.
    #[test]
    fn a_correlated_sweep_is_not_penalized_as_independent_tests() {
        let base = series(250, 0.005, 0.0, 77);
        // Thirty variants that are the same idea with small perturbations.
        let family: Vec<Vec<f64>> = (0..30)
            .map(|i| {
                let jitter = series(250, 0.0, 0.0, 400 + i);
                base.iter()
                    .zip(&jitter)
                    .map(|(b, j)| b + j * 0.05)
                    .collect()
            })
            .collect();
        let r = romano_wolf(&family, 0.05, 5_000, 5).expect("stepdown");
        assert!(
            r.rejected_count() > 0,
            "a correlated sweep over a real edge should still reject; none did"
        );
    }

    #[test]
    fn the_stepdown_is_deterministic_given_a_seed() {
        let family: Vec<Vec<f64>> = (0..5).map(|i| series(200, 0.001, 0.0, 500 + i)).collect();
        let a = romano_wolf(&family, 0.05, 5_000, 21).unwrap();
        let b = romano_wolf(&family, 0.05, 5_000, 21).unwrap();
        assert_eq!(a, b);
    }
}
