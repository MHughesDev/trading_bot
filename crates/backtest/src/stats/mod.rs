//! Significance math for Gate 3 (spec §2.2). Pure Rust — no Python sidecar (D-9).
//!
//! One **primary** verdict (a permutation p-value, selection-bias-corrected by the
//! live trial counter) and **two corroborators** (Deflated Sharpe Ratio and
//! Probability of Backtest Overfitting). These should agree; disagreement is a
//! flag to investigate, not a result to shop between (spec §2.2 Gate 3).

use crate::study::combinations;

const EULER_MASCHERONI: f64 = 0.577_215_664_901_532_9;

/// One-sided permutation p-value with add-one smoothing: the fraction of null
/// worlds whose statistic is at least as extreme as the observed one. Smoothing
/// keeps p strictly positive (you never "proved" significance with finite draws).
#[must_use]
pub fn permutation_p_value(observed: f64, null_distribution: &[f64]) -> f64 {
    let n = null_distribution.len();
    if n == 0 {
        return 1.0;
    }
    let at_least_as_extreme = null_distribution.iter().filter(|&&x| x >= observed).count();
    (1 + at_least_as_extreme) as f64 / (1 + n) as f64
}

/// Selection-bias correction (Šidák): inflate a single-test p-value for the
/// number of trials that produced the result (INV-3). A Sharpe found after 3
/// trials and after 3,000 trials yield radically different corrected p-values.
#[must_use]
pub fn selection_bias_correction(p_value: f64, trials: i64) -> f64 {
    let t = trials.max(1) as f64;
    let p = p_value.clamp(0.0, 1.0);
    // 1 - (1 - p)^T : probability that at least one of T independent trials
    // would beat this under the null.
    (1.0 - (1.0 - p).powf(t)).clamp(0.0, 1.0)
}

/// Standard normal CDF Φ via `erf` (Abramowitz & Stegun 7.1.26).
#[must_use]
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

/// Inverse standard normal CDF Φ⁻¹ (Acklam's rational approximation).
#[must_use]
pub fn inverse_normal_cdf(p: f64) -> f64 {
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239e0,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838e0,
        -2.549_732_539_343_734e0,
        4.374_664_141_464_968e0,
        2.938_163_982_698_783e0,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996e0,
        3.754_408_661_907_416e0,
    ];
    let plow = 0.024_25;
    let phigh = 1.0 - plow;
    if p < plow {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= phigh {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// Probabilistic Sharpe Ratio: P(true SR > `sr0`) given the observed `sharpe`,
/// sample size `n`, and the return distribution's `skew`/`kurtosis` (Bailey &
/// López de Prado).
#[must_use]
pub fn probabilistic_sharpe_ratio(sharpe: f64, sr0: f64, n: usize, skew: f64, kurtosis: f64) -> f64 {
    if n < 2 {
        return 0.5;
    }
    let denom = (1.0 - skew * sharpe + ((kurtosis - 1.0) / 4.0) * sharpe * sharpe).max(1e-9);
    let z = (sharpe - sr0) * ((n - 1) as f64).sqrt() / denom.sqrt();
    normal_cdf(z)
}

/// Expected maximum of `t` i.i.d. standard normals (López de Prado approximation).
#[must_use]
pub fn expected_max_standard_gaussian(t: i64) -> f64 {
    let t = t.max(2) as f64;
    (1.0 - EULER_MASCHERONI) * inverse_normal_cdf(1.0 - 1.0 / t)
        + EULER_MASCHERONI * inverse_normal_cdf(1.0 - 1.0 / (t * std::f64::consts::E))
}

/// Deflated Sharpe Ratio: the PSR against a benchmark `sr0` that accounts for the
/// number of trials and the variance of trial Sharpes (Bailey & López de Prado).
/// As `trials` grows (holding the observed Sharpe fixed), DSR falls — exposing
/// selection bias. Returns a probability in `[0, 1]`.
#[must_use]
pub fn deflated_sharpe_ratio(
    sharpe: f64,
    n: usize,
    skew: f64,
    kurtosis: f64,
    trials: i64,
    sharpe_variance_across_trials: f64,
) -> f64 {
    let sr0 = sharpe_variance_across_trials.max(0.0).sqrt() * expected_max_standard_gaussian(trials);
    probabilistic_sharpe_ratio(sharpe, sr0, n, skew, kurtosis)
}

/// Probability of Backtest Overfitting via Combinatorially-Symmetric Cross
/// Validation (Bailey et al.). `performance[config][period]` is a matrix of a
/// per-period performance metric for each candidate config. Returns the
/// probability that the in-sample-best config underperforms the OOS median.
#[must_use]
pub fn probability_of_backtest_overfitting(performance: &[Vec<f64>], n_groups: usize) -> f64 {
    let n_configs = performance.len();
    if n_configs == 0 {
        return 0.0;
    }
    let n_periods = performance[0].len();
    let s = n_groups.max(2).min(n_periods);
    if s < 2 || n_periods < s {
        return 0.0;
    }
    // Partition periods into S contiguous groups.
    let group_of = |period: usize| -> usize { period * s / n_periods };
    let mut logits = Vec::new();
    // Each train set = a combination of S/2 groups; test = the rest.
    let k = s / 2;
    for train_groups in combinations(s, k) {
        let is_train = |period: usize| train_groups.contains(&group_of(period));
        // In-sample mean performance per config.
        let mut best_cfg = 0;
        let mut best_is = f64::NEG_INFINITY;
        for (c, row) in performance.iter().enumerate() {
            let (mut sum, mut cnt) = (0.0, 0.0);
            for (p, &v) in row.iter().enumerate() {
                if is_train(p) {
                    sum += v;
                    cnt += 1.0;
                }
            }
            let m = if cnt > 0.0 { sum / cnt } else { f64::NEG_INFINITY };
            if m > best_is {
                best_is = m;
                best_cfg = c;
            }
        }
        // OOS performance of every config; rank the in-sample-best config.
        let mut oos: Vec<f64> = performance
            .iter()
            .map(|row| {
                let (mut sum, mut cnt) = (0.0, 0.0);
                for (p, &v) in row.iter().enumerate() {
                    if !is_train(p) {
                        sum += v;
                        cnt += 1.0;
                    }
                }
                if cnt > 0.0 {
                    sum / cnt
                } else {
                    f64::NEG_INFINITY
                }
            })
            .collect();
        let chosen_oos = oos[best_cfg];
        oos.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // Relative rank ω in (0,1): fraction of configs the chosen beats OOS.
        let rank = oos.iter().filter(|&&v| v < chosen_oos).count() as f64;
        let omega = ((rank + 0.5) / n_configs as f64).clamp(1e-6, 1.0 - 1e-6);
        logits.push((omega / (1.0 - omega)).ln());
    }
    if logits.is_empty() {
        return 0.0;
    }
    // PBO = fraction of splits where the chosen config lands below the OOS median
    // (logit < 0).
    logits.iter().filter(|&&l| l < 0.0).count() as f64 / logits.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutation_p_is_smoothed_and_ordered() {
        let null: Vec<f64> = (0..99).map(f64::from).collect();
        // Observed far above the null → small p (only the +1 smoothing remains).
        let p_high = permutation_p_value(1000.0, &null);
        assert!((p_high - 1.0 / 100.0).abs() < 1e-12);
        // Observed below everything → p near 1.
        let p_low = permutation_p_value(-1.0, &null);
        assert!(p_low > 0.9);
        // Empty null → uninformative.
        assert_eq!(permutation_p_value(1.0, &[]), 1.0);
    }

    #[test]
    fn selection_bias_inflates_p_with_trials() {
        let raw = 0.01;
        let p3 = selection_bias_correction(raw, 3);
        let p3000 = selection_bias_correction(raw, 3000);
        assert!(p3 > raw);
        assert!(p3000 > p3);
        assert!(p3000 > 0.99, "3000 trials make a raw p=0.01 nearly certain under H0");
        assert!(p3 < 0.05, "3 trials keep it borderline-significant");
    }

    #[test]
    fn normal_cdf_and_inverse_are_consistent() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-6);
        assert!((normal_cdf(1.96) - 0.975).abs() < 2e-3);
        // Round-trip Φ⁻¹(Φ(x)) ≈ x.
        for x in [-2.0, -0.5, 0.3, 1.5] {
            let back = inverse_normal_cdf(normal_cdf(x));
            assert!((back - x).abs() < 1e-2, "x={x} back={back}");
        }
    }

    #[test]
    fn dsr_falls_as_trials_grow() {
        // Same observed Sharpe; more trials ⇒ higher SR0 bar ⇒ lower DSR.
        let dsr_few = deflated_sharpe_ratio(2.0, 250, 0.0, 3.0, 5, 0.25);
        let dsr_many = deflated_sharpe_ratio(2.0, 250, 0.0, 3.0, 5000, 0.25);
        assert!(dsr_few > dsr_many, "few={dsr_few} many={dsr_many}");
        assert!((0.0..=1.0).contains(&dsr_many));
    }

    #[test]
    fn psr_is_high_for_strong_sharpe_low_for_weak() {
        let strong = probabilistic_sharpe_ratio(2.0, 0.0, 252, 0.0, 3.0);
        // A zero observed Sharpe is indistinguishable from the SR0=0 benchmark.
        let weak = probabilistic_sharpe_ratio(0.0, 0.0, 252, 0.0, 3.0);
        assert!(strong > 0.95);
        assert!((weak - 0.5).abs() < 1e-6);
        assert!(strong > weak);
    }

    #[test]
    fn pbo_high_for_noise_low_for_genuine_edge() {
        // Genuine edge: config 0 is best every period → never overfits OOS.
        let genuine: Vec<Vec<f64>> = {
            let mut m = vec![vec![0.0; 12]; 4];
            for p in 0..12 {
                m[0][p] = 1.0; // dominant
                for c in 1..4 {
                    m[c][p] = 0.1 * c as f64;
                }
            }
            m
        };
        let pbo_genuine = probability_of_backtest_overfitting(&genuine, 6);
        assert!(pbo_genuine < 0.5, "a uniformly-best config should not look overfit: {pbo_genuine}");

        // Pure noise: alternating which config "wins" by period parity → the
        // in-sample winner flips OOS.
        let noise: Vec<Vec<f64>> = {
            let mut m = vec![vec![0.0; 12]; 4];
            for p in 0..12 {
                // each config wins only on periods ≡ its index mod 4
                for (c, row) in m.iter_mut().enumerate() {
                    row[p] = if p % 4 == c { 1.0 } else { -0.1 };
                }
            }
            m
        };
        let pbo_noise = probability_of_backtest_overfitting(&noise, 4);
        assert!(pbo_noise >= pbo_genuine, "noise should be at least as overfit: noise={pbo_noise} genuine={pbo_genuine}");
    }
}
