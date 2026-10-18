//! Where does a finished run lose money, by the platform's own regime labels
//! (S-3.13 `regime_failure_search`, AGENT-006 §5.5; ADR-D-12's re-read form).
//!
//! # The re-read form
//!
//! This partitions a **completed** run's equity curve by labelled regime windows
//! and tests each regime's mean per-period return. It dispatches no simulator run
//! and reads no new data, so it spends no trial (ADR-D-12): a statistic computed
//! from an existing look is not a new look.
//!
//! # Why the regimes are not chosen by the caller
//!
//! A hunt over caller-chosen partitions is a garden of forking paths: try enough
//! cuts and one of them shows a loss. The partition is the platform's causal
//! labels (`api::knowledge`), fixed before the run is read, and the correction
//! below is applied over every regime that had enough observations to test.
//!
//! # What is reported, and what stays unmeasured
//!
//! * per regime: `n`, mean, sd, Newey-West t of the mean, a one-sided p for
//!   "this regime's mean return is negative", and the Benjamini-Hochberg adjusted
//!   p across **all tested regimes**. A regime is never reported as a failure on
//!   its raw p.
//! * `worst` is the regime with the lowest mean among those tested; whether it is
//!   a *finding* is `adjusted_p`'s call, and `failure_found` says so.
//! * `unrepresented_risk`: regimes with fewer than `min_obs` periods are listed,
//!   not tested, and so is the share of the curve no label covered. A run that
//!   never met a regime has not been shown to survive it.

use backtest::stats::bh_adjusted;
use backtest::stats::diagnostics::RegimeWindow;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Per-regime result.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegimeRow {
    pub label: String,
    pub n: usize,
    pub mean: f64,
    pub sd: f64,
    pub t_nw: Option<f64>,
    /// One-sided p that the true mean is below zero. `None` below `min_obs`.
    pub p_negative: Option<f64>,
    pub adjusted_p: Option<f64>,
    pub tested: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RegimeSearch {
    pub regimes: Vec<RegimeRow>,
    /// Lowest-mean regime among the tested ones.
    pub worst: Option<String>,
    /// True only when the worst regime's mean is negative **and** its BH-adjusted
    /// p is at or below `alpha`.
    pub failure_found: bool,
    pub alpha: f64,
    pub n_tested: usize,
    pub min_obs: usize,
    /// Regimes seen with too few periods to test.
    pub untested: Vec<String>,
    /// Share of the curve's periods that fall in no labelled window.
    pub unlabelled_share: f64,
    pub n_periods: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegimeSearchError {
    #[error("the equity curve has fewer than {need} points")]
    TooShort { need: usize },
    #[error("no labelled windows were supplied")]
    NoWindows,
}

/// Standard normal CDF via the complementary error function.
fn norm_cdf(x: f64) -> f64 {
    0.5 * erfc(-x / std::f64::consts::SQRT_2)
}

/// Abramowitz & Stegun 7.1.26 style erfc with max error ~1.5e-7, adequate for a
/// screening p-value that is then BH-adjusted.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// Newey-West standard error of the mean with Bartlett weights.
fn nw_se(x: &[f64]) -> f64 {
    let n = x.len();
    if n < 3 {
        return f64::NAN;
    }
    let mean = x.iter().sum::<f64>() / n as f64;
    let e: Vec<f64> = x.iter().map(|v| v - mean).collect();
    let lags = ((n as f64).powf(1.0 / 3.0).floor() as usize).min(n - 2);
    let mut total = e.iter().map(|v| v * v).sum::<f64>() / n as f64;
    for l in 1..=lags {
        let w = 1.0 - l as f64 / (lags as f64 + 1.0);
        let g = e[l..]
            .iter()
            .zip(&e[..n - l])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / n as f64;
        total += 2.0 * w * g;
    }
    (total.max(0.0) / n as f64).sqrt()
}

/// Search a run's equity curve for a regime it loses in.
///
/// `equity` is `(time, equity)` in time order; the return at point `i` is
/// `equity[i] / equity[i-1] - 1` and belongs to the window containing
/// `time[i]`. Windows are half-open `[start, end)`.
///
/// # Errors
/// A curve under 30 points, or no windows.
pub fn search(
    equity: &[(DateTime<Utc>, f64)],
    windows: &[RegimeWindow],
    min_obs: usize,
    alpha: f64,
) -> Result<RegimeSearch, RegimeSearchError> {
    if equity.len() < 30 {
        return Err(RegimeSearchError::TooShort { need: 30 });
    }
    if windows.is_empty() {
        return Err(RegimeSearchError::NoWindows);
    }
    let mut by: std::collections::BTreeMap<String, Vec<f64>> = std::collections::BTreeMap::new();
    let mut unlabelled = 0usize;
    let mut n_periods = 0usize;
    for i in 1..equity.len() {
        let (t, e) = equity[i];
        let prev = equity[i - 1].1;
        if !(prev > 0.0 && e.is_finite()) {
            continue;
        }
        n_periods += 1;
        let r = e / prev - 1.0;
        match windows.iter().find(|w| t >= w.start && t < w.end) {
            Some(w) => by.entry(w.label.clone()).or_default().push(r),
            None => unlabelled += 1,
        }
    }
    let mut rows: Vec<RegimeRow> = Vec::new();
    for (label, xs) in &by {
        let n = xs.len();
        let mean = xs.iter().sum::<f64>() / n as f64;
        let sd = if n > 1 {
            (xs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt()
        } else {
            f64::NAN
        };
        let tested = n >= min_obs;
        let (t_nw, p) = if tested {
            let se = nw_se(xs);
            if se > 0.0 && se.is_finite() {
                let t = mean / se;
                (Some(t), Some(norm_cdf(t)))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };
        rows.push(RegimeRow {
            label: label.clone(),
            n,
            mean,
            sd,
            t_nw,
            p_negative: p,
            adjusted_p: None,
            tested: tested && p.is_some(),
        });
    }
    let idx: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.tested)
        .map(|(i, _)| i)
        .collect();
    let ps: Vec<f64> = idx.iter().filter_map(|&i| rows[i].p_negative).collect();
    let adj = bh_adjusted(&ps);
    for (k, &i) in idx.iter().enumerate() {
        rows[i].adjusted_p = Some(adj[k]);
    }
    let worst = rows
        .iter()
        .filter(|r| r.tested)
        .min_by(|a, b| a.mean.total_cmp(&b.mean));
    let failure_found =
        worst.is_some_and(|w| w.mean < 0.0 && w.adjusted_p.is_some_and(|p| p <= alpha));
    Ok(RegimeSearch {
        worst: worst.map(|w| w.label.clone()),
        failure_found,
        alpha,
        n_tested: idx.len(),
        min_obs,
        untested: rows
            .iter()
            .filter(|r| !r.tested)
            .map(|r| r.label.clone())
            .collect(),
        unlabelled_share: if n_periods == 0 {
            0.0
        } else {
            unlabelled as f64 / n_periods as f64
        },
        n_periods,
        regimes: rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(i: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + i * 3600, 0).unwrap()
    }

    /// Deterministic pseudo-noise so the test needs no RNG crate.
    fn noise(i: usize) -> f64 {
        let x = (i as f64 * 12.9898).sin() * 43758.5453;
        (x - x.floor() - 0.5) * 0.004
    }

    /// 600 points: first 300 in `calm` earning +0.05%/period, next 300 in `crash`
    /// losing 0.1%/period.
    fn curve(drift_calm: f64, drift_crash: f64) -> (Vec<(DateTime<Utc>, f64)>, Vec<RegimeWindow>) {
        let mut e = 100.0;
        let mut v = vec![(t(0), e)];
        for i in 1..600 {
            let d = if i < 300 { drift_calm } else { drift_crash };
            e *= 1.0 + d + noise(i);
            v.push((t(i as i64), e));
        }
        let w = vec![
            RegimeWindow {
                start: t(0),
                end: t(300),
                label: "calm".into(),
            },
            RegimeWindow {
                start: t(300),
                end: t(601),
                label: "crash".into(),
            },
        ];
        (v, w)
    }

    /// The planted failing regime is found, named, and survives correction.
    #[test]
    fn a_losing_regime_is_found_after_correction() {
        let (e, w) = curve(0.0005, -0.001);
        let r = search(&e, &w, 250, 0.05).unwrap();
        assert_eq!(r.worst.as_deref(), Some("crash"));
        assert!(r.failure_found, "{r:?}");
        assert_eq!(r.n_tested, 2);
    }

    /// A uniformly profitable run has a worst regime and no failure: the tool
    /// does not manufacture one.
    #[test]
    fn a_profitable_run_has_a_worst_regime_but_no_failure() {
        let (e, w) = curve(0.0006, 0.0004);
        let r = search(&e, &w, 250, 0.05).unwrap();
        assert!(r.worst.is_some());
        assert!(!r.failure_found, "{r:?}");
    }

    /// A regime with too few periods is listed as untested, never as survived.
    #[test]
    fn a_thin_regime_is_untested_not_survived() {
        let (e, mut w) = curve(0.0005, -0.001);
        w[1] = RegimeWindow {
            start: t(300),
            end: t(340),
            label: "crash".into(),
        };
        let r = search(&e, &w, 250, 0.05).unwrap();
        assert_eq!(r.untested, vec!["crash".to_string()]);
        assert!(r.unlabelled_share > 0.4);
        assert!(!r.failure_found);
    }

    #[test]
    fn no_windows_is_an_error_not_a_clean_bill() {
        let (e, _) = curve(0.0, 0.0);
        assert_eq!(
            search(&e, &[], 250, 0.05),
            Err(RegimeSearchError::NoWindows)
        );
    }

    #[test]
    fn the_normal_cdf_is_right_where_it_matters() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-6);
        assert!((norm_cdf(-1.96) - 0.025).abs() < 1e-4);
    }
}
