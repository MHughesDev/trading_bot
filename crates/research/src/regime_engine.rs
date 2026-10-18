//! A statistical jump model for regimes, filtered only (Set R-b RB-4.2; FEAT-003 §10).
//!
//! Three persistent states over causal features of a return series. A state is a
//! centroid in standardised feature space; the model prefers to stay where it is and
//! pays a penalty `lambda` for each jump. That penalty is what makes regimes
//! *regimes* rather than a noisy per-day cluster label, and it is chosen from a grid
//! by an information criterion over the training window, not set by hand.
//!
//! # Filtered, always (ADR-RB-05)
//!
//! The label at time `t` is the argmin of a forward recursion over `x_1 .. x_t` and
//! nothing later. [`Filter::update`] is that recursion, one observation at a time;
//! [`label_series`] is `update` in a loop. There is no function that revisits an
//! earlier label in the light of later data, no backtracking pass, and the fit itself
//! updates centroids from *filtered* assignments so not even training smooths. A label
//! written to `regime_causal.regime_state` therefore cannot change when later bars
//! arrive, which `a_label_at_t_is_unchanged_when_later_bars_arrive` asserts by
//! construction rather than by sampling.
//!
//! # What this is not
//!
//! It does not read the future, and it does not look at a strategy's returns: the
//! series is the *market scope's* (ADR-P3-07), and the caller chooses it.

use serde::{Deserialize, Serialize};

/// Number of regimes. Three, because the rule tier it replaces has three and the
/// `regime_causal.regime_state` label set is `low_vol`, `mid_vol`, `high_vol`.
pub const K: usize = 3;

/// The causal features of a return series: exponentially weighted mean, downside
/// deviation and volatility. Row `t` uses returns up to and including `t` only.
///
/// `half_life_mean` and `half_life_vol` are in observations.
#[must_use]
pub fn features(returns: &[f64], half_life_mean: f64, half_life_vol: f64) -> Vec<Vec<f64>> {
    let am = 1.0 - 0.5_f64.powf(1.0 / half_life_mean.max(1.0));
    let av = 1.0 - 0.5_f64.powf(1.0 / half_life_vol.max(1.0));
    let (mut m, mut dd, mut vv) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut out = Vec::with_capacity(returns.len());
    for (i, r) in returns.iter().enumerate() {
        if i == 0 {
            m = *r;
            dd = r.min(0.0).powi(2);
            vv = r * r;
        } else {
            m += am * (r - m);
            dd += av * (r.min(0.0).powi(2) - dd);
            vv += av * (r * r - vv);
        }
        out.push(vec![m, dd.sqrt(), vv.sqrt()]);
    }
    out
}

fn dist2(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// The forward recursion. Holds `V_{t-1}(s)`, the cost of the best path ending in
/// state `s`, and nothing else: there is no path to walk back along.
#[derive(Clone, Debug)]
pub struct Filter {
    centroids: Vec<Vec<f64>>,
    lambda: f64,
    cost: Option<Vec<f64>>,
}

impl Filter {
    #[must_use]
    pub fn new(centroids: Vec<Vec<f64>>, lambda: f64) -> Self {
        Self {
            centroids,
            lambda,
            cost: None,
        }
    }

    /// Consume one standardised observation and return the state index in force at it.
    pub fn update(&mut self, x: &[f64]) -> usize {
        let k = self.centroids.len();
        let d: Vec<f64> = self.centroids.iter().map(|c| dist2(x, c)).collect();
        let next: Vec<f64> = match &self.cost {
            None => d,
            Some(prev) => {
                let best = prev.iter().copied().fold(f64::INFINITY, f64::min);
                (0..k)
                    .map(|s| d[s] + prev[s].min(best + self.lambda))
                    .collect()
            }
        };
        // Re-base so the numbers stay bounded; the argmin is unchanged.
        let m = next.iter().copied().fold(f64::INFINITY, f64::min);
        let label = next
            .iter()
            .position(|v| (*v - m).abs() < f64::EPSILON)
            .unwrap_or(0);
        self.cost = Some(next.into_iter().map(|v| v - m).collect());
        label
    }
}

/// Filtered state indices for a standardised feature series.
#[must_use]
pub fn label_series(rows: &[Vec<f64>], centroids: &[Vec<f64>], lambda: f64) -> Vec<usize> {
    let mut f = Filter::new(centroids.to_vec(), lambda);
    rows.iter().map(|r| f.update(r)).collect()
}

/// A fitted model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JumpModel {
    pub lambda: f64,
    /// Feature standardisation learned on the training window.
    pub mean: Vec<f64>,
    pub sd: Vec<f64>,
    /// Centroids in standardised space, ordered by volatility: index 0 is the calmest
    /// regime and `K - 1` the most volatile, so a label index means the same thing in
    /// every fit.
    pub centroids: Vec<Vec<f64>>,
    /// The information criterion at each candidate penalty, for the record.
    pub criterion: Vec<(f64, f64)>,
    pub n_train: usize,
}

impl JumpModel {
    /// Standardise raw feature rows with this model's training statistics.
    #[must_use]
    pub fn standardise(&self, rows: &[Vec<f64>]) -> Vec<Vec<f64>> {
        rows.iter()
            .map(|r| {
                r.iter()
                    .enumerate()
                    .map(|(i, v)| (v - self.mean[i]) / self.sd[i])
                    .collect()
            })
            .collect()
    }

    /// Filtered regime index (0 calm .. 2 stressed) for each raw feature row.
    #[must_use]
    pub fn label(&self, raw_rows: &[Vec<f64>]) -> Vec<usize> {
        label_series(&self.standardise(raw_rows), &self.centroids, self.lambda)
    }

    /// A streaming filter over raw rows, for labelling one bar at a time.
    #[must_use]
    pub fn filter(&self) -> Filter {
        Filter::new(self.centroids.clone(), self.lambda)
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FitError {
    #[error("{0} observations is too few to fit {K} regimes; need at least {1}")]
    TooShort(usize, usize),
    #[error("the feature rows are ragged or empty")]
    Ragged,
}

fn standardise_fit(rows: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
    let d = rows[0].len();
    let n = rows.len() as f64;
    let mean: Vec<f64> = (0..d)
        .map(|j| rows.iter().map(|r| r[j]).sum::<f64>() / n)
        .collect();
    let sd: Vec<f64> = (0..d)
        .map(|j| {
            let v = rows.iter().map(|r| (r[j] - mean[j]).powi(2)).sum::<f64>() / n;
            if v.sqrt() > 1e-12 {
                v.sqrt()
            } else {
                1.0
            }
        })
        .collect();
    (mean, sd)
}

/// The shortest median regime length a fit may have to be chosen (observations).
pub const MIN_MEDIAN_RUN: f64 = 5.0;

/// The penalty grid the criterion chooses from.
pub const LAMBDA_GRID: [f64; 7] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];

#[allow(clippy::needless_range_loop)] // s indexes next[s] and the cluster id together
fn fit_at(std_rows: &[Vec<f64>], lambda: f64, vol_col: usize) -> (Vec<Vec<f64>>, f64) {
    let d = std_rows[0].len();
    // Deterministic initialisation: centroids at the terciles of the volatility feature.
    let mut idx: Vec<usize> = (0..std_rows.len()).collect();
    idx.sort_by(|&a, &b| std_rows[a][vol_col].total_cmp(&std_rows[b][vol_col]));
    let mut cent: Vec<Vec<f64>> = (0..K)
        .map(|s| {
            let lo = idx.len() * s / K;
            let hi = (idx.len() * (s + 1) / K).max(lo + 1);
            let group = &idx[lo..hi.min(idx.len())];
            (0..d)
                .map(|j| group.iter().map(|&i| std_rows[i][j]).sum::<f64>() / group.len() as f64)
                .collect()
        })
        .collect();
    for _ in 0..30 {
        let labels = label_series(std_rows, &cent, lambda);
        let mut next = cent.clone();
        for s in 0..K {
            let members: Vec<&Vec<f64>> = std_rows
                .iter()
                .zip(&labels)
                .filter(|(_, l)| **l == s)
                .map(|(r, _)| r)
                .collect();
            if !members.is_empty() {
                next[s] = (0..d)
                    .map(|j| members.iter().map(|r| r[j]).sum::<f64>() / members.len() as f64)
                    .collect();
            }
        }
        let moved: f64 = cent.iter().zip(&next).map(|(a, b)| dist2(a, b)).sum();
        cent = next;
        if moved < 1e-10 {
            break;
        }
    }
    let labels = label_series(std_rows, &cent, lambda);
    let sse: f64 = std_rows
        .iter()
        .zip(&labels)
        .map(|(r, l)| dist2(r, &cent[*l]))
        .sum();
    let jumps = labels.windows(2).filter(|w| w[0] != w[1]).count();
    let n = std_rows.len() as f64;
    let ic = n * d as f64 * (sse / (n * d as f64) + 1e-12).ln()
        + n.ln() * ((K * d) as f64 + jumps as f64);
    (cent, ic)
}

/// Fit a jump model. `vol_col` is the column of the feature rows holding volatility,
/// used to initialise and to order the regimes.
///
/// # Errors
/// [`FitError`] for too little or ragged data.
pub fn fit(raw_rows: &[Vec<f64>], vol_col: usize) -> Result<JumpModel, FitError> {
    let min = 60;
    if raw_rows.len() < min {
        return Err(FitError::TooShort(raw_rows.len(), min));
    }
    let d = raw_rows[0].len();
    if d == 0 || vol_col >= d || raw_rows.iter().any(|r| r.len() != d) {
        return Err(FitError::Ragged);
    }
    let (mean, sd) = standardise_fit(raw_rows);
    let std_rows: Vec<Vec<f64>> = raw_rows
        .iter()
        .map(|r| {
            r.iter()
                .enumerate()
                .map(|(j, v)| (v - mean[j]) / sd[j])
                .collect()
        })
        .collect();
    // Candidates: every penalty on the grid, each with its information criterion and the
    // median regime length it produces on the training window. Persistence is a requirement
    // on the choice (FEAT-003 section 15: median regime length at least five observations),
    // not a hope: among penalties that meet it the criterion decides, and if none does the
    // most persistent one is taken, so a flickering fit is never chosen over a steadier one.
    let mut candidates: Vec<(f64, f64, f64, Vec<Vec<f64>>)> = Vec::new();
    let mut criterion = Vec::new();
    for lambda in LAMBDA_GRID {
        let (cent, ic) = fit_at(&std_rows, lambda, vol_col);
        let run = median_run_length(&label_series(&std_rows, &cent, lambda));
        criterion.push((lambda, ic));
        candidates.push((lambda, ic, run, cent));
    }
    let pick = candidates
        .iter()
        .filter(|c| c.2 >= MIN_MEDIAN_RUN)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .or_else(|| candidates.iter().max_by(|a, b| a.2.total_cmp(&b.2)))
        .ok_or(FitError::Ragged)?;
    let (lambda, mut centroids) = (pick.0, pick.3.clone());
    centroids.sort_by(|a, b| a[vol_col].total_cmp(&b[vol_col]));
    Ok(JumpModel {
        lambda,
        mean,
        sd,
        centroids,
        criterion,
        n_train: raw_rows.len(),
    })
}

/// Median length of a run of equal labels.
#[must_use]
pub fn median_run_length(labels: &[usize]) -> f64 {
    if labels.is_empty() {
        return 0.0;
    }
    let mut runs = Vec::new();
    let mut len = 1usize;
    for w in labels.windows(2) {
        if w[0] == w[1] {
            len += 1;
        } else {
            runs.push(len);
            len = 1;
        }
    }
    runs.push(len);
    runs.sort_unstable();
    let m = runs.len();
    if m % 2 == 1 {
        runs[m / 2] as f64
    } else {
        (runs[m / 2 - 1] + runs[m / 2]) as f64 / 2.0
    }
}

/// Refit cadence: whether a monthly refit is due (RB-4.2: "refit monthly").
#[must_use]
pub fn refit_due(days_since_fit: i64, period_days: i64) -> bool {
    days_since_fit >= period_days
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampler::Rng;

    /// A persistent three-regime series: calm, normal and stressed volatility, each held
    /// for 20 to 60 observations, like a market's.
    fn regime_returns(n: usize, seed: u64) -> (Vec<f64>, Vec<usize>) {
        let mut rng = Rng::new(seed);
        let sig = [0.004, 0.012, 0.035];
        let (mut r, mut truth) = (Vec::new(), Vec::new());
        let mut s = 1usize;
        while r.len() < n {
            let len = 20 + rng.below(40);
            for _ in 0..len {
                r.push(sig[s] * rng.next_normal());
                truth.push(s);
            }
            // jump to a different state
            s = (s + 1 + rng.below(2)) % 3;
        }
        r.truncate(n);
        truth.truncate(n);
        (r, truth)
    }

    #[test]
    fn features_are_causal() {
        let (r, _) = regime_returns(200, 1);
        let full = features(&r, 5.0, 10.0);
        let part = features(&r[..120], 5.0, 10.0);
        assert_eq!(
            &full[..120],
            &part[..],
            "row t depends on returns up to t only"
        );
    }

    #[test]
    fn a_label_at_t_is_unchanged_when_later_bars_arrive() {
        let (r, _) = regime_returns(600, 2);
        let rows = features(&r, 5.0, 10.0);
        let model = fit(&rows, 2).unwrap();
        let all = model.label(&rows);
        for t in [60, 150, 299, 450] {
            let prefix = model.label(&rows[..=t]);
            assert_eq!(
                prefix,
                all[..=t].to_vec(),
                "labels up to {t} changed after bars {t}.. arrived"
            );
            assert_eq!(prefix[t], all[t]);
        }
        // And the streaming filter says the same as the batch call, bar by bar.
        let mut f = model.filter();
        let std = model.standardise(&rows);
        let streamed: Vec<usize> = std.iter().map(|x| f.update(x)).collect();
        assert_eq!(streamed, all);
    }

    #[test]
    fn regimes_persist_and_recover_the_volatility_ordering() {
        let (r, truth) = regime_returns(1500, 3);
        let rows = features(&r, 5.0, 10.0);
        let model = fit(&rows, 2).unwrap();
        let labels = model.label(&rows);
        // Median regime length at least 5 observations (RB-4.2's persistence bar).
        assert!(
            median_run_length(&labels) >= 5.0,
            "{}",
            median_run_length(&labels)
        );
        // The labels track the true regimes better than chance (1/3), after burn-in.
        let agree = labels
            .iter()
            .zip(&truth)
            .skip(60)
            .filter(|(l, t)| l == t)
            .count() as f64
            / (labels.len() - 60) as f64;
        assert!(agree > 0.6, "agreement {agree}");
        // Centroid volatility is increasing: index 0 is the calmest.
        let v: Vec<f64> = model.centroids.iter().map(|c| c[2]).collect();
        assert!(v[0] < v[1] && v[1] < v[2], "{v:?}");
    }

    #[test]
    fn the_penalty_is_chosen_by_the_criterion_among_penalties_that_persist() {
        let (r, _) = regime_returns(800, 4);
        let rows = features(&r, 5.0, 10.0);
        let model = fit(&rows, 2).unwrap();
        assert_eq!(model.criterion.len(), LAMBDA_GRID.len());
        assert!(LAMBDA_GRID.contains(&model.lambda));
        // The chosen fit persists, and no eligible penalty had a smaller criterion.
        assert!(median_run_length(&model.label(&rows)) >= MIN_MEDIAN_RUN);
        let chosen_ic = model
            .criterion
            .iter()
            .find(|(l, _)| *l == model.lambda)
            .unwrap()
            .1;
        for (lambda, ic) in &model.criterion {
            let (_, std_rows) = (0, model.standardise(&rows));
            let (cent, _) = fit_at(&std_rows, *lambda, 2);
            let run = median_run_length(&label_series(&std_rows, &cent, *lambda));
            if run >= MIN_MEDIAN_RUN {
                assert!(
                    chosen_ic <= *ic + 1e-9,
                    "lambda {lambda} was eligible with a smaller criterion"
                );
            }
        }
    }

    #[test]
    fn too_little_data_and_ragged_rows_are_refused() {
        assert!(matches!(
            fit(&vec![vec![0.0; 3]; 10], 2),
            Err(FitError::TooShort(10, _))
        ));
        let mut rows = vec![vec![0.0; 3]; 100];
        rows[5] = vec![0.0];
        assert_eq!(fit(&rows, 2), Err(FitError::Ragged));
    }

    #[test]
    fn nothing_in_the_public_api_smooths() {
        // ADR-RB-05: a smoothed label has no code path. The public items are `features`,
        // `Filter`, `label_series`, `fit`, `median_run_length`, `refit_due`.
        let src = include_str!("regime_engine.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        for line in prod.lines().filter(|l| {
            l.trim_start().starts_with("pub fn") || l.trim_start().starts_with("pub struct")
        }) {
            let l = line.to_ascii_lowercase();
            assert!(
                !l.contains("smooth") && !l.contains("viterbi") && !l.contains("backward"),
                "{line}"
            );
        }
    }

    #[test]
    fn a_refit_is_due_monthly() {
        assert!(!refit_due(29, 30));
        assert!(refit_due(30, 30));
    }
}
