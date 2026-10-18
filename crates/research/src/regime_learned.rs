//! The learned regime tier: a Gaussian HMM and Bayesian online change-point detection
//! (Set R-b RB-4.6; ADR-P4-01; FEAT-003 §10).
//!
//! M7 in the research-process ladder is *vol terciles now, an HMM when 3.5's
//! validation passes*. This is the HMM and the change-point detector, both runnable on
//! this machine with no library beyond `std`, and the trigger that decides whether the
//! learned tier may answer at all.
//!
//! # Filtered posteriors only
//!
//! [`Hmm::filter`] returns `P(state_t | x_1 .. x_t)`. Baum-Welch needs a backward
//! pass to *estimate* parameters on the training window; that pass lives inside the
//! private `fit` and its result is parameters, never a label (ADR-RB-09). Nothing here
//! returns a smoothed posterior, so a regime a strategy reads cannot have been revised
//! by data the strategy had not seen (ADR-RB-05).
//!
//! # The trigger
//!
//! [`LearnedGate`] says whether the learned tier may answer: only when the rule tier's
//! separation test has *passed* (the labels separate returns out of sample), because a
//! learned model of a labelling that does not mean anything is a confident wrong
//! answer. Below the trigger the rule tier answers and the decision records
//! `decision_tier = "rule"` with the reason; above it the learned tier runs and both
//! tiers' labels are logged so they can be compared (A/B by `decision_tier`).

use serde::{Deserialize, Serialize};

const MIN_VAR: f64 = 1e-10;

fn ln_normal(x: f64, mean: f64, var: f64) -> f64 {
    -0.5 * ((2.0 * std::f64::consts::PI * var).ln() + (x - mean).powi(2) / var)
}

/// A Gaussian hidden Markov model over a univariate series.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hmm {
    pub means: Vec<f64>,
    pub vars: Vec<f64>,
    /// `trans[i][j] = P(next = j | now = i)`.
    pub trans: Vec<Vec<f64>>,
    pub init: Vec<f64>,
}

impl Hmm {
    /// States ordered by variance, so index 0 is the calmest.
    ///
    /// # Panics
    /// Never; `k >= 1` is asserted by the caller through [`fit`].
    fn sorted(mut self) -> Self {
        let k = self.means.len();
        let mut order: Vec<usize> = (0..k).collect();
        order.sort_by(|&a, &b| self.vars[a].total_cmp(&self.vars[b]));
        let means = order.iter().map(|&i| self.means[i]).collect();
        let vars = order.iter().map(|&i| self.vars[i]).collect();
        let trans = order
            .iter()
            .map(|&i| order.iter().map(|&j| self.trans[i][j]).collect())
            .collect();
        let init = order.iter().map(|&i| self.init[i]).collect();
        self.means = means;
        self.vars = vars;
        self.trans = trans;
        self.init = init;
        self
    }

    #[must_use]
    pub fn k(&self) -> usize {
        self.means.len()
    }

    /// `P(state_t | x_1 .. x_t)` for each `t`.
    #[must_use]
    pub fn filter(&self, xs: &[f64]) -> Vec<Vec<f64>> {
        let k = self.k();
        let mut out = Vec::with_capacity(xs.len());
        let mut alpha: Vec<f64> = Vec::new();
        for (t, &x) in xs.iter().enumerate() {
            let prior: Vec<f64> = if t == 0 {
                self.init.clone()
            } else {
                (0..k)
                    .map(|j| (0..k).map(|i| alpha[i] * self.trans[i][j]).sum())
                    .collect()
            };
            let mut post: Vec<f64> = (0..k)
                .map(|s| prior[s] * ln_normal(x, self.means[s], self.vars[s]).exp())
                .collect();
            let z: f64 = post.iter().sum();
            if z > 0.0 && z.is_finite() {
                post.iter_mut().for_each(|p| *p /= z);
            } else {
                post = prior;
            }
            alpha = post.clone();
            out.push(post);
        }
        out
    }

    /// The most probable *filtered* state at each `t`.
    #[must_use]
    pub fn labels(&self, xs: &[f64]) -> Vec<usize> {
        self.filter(xs)
            .iter()
            .map(|p| {
                p.iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .map_or(0, |(i, _)| i)
            })
            .collect()
    }

    /// Log-likelihood of a series under the model.
    #[must_use]
    pub fn log_likelihood(&self, xs: &[f64]) -> f64 {
        let k = self.k();
        let mut alpha = self.init.clone();
        let mut ll = 0.0;
        for (t, &x) in xs.iter().enumerate() {
            let prior: Vec<f64> = if t == 0 {
                alpha.clone()
            } else {
                (0..k)
                    .map(|j| (0..k).map(|i| alpha[i] * self.trans[i][j]).sum())
                    .collect()
            };
            let mut post: Vec<f64> = (0..k)
                .map(|s| prior[s] * ln_normal(x, self.means[s], self.vars[s]).exp())
                .collect();
            let z: f64 = post.iter().sum::<f64>().max(1e-300);
            ll += z.ln();
            post.iter_mut().for_each(|p| *p /= z);
            alpha = post;
        }
        ll
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HmmError {
    #[error("{0} observations is too few to fit {1} states; need at least {2}")]
    TooShort(usize, usize, usize),
}

/// Baum-Welch with scaled forward-backward, initialised at the variance quantiles.
///
/// # Errors
/// [`HmmError::TooShort`].
pub fn fit(xs: &[f64], k: usize, iterations: usize) -> Result<Hmm, HmmError> {
    let need = 30 * k;
    if xs.len() < need {
        return Err(HmmError::TooShort(xs.len(), k, need));
    }
    let n = xs.len();
    let mean_all = xs.iter().sum::<f64>() / n as f64;
    // Initial states from the absolute-deviation quantiles.
    let mut by_abs: Vec<f64> = xs.iter().map(|x| (x - mean_all).abs()).collect();
    by_abs.sort_by(f64::total_cmp);
    let mut m = Hmm {
        means: vec![mean_all; k],
        vars: (0..k)
            .map(|s| {
                let q = by_abs[((2 * s + 1) * n / (2 * k)).min(n - 1)];
                (q * q).max(MIN_VAR)
            })
            .collect(),
        trans: (0..k)
            .map(|i| {
                (0..k)
                    .map(|j| {
                        if i == j {
                            0.9
                        } else {
                            0.1 / (k - 1).max(1) as f64
                        }
                    })
                    .collect()
            })
            .collect(),
        init: vec![1.0 / k as f64; k],
    };
    for _ in 0..iterations {
        // E-step (scaled). `gamma`/`xi` are parameter-estimation quantities and never leave
        // this function (ADR-RB-09).
        let emit: Vec<Vec<f64>> = xs
            .iter()
            .map(|&x| {
                (0..k)
                    .map(|s| ln_normal(x, m.means[s], m.vars[s]).exp().max(1e-300))
                    .collect()
            })
            .collect();
        let mut alpha = vec![vec![0.0; k]; n];
        let mut c = vec![0.0; n];
        for t in 0..n {
            for s in 0..k {
                let prior = if t == 0 {
                    m.init[s]
                } else {
                    (0..k).map(|i| alpha[t - 1][i] * m.trans[i][s]).sum()
                };
                alpha[t][s] = prior * emit[t][s];
            }
            c[t] = alpha[t].iter().sum::<f64>().max(1e-300);
            alpha[t].iter_mut().for_each(|a| *a /= c[t]);
        }
        let mut beta = vec![vec![1.0; k]; n];
        for t in (0..n - 1).rev() {
            for i in 0..k {
                beta[t][i] = (0..k)
                    .map(|j| m.trans[i][j] * emit[t + 1][j] * beta[t + 1][j])
                    .sum::<f64>()
                    / c[t + 1];
            }
        }
        let gamma: Vec<Vec<f64>> = (0..n)
            .map(|t| {
                let g: Vec<f64> = (0..k).map(|s| alpha[t][s] * beta[t][s]).collect();
                let z: f64 = g.iter().sum::<f64>().max(1e-300);
                g.into_iter().map(|v| v / z).collect()
            })
            .collect();
        let mut trans = vec![vec![0.0; k]; k];
        for t in 0..n - 1 {
            for i in 0..k {
                for j in 0..k {
                    trans[i][j] +=
                        alpha[t][i] * m.trans[i][j] * emit[t + 1][j] * beta[t + 1][j] / c[t + 1];
                }
            }
        }
        for row in &mut trans {
            let z: f64 = row.iter().sum::<f64>().max(1e-300);
            row.iter_mut().for_each(|v| *v = (*v / z).clamp(1e-6, 1.0));
            let z2: f64 = row.iter().sum();
            row.iter_mut().for_each(|v| *v /= z2);
        }
        for s in 0..k {
            let w: f64 = gamma.iter().map(|g| g[s]).sum::<f64>().max(1e-300);
            m.means[s] = gamma.iter().zip(xs).map(|(g, x)| g[s] * x).sum::<f64>() / w;
            m.vars[s] = (gamma
                .iter()
                .zip(xs)
                .map(|(g, x)| g[s] * (x - m.means[s]).powi(2))
                .sum::<f64>()
                / w)
                .max(MIN_VAR);
        }
        m.trans = trans;
        m.init = gamma[0].clone();
    }
    Ok(m.sorted())
}

// ── Bayesian online change-point detection (Adams & MacKay 2007) ────────────

/// Normal-Inverse-Gamma conjugate BOCPD with a constant hazard.
#[derive(Clone, Debug)]
pub struct Bocpd {
    hazard: f64,
    mu0: f64,
    kappa0: f64,
    alpha0: f64,
    beta0: f64,
    // Per run-length sufficient statistics.
    mu: Vec<f64>,
    kappa: Vec<f64>,
    alpha: Vec<f64>,
    beta: Vec<f64>,
    /// Run-length posterior, `r[l] = P(run length = l | data so far)`.
    r: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChangeState {
    /// `P(run length <= window)`: the posterior mass on a change in the last
    /// `window` observations.
    pub recent_change: f64,
    /// The most probable run length.
    pub map_run_length: usize,
}

impl Bocpd {
    /// `expected_run` is the prior mean run length, so the hazard is its inverse.
    #[must_use]
    pub fn new(expected_run: f64, scale: f64) -> Self {
        let b0 = scale * scale;
        Self {
            hazard: 1.0 / expected_run.max(2.0),
            mu0: 0.0,
            kappa0: 1.0,
            alpha0: 1.0,
            beta0: b0.max(MIN_VAR),
            mu: vec![0.0],
            kappa: vec![1.0],
            alpha: vec![1.0],
            beta: vec![b0.max(MIN_VAR)],
            r: vec![1.0],
        }
    }

    fn predictive(&self, l: usize, x: f64) -> f64 {
        // Student-t with 2*alpha degrees of freedom.
        let nu = 2.0 * self.alpha[l];
        let scale2 = self.beta[l] * (self.kappa[l] + 1.0) / (self.alpha[l] * self.kappa[l]);
        let z = (x - self.mu[l]).powi(2) / (nu * scale2);
        let ln_gamma = |v: f64| libm_ln_gamma(v);
        (ln_gamma((nu + 1.0) / 2.0)
            - ln_gamma(nu / 2.0)
            - 0.5 * (nu * std::f64::consts::PI * scale2).ln()
            - (nu + 1.0) / 2.0 * (1.0 + z).ln())
        .exp()
    }

    /// Consume one observation. `window` is how far back a change counts as "recent".
    pub fn update(&mut self, x: f64, window: usize) -> ChangeState {
        let n = self.r.len();
        let pred: Vec<f64> = (0..n).map(|l| self.predictive(l, x)).collect();
        let mut growth: Vec<f64> = (0..n)
            .map(|l| self.r[l] * pred[l] * (1.0 - self.hazard))
            .collect();
        let cp: f64 = (0..n).map(|l| self.r[l] * pred[l] * self.hazard).sum();
        let mut next_r = Vec::with_capacity(n + 1);
        next_r.push(cp);
        next_r.append(&mut growth);
        let z: f64 = next_r.iter().sum::<f64>().max(1e-300);
        next_r.iter_mut().for_each(|v| *v /= z);

        let mut mu = vec![self.mu0];
        let mut kappa = vec![self.kappa0];
        let mut alpha = vec![self.alpha0];
        let mut beta = vec![self.beta0];
        for l in 0..n {
            mu.push((self.kappa[l] * self.mu[l] + x) / (self.kappa[l] + 1.0));
            kappa.push(self.kappa[l] + 1.0);
            alpha.push(self.alpha[l] + 0.5);
            beta.push(
                self.beta[l]
                    + self.kappa[l] * (x - self.mu[l]).powi(2) / (2.0 * (self.kappa[l] + 1.0)),
            );
        }
        self.mu = mu;
        self.kappa = kappa;
        self.alpha = alpha;
        self.beta = beta;
        self.r = next_r;
        let recent: f64 = self.r.iter().take(window + 1).sum();
        let map = self
            .r
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i);
        ChangeState {
            recent_change: recent,
            map_run_length: map,
        }
    }
}

/// Lanczos approximation of `ln Gamma(x)` for `x > 0`; accurate to ~1e-10, which is
/// far finer than a predictive density needs.
fn libm_ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln()
            - libm_ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

// ── The trigger (ADR-P4-01, M7) ─────────────────────────────────────────────

/// Which tier answered a regime question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionTier {
    Rule,
    Learned,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TierAnswer {
    pub decision_tier: DecisionTier,
    pub reason: String,
}

/// M7's trigger: the learned tier answers only after the rule tier's separation test
/// has passed on enough regimes. `separation_passed` is `None` when the test has never
/// run, which is *not met*, not "probably fine".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LearnedGate {
    pub separation_passed: Option<bool>,
}

impl LearnedGate {
    #[must_use]
    pub fn answer(&self) -> TierAnswer {
        match self.separation_passed {
            Some(true) => TierAnswer {
                decision_tier: DecisionTier::Learned,
                reason: "the rule tier's out-of-sample separation test passed; the learned tier answers and both are logged".into(),
            },
            Some(false) => TierAnswer {
                decision_tier: DecisionTier::Rule,
                reason: "the rule tier's separation test did not pass: its labels do not separate returns, so a learned model of them is not trusted".into(),
            },
            None => TierAnswer {
                decision_tier: DecisionTier::Rule,
                reason: "the separation test has never run for this scope; the rule tier answers".into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampler::Rng;

    fn mixture(n: usize, seed: u64) -> (Vec<f64>, Vec<usize>) {
        let mut rng = Rng::new(seed);
        let sig = [0.004, 0.012, 0.035];
        let (mut x, mut s_t) = (Vec::new(), Vec::new());
        let mut s = 1usize;
        while x.len() < n {
            for _ in 0..(25 + rng.below(40)) {
                x.push(sig[s] * rng.next_normal());
                s_t.push(s);
            }
            s = (s + 1 + rng.below(2)) % 3;
        }
        x.truncate(n);
        s_t.truncate(n);
        (x, s_t)
    }

    #[test]
    fn the_hmm_recovers_ordered_variances_and_sticky_transitions() {
        let (x, _) = mixture(2500, 7);
        let m = fit(&x, 3, 40).unwrap();
        assert!(
            m.vars[0] < m.vars[1] && m.vars[1] < m.vars[2],
            "{:?}",
            m.vars
        );
        for i in 0..3 {
            assert!(
                m.trans[i][i] > 0.8,
                "state {i} self-transition {}",
                m.trans[i][i]
            );
            assert!((m.trans[i].iter().sum::<f64>() - 1.0).abs() < 1e-9);
        }
        // The calm state's sd is near the truth.
        assert!(
            (m.vars[0].sqrt() - 0.004).abs() < 0.002,
            "{}",
            m.vars[0].sqrt()
        );
    }

    #[test]
    fn filtered_labels_do_not_change_when_later_data_arrives() {
        let (x, truth) = mixture(1200, 8);
        let m = fit(&x, 3, 30).unwrap();
        let all = m.labels(&x);
        for t in [100usize, 400, 799] {
            assert_eq!(m.labels(&x[..=t]), all[..=t].to_vec());
        }
        let agree = all
            .iter()
            .zip(&truth)
            .skip(50)
            .filter(|(a, b)| a == b)
            .count() as f64
            / (x.len() - 50) as f64;
        assert!(agree > 0.6, "{agree}");
        // Filtered posteriors are distributions.
        for p in m.filter(&x[..200]) {
            assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn the_hmm_beats_a_single_gaussian_on_likelihood() {
        let (x, _) = mixture(1500, 9);
        let m = fit(&x, 3, 30).unwrap();
        let single = fit_single_ll(&x);
        assert!(
            m.log_likelihood(&x) > single + 50.0,
            "{} vs {single}",
            m.log_likelihood(&x)
        );
    }

    fn fit_single_ll(x: &[f64]) -> f64 {
        let n = x.len() as f64;
        let mean = x.iter().sum::<f64>() / n;
        let var = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
        x.iter().map(|v| ln_normal(*v, mean, var)).sum()
    }

    #[test]
    fn too_short_a_series_is_refused() {
        assert!(matches!(
            fit(&[0.0; 20], 3, 5),
            Err(HmmError::TooShort(20, 3, 90))
        ));
    }

    #[test]
    fn bocpd_detects_a_variance_break_and_stays_quiet_before_it() {
        // A break is the most probable run length collapsing: stationary data keeps a long MAP
        // run, the break sends it to a handful of observations. (The mass on short runs is not
        // the signal: under stationarity every short run fits the data too.)
        let mut rng = Rng::new(21);
        let mut det = Bocpd::new(100.0, 0.01);
        let (mut before_min, mut after_min) = (usize::MAX, usize::MAX);
        for t in 0..300 {
            let sd = if t < 150 { 0.005 } else { 0.03 };
            let s = det.update(sd * rng.next_normal(), 15);
            if (60..150).contains(&t) {
                before_min = before_min.min(s.map_run_length);
            }
            if (152..175).contains(&t) {
                after_min = after_min.min(s.map_run_length);
            }
        }
        assert!(
            before_min >= 30,
            "MAP run length dipped to {before_min} on stationary data"
        );
        assert!(
            after_min <= 15,
            "MAP run length never collapsed after the break: {after_min}"
        );
    }

    #[test]
    fn ln_gamma_matches_known_values() {
        assert!((libm_ln_gamma(1.0)).abs() < 1e-9);
        assert!((libm_ln_gamma(5.0) - 24.0_f64.ln()).abs() < 1e-9);
        assert!((libm_ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-9);
    }

    #[test]
    fn below_the_trigger_the_rule_tier_answers_and_says_why() {
        for g in [None, Some(false)] {
            let a = LearnedGate {
                separation_passed: g,
            }
            .answer();
            assert_eq!(a.decision_tier, DecisionTier::Rule);
            assert!(!a.reason.is_empty());
        }
        assert_eq!(
            LearnedGate {
                separation_passed: Some(true)
            }
            .answer()
            .decision_tier,
            DecisionTier::Learned
        );
    }

    #[test]
    fn no_public_function_returns_a_smoothed_posterior() {
        let src = include_str!("regime_learned.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        for line in prod
            .lines()
            .filter(|l| l.trim_start().starts_with("pub fn"))
        {
            let l = line.to_ascii_lowercase();
            assert!(
                !l.contains("smooth")
                    && !l.contains("viterbi")
                    && !l.contains("backward")
                    && !l.contains("gamma"),
                "{line}"
            );
        }
    }
}
