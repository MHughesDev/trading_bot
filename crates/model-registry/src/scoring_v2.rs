//! Scoring additions and the hard usage rules (Set N, FEAT-006 §7; MD-09, MD-10).
//!
//! The Python `scoring.py` the trainer uses already has pinball, CRPS, PIT,
//! Diebold-Mariano and the coverage tests. This file adds what FEAT-006 §7 asks for
//! and does it in Rust, because the **usage rules** below run in the validator and
//! the gate, which are Rust, and a verdict must not need a sidecar call (ADR-N-04).
//!
//! # Every function refuses rather than guesses
//!
//! Too few observations, mismatched lengths, a non-finite value, a non-positive
//! variance and a degenerate regression each return a typed [`ScoreError`]. None
//! returns NaN or a neutral number: a statistic that cannot be computed is not a
//! statistic that came out zero, and a downstream rule that reads NaN as "no
//! evidence against" has passed a model on the strength of an arithmetic accident.
//!
//! # The usage rules, and why they are a type
//!
//! A forecast is allowed to *size a position* only if it earned it:
//!
//! * a **return or direction** forecast needs a walk-forward out-of-sample
//!   R-squared above zero *and* a Clark-West p-value under 0.05 against the
//!   historical mean ([`return_forecast_rule`]);
//! * a **volatility** forecast must beat HARQ on QLIKE ([`vol_forecast_rule`]);
//! * a forecast that supplies `var_95` or `es_95` needs a **passed ES backtest**
//!   ([`tail_risk_rule`]).
//!
//! A measurement that is missing is [`RuleVerdict::NotComputable`], and
//! `NotComputable` blocks (ADR-N-05). [`BindingVerdict`] has private fields and one
//! constructor, [`judge_binding`], so the only way to hold an "allowed" is to have
//! put every rule in play through the rules above; there is no `from(bool)`.

use backtest::rng::DetRng;
use backtest::stats::{inverse_normal_cdf, normal_cdf};
use serde::Serialize;

/// Why a statistic was not produced.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ScoreError {
    #[error("need at least {need} observations, got {got}")]
    TooShort { need: usize, got: usize },
    #[error("the series have different lengths ({a} and {b})")]
    LengthMismatch { a: usize, b: usize },
    #[error("a value is not finite")]
    NonFinite,
    #[error("{0} must be positive")]
    NonPositive(&'static str),
    #[error("degenerate: {0}")]
    Degenerate(&'static str),
    #[error("alpha must be in (0, 0.5)")]
    BadAlpha,
}

fn finite(xs: &[f64]) -> Result<(), ScoreError> {
    if xs.iter().all(|x| x.is_finite()) {
        Ok(())
    } else {
        Err(ScoreError::NonFinite)
    }
}

fn same_len(a: &[f64], b: &[f64]) -> Result<(), ScoreError> {
    if a.len() == b.len() {
        Ok(())
    } else {
        Err(ScoreError::LengthMismatch {
            a: a.len(),
            b: b.len(),
        })
    }
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// The expanding historical-mean forecast of each observation from the ones before it:
/// `out[t] = mean(y[..t])`, undefined for `t < min_train`, which are left out.
/// Returns `(first_scored_index, forecasts)`.
///
/// # Errors
/// `min_train` of zero, or fewer than `min_train + 1` observations.
pub fn expanding_mean_benchmark(
    y: &[f64],
    min_train: usize,
) -> Result<(usize, Vec<f64>), ScoreError> {
    if min_train == 0 {
        return Err(ScoreError::NonPositive("min_train"));
    }
    if y.len() <= min_train {
        return Err(ScoreError::TooShort {
            need: min_train + 1,
            got: y.len(),
        });
    }
    finite(y)?;
    let mut sum: f64 = y[..min_train].iter().sum();
    let mut out = Vec::with_capacity(y.len() - min_train);
    for (t, yt) in y.iter().enumerate().skip(min_train) {
        out.push(sum / t as f64);
        sum += yt;
    }
    Ok((min_train, out))
}

// ── Campbell-Thompson out-of-sample R-squared ────────────────────────────────

/// `1 - SSE(model) / SSE(benchmark)`, the Campbell-Thompson (2008) out-of-sample
/// R-squared against the historical-mean benchmark.
///
/// Positive means the model beat the benchmark. A model that merely reproduces the
/// benchmark scores exactly zero.
///
/// # Errors
/// Mismatched, non-finite or too-short inputs, or a benchmark that is exact.
pub fn oos_r2(actual: &[f64], model: &[f64], benchmark: &[f64]) -> Result<f64, ScoreError> {
    same_len(actual, model)?;
    same_len(actual, benchmark)?;
    if actual.len() < 10 {
        return Err(ScoreError::TooShort {
            need: 10,
            got: actual.len(),
        });
    }
    finite(actual)?;
    finite(model)?;
    finite(benchmark)?;
    let sse = |f: &[f64]| -> f64 { actual.iter().zip(f).map(|(y, f)| (y - f).powi(2)).sum() };
    let denom = sse(benchmark);
    if denom <= 0.0 {
        return Err(ScoreError::Degenerate(
            "the benchmark forecasts every observation exactly",
        ));
    }
    Ok(1.0 - sse(model) / denom)
}

// ── Clark-West ───────────────────────────────────────────────────────────────

/// The Clark-West (2007) adjusted-MSPE test of a nested model against its restriction.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ClarkWest {
    pub statistic: f64,
    /// One-sided: the probability of a statistic this large if the restricted model is right.
    pub p_value: f64,
    pub n: usize,
}

/// `restricted` is the benchmark (for example the historical mean) and `unrestricted`
/// the model that nests it. `lags` is the Newey-West truncation, `h - 1` for an
/// `h`-step forecast and zero for one step.
///
/// The adjustment `(restricted - unrestricted)^2` removes the noise a larger nested
/// model adds under the null, which is what makes plain DM over-conservative here.
///
/// # Errors
/// Mismatched, non-finite or too-short inputs, or an adjusted loss differential with no variance.
pub fn clark_west(
    actual: &[f64],
    restricted: &[f64],
    unrestricted: &[f64],
    lags: usize,
) -> Result<ClarkWest, ScoreError> {
    same_len(actual, restricted)?;
    same_len(actual, unrestricted)?;
    let n = actual.len();
    if n < 20 {
        return Err(ScoreError::TooShort { need: 20, got: n });
    }
    finite(actual)?;
    finite(restricted)?;
    finite(unrestricted)?;
    let f: Vec<f64> = (0..n)
        .map(|t| {
            let e1 = actual[t] - restricted[t];
            let e2 = actual[t] - unrestricted[t];
            e1 * e1 - (e2 * e2 - (restricted[t] - unrestricted[t]).powi(2))
        })
        .collect();
    let m = mean(&f);
    let lrv = newey_west_variance(&f, lags);
    if lrv <= 0.0 {
        return Err(ScoreError::Degenerate(
            "the adjusted loss differential has no variance",
        ));
    }
    let statistic = m / (lrv / n as f64).sqrt();
    Ok(ClarkWest {
        statistic,
        p_value: 1.0 - normal_cdf(statistic),
        n,
    })
}

/// Long-run variance of a series by the Bartlett-kernel Newey-West estimator.
#[allow(clippy::many_single_char_names)] // Newey-West notation: x, m, d, k, t, v
fn newey_west_variance(x: &[f64], lags: usize) -> f64 {
    let n = x.len();
    let m = mean(x);
    let d: Vec<f64> = x.iter().map(|v| v - m).collect();
    let gamma = |k: usize| -> f64 { (k..n).map(|t| d[t] * d[t - k]).sum::<f64>() / n as f64 };
    let mut v = gamma(0);
    for k in 1..=lags.min(n - 1) {
        let w = 1.0 - k as f64 / (lags as f64 + 1.0);
        v += 2.0 * w * gamma(k);
    }
    v
}

// ── QLIKE and variance MSE ───────────────────────────────────────────────────

/// Mean QLIKE loss `rv/f - ln(rv/f) - 1`: zero when the forecast is exact, and
/// minimised in expectation by the true conditional variance, which is why it is the
/// robust loss for ranking volatility forecasts (Patton 2011).
///
/// # Errors
/// Mismatched, empty, non-finite or non-positive inputs.
pub fn qlike(realised_var: &[f64], forecast_var: &[f64]) -> Result<f64, ScoreError> {
    same_len(realised_var, forecast_var)?;
    if realised_var.is_empty() {
        return Err(ScoreError::TooShort { need: 1, got: 0 });
    }
    finite(realised_var)?;
    finite(forecast_var)?;
    if realised_var.iter().any(|v| *v <= 0.0) {
        return Err(ScoreError::NonPositive("realised variance"));
    }
    if forecast_var.iter().any(|v| *v <= 0.0) {
        return Err(ScoreError::NonPositive("forecast variance"));
    }
    let s: f64 = realised_var
        .iter()
        .zip(forecast_var)
        .map(|(r, f)| {
            let q = r / f;
            q - q.ln() - 1.0
        })
        .sum();
    Ok(s / realised_var.len() as f64)
}

/// Mean squared error on variances.
///
/// # Errors
/// Mismatched, empty or non-finite inputs.
pub fn variance_mse(realised_var: &[f64], forecast_var: &[f64]) -> Result<f64, ScoreError> {
    same_len(realised_var, forecast_var)?;
    if realised_var.is_empty() {
        return Err(ScoreError::TooShort { need: 1, got: 0 });
    }
    finite(realised_var)?;
    finite(forecast_var)?;
    Ok(realised_var
        .iter()
        .zip(forecast_var)
        .map(|(r, f)| (r - f).powi(2))
        .sum::<f64>()
        / realised_var.len() as f64)
}

// ── Mincer-Zarnowitz ─────────────────────────────────────────────────────────

/// `actual = alpha + beta * forecast + e`, and whether `(alpha, beta) = (0, 1)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct MincerZarnowitz {
    pub alpha: f64,
    pub beta: f64,
    pub r2: f64,
    /// Joint Wald test of `(0, 1)` with homoskedastic errors; chi-square with 2 d.f.,
    /// whose survival function is `exp(-W / 2)` exactly.
    pub wald: f64,
    pub p_value: f64,
    pub n: usize,
}

impl MincerZarnowitz {
    /// The recalibrated forecast `alpha + beta * f`, which strips a scaling artefact.
    #[must_use]
    pub fn recalibrate(&self, forecast: f64) -> f64 {
        self.alpha + self.beta * forecast
    }
}

/// # Errors
/// Mismatched, non-finite or too-short inputs, or a constant forecast.
pub fn mincer_zarnowitz(actual: &[f64], forecast: &[f64]) -> Result<MincerZarnowitz, ScoreError> {
    same_len(actual, forecast)?;
    let n = actual.len();
    if n < 10 {
        return Err(ScoreError::TooShort { need: 10, got: n });
    }
    finite(actual)?;
    finite(forecast)?;
    let nf = n as f64;
    let (my, mx) = (mean(actual), mean(forecast));
    let sxx: f64 = forecast.iter().map(|x| (x - mx).powi(2)).sum();
    if sxx <= 0.0 {
        return Err(ScoreError::Degenerate(
            "the forecast is constant, so it has no slope",
        ));
    }
    let sxy: f64 = forecast
        .iter()
        .zip(actual)
        .map(|(x, y)| (x - mx) * (y - my))
        .sum();
    let beta = sxy / sxx;
    let alpha = my - beta * mx;
    let sse: f64 = forecast
        .iter()
        .zip(actual)
        .map(|(x, y)| (y - alpha - beta * x).powi(2))
        .sum();
    let syy: f64 = actual.iter().map(|y| (y - my).powi(2)).sum();
    let r2 = if syy > 0.0 { 1.0 - sse / syy } else { 0.0 };
    let s2 = sse / (nf - 2.0);
    // cov = s2 * (X'X)^-1 with X = [1, x].
    let sum_x2: f64 = forecast.iter().map(|x| x * x).sum();
    let det = nf * sxx; // n * sum (x - mx)^2
    let (v00, v01, v11) = (s2 * sum_x2 / det, -s2 * (nf * mx) / det, s2 * nf / det);
    let (da, db) = (alpha, beta - 1.0);
    let det_v = v00 * v11 - v01 * v01;
    if det_v <= 0.0 {
        return Err(ScoreError::Degenerate(
            "the coefficient covariance is singular",
        ));
    }
    let wald = (v11 * da * da - 2.0 * v01 * da * db + v00 * db * db) / det_v;
    Ok(MincerZarnowitz {
        alpha,
        beta,
        r2,
        wald,
        p_value: (-wald / 2.0).exp(),
        n,
    })
}

// ── Expected-shortfall backtests (Acerbi-Szekely) and the Fissler-Ziegel loss ─

/// A verdict on a tail-risk forecast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum EsVerdict {
    Pass,
    Fail,
}

/// Acerbi-Szekely (2014) statistics in the **loss** convention: a loss is `-return`,
/// `VaR` and `ES` are positive loss levels, and a positive statistic means the tail
/// was **understated**. Both are zero in expectation when the forecasts are right.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct EsBacktest {
    /// Mean of `loss / ES` over the exceedances, minus one. `None` with no exceedance.
    pub z1: Option<f64>,
    /// `sum(loss * 1{exceed} / ES) / (T * alpha) - 1`.
    pub z2: f64,
    pub n_exceed: usize,
    pub p_z2: f64,
    pub p_z1: Option<f64>,
    pub verdict: EsVerdict,
}

fn gaussian_var_es(mu: f64, sigma: f64, alpha: f64) -> (f64, f64) {
    let z = inverse_normal_cdf(alpha);
    let pdf = (-0.5 * z * z).exp() / (2.0 * std::f64::consts::PI).sqrt();
    // Loss convention: VaR and ES are positive for a left-tail level alpha.
    (-(mu + sigma * z), -mu + sigma * pdf / alpha)
}

fn es_stats(loss: &[f64], var: &[f64], es: &[f64], alpha: f64) -> (Option<f64>, f64, usize) {
    let t = loss.len() as f64;
    let mut sum_ratio = 0.0;
    let mut n_exc = 0usize;
    for i in 0..loss.len() {
        if loss[i] > var[i] {
            n_exc += 1;
            sum_ratio += loss[i] / es[i];
        }
    }
    let z1 = (n_exc > 0).then(|| sum_ratio / n_exc as f64 - 1.0);
    let z2 = sum_ratio / (t * alpha) - 1.0;
    (z1, z2, n_exc)
}

fn std_normal(rng: &mut DetRng) -> f64 {
    // Box-Muller; u1 is kept away from zero so the log is finite.
    let u1 = rng.next_f64().max(1e-300);
    let u2 = rng.next_f64();
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

/// ES backtest of a **Gaussian** predictive distribution, with Monte-Carlo p-values.
///
/// `mu` and `sigma` are the forecast's per-period mean and standard deviation; VaR
/// and ES at level `alpha` follow from them. The p-values come from simulating the
/// statistics under the null that returns really are drawn from those Gaussians,
/// which is the null Acerbi and Szekely specify (their statistics have no
/// closed-form distribution). The verdict fails at one-sided `p_z2 < 0.05`: a
/// forecast whose tail is understated more than chance explains.
///
/// # Errors
/// Mismatched, non-finite or too-short inputs, a non-positive sigma or a bad alpha.
pub fn es_backtest_gaussian(
    returns: &[f64],
    mu: &[f64],
    sigma: &[f64],
    alpha: f64,
    sims: usize,
    seed: u64,
) -> Result<EsBacktest, ScoreError> {
    if !(alpha > 0.0 && alpha < 0.5) {
        return Err(ScoreError::BadAlpha);
    }
    same_len(returns, mu)?;
    same_len(returns, sigma)?;
    if returns.len() < 50 {
        return Err(ScoreError::TooShort {
            need: 50,
            got: returns.len(),
        });
    }
    if sims < 200 {
        return Err(ScoreError::TooShort {
            need: 200,
            got: sims,
        });
    }
    finite(returns)?;
    finite(mu)?;
    finite(sigma)?;
    if sigma.iter().any(|s| *s <= 0.0) {
        return Err(ScoreError::NonPositive("sigma"));
    }
    let n = returns.len();
    let (mut var, mut es) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let (v, e) = gaussian_var_es(mu[i], sigma[i], alpha);
        var.push(v);
        es.push(e);
    }
    let loss: Vec<f64> = returns.iter().map(|r| -r).collect();
    let (z1, z2, n_exceed) = es_stats(&loss, &var, &es, alpha);

    let mut rng = DetRng::new(seed);
    let (mut ge2, mut ge1, mut have1) = (0usize, 0usize, 0usize);
    let mut sim_loss = vec![0.0; n];
    for _ in 0..sims {
        for i in 0..n {
            sim_loss[i] = -(mu[i] + sigma[i] * std_normal(&mut rng));
        }
        let (s1, s2, _) = es_stats(&sim_loss, &var, &es, alpha);
        if s2 >= z2 {
            ge2 += 1;
        }
        if let (Some(a), Some(b)) = (s1, z1) {
            have1 += 1;
            if a >= b {
                ge1 += 1;
            }
        }
    }
    // The +1 keeps a p-value off exactly zero: a simulation cannot prove impossibility.
    let p_z2 = (ge2 as f64 + 1.0) / (sims as f64 + 1.0);
    let p_z1 = z1.map(|_| (ge1 as f64 + 1.0) / (have1 as f64 + 1.0));
    Ok(EsBacktest {
        z1,
        z2,
        n_exceed,
        p_z2,
        p_z1,
        verdict: if p_z2 < 0.05 {
            EsVerdict::Fail
        } else {
            EsVerdict::Pass
        },
    })
}

/// Mean Fissler-Ziegel (FZ0) joint loss for a `(VaR, ES)` forecast at level `alpha`
/// (Patton, Ziegel and Chen 2019). Returns, VaR and ES are in the **return**
/// convention: `VaR < 0` and `ES <= VaR`. Lower is better, and the loss is strictly
/// consistent: it is minimised in expectation only by the true pair.
///
/// # Errors
/// Mismatched, empty or non-finite inputs, a non-negative VaR or ES, `ES > VaR`, or a bad alpha.
pub fn fissler_ziegel(
    returns: &[f64],
    var: &[f64],
    es: &[f64],
    alpha: f64,
) -> Result<f64, ScoreError> {
    if !(alpha > 0.0 && alpha < 0.5) {
        return Err(ScoreError::BadAlpha);
    }
    same_len(returns, var)?;
    same_len(returns, es)?;
    if returns.is_empty() {
        return Err(ScoreError::TooShort { need: 1, got: 0 });
    }
    finite(returns)?;
    finite(var)?;
    finite(es)?;
    let mut total = 0.0;
    for i in 0..returns.len() {
        let (y, v, e) = (returns[i], var[i], es[i]);
        if v >= 0.0 || e >= 0.0 {
            return Err(ScoreError::NonPositive(
                "the magnitude of VaR and ES (they are negative returns)",
            ));
        }
        if e > v {
            return Err(ScoreError::Degenerate("ES must be at least as bad as VaR"));
        }
        let hit = if y <= v { 1.0 } else { 0.0 };
        total += -hit * (v - y) / (alpha * e) + v / e + (-e).ln() - 1.0;
    }
    Ok(total / returns.len() as f64)
}

// ── Economic value and the baseline ──────────────────────────────────────────

/// Mean-variance utility gain of a model's weights over a benchmark's:
/// `U(w) = mean(w r) - gamma / 2 * var(w r)`, returned as `U(model) - U(benchmark)`
/// per period. Positive means the forecast was worth acting on.
///
/// # Errors
/// Mismatched, non-finite or too-short inputs, or a non-positive risk aversion.
pub fn utility_gain(
    returns: &[f64],
    model_weights: &[f64],
    benchmark_weights: &[f64],
    gamma: f64,
) -> Result<f64, ScoreError> {
    same_len(returns, model_weights)?;
    same_len(returns, benchmark_weights)?;
    if returns.len() < 10 {
        return Err(ScoreError::TooShort {
            need: 10,
            got: returns.len(),
        });
    }
    if !(gamma.is_finite() && gamma > 0.0) {
        return Err(ScoreError::NonPositive("gamma"));
    }
    finite(returns)?;
    finite(model_weights)?;
    finite(benchmark_weights)?;
    let u = |w: &[f64]| -> f64 {
        let p: Vec<f64> = returns.iter().zip(w).map(|(r, w)| r * w).collect();
        let m = mean(&p);
        let var = p.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (p.len() as f64 - 1.0);
        m - gamma / 2.0 * var
    };
    Ok(u(model_weights) - u(benchmark_weights))
}

/// The vol-timed momentum baseline complex return models must beat (BACKTEST_SUITE
/// v2 T5): hold the sign of the trailing `lookback`-bar return, scaled to
/// `target_vol` by the trailing `vol_window`-bar standard deviation, capped at
/// `cap`. **Causal:** the position at `t` uses returns strictly before `t`.
///
/// Returns the strategy's per-bar returns from the first bar with a full history.
///
/// # Errors
/// Non-finite input, a zero window or too little history.
pub fn vol_timed_momentum(
    returns: &[f64],
    lookback: usize,
    vol_window: usize,
    target_vol_per_bar: f64,
    cap: f64,
) -> Result<Vec<f64>, ScoreError> {
    if lookback == 0 || vol_window < 2 {
        return Err(ScoreError::NonPositive("lookback and vol_window"));
    }
    if !(target_vol_per_bar.is_finite() && target_vol_per_bar > 0.0 && cap.is_finite() && cap > 0.0)
    {
        return Err(ScoreError::NonPositive("target_vol_per_bar and cap"));
    }
    finite(returns)?;
    let start = lookback.max(vol_window);
    if returns.len() <= start {
        return Err(ScoreError::TooShort {
            need: start + 1,
            got: returns.len(),
        });
    }
    let mut out = Vec::with_capacity(returns.len() - start);
    for t in start..returns.len() {
        let trend: f64 = returns[t - lookback..t].iter().sum();
        let window = &returns[t - vol_window..t];
        let m = mean(window);
        let sd = (window.iter().map(|x| (x - m).powi(2)).sum::<f64>()
            / (window.len() as f64 - 1.0))
            .sqrt();
        let scale = if sd > 0.0 {
            (target_vol_per_bar / sd).min(cap)
        } else {
            0.0
        };
        out.push(returns[t] * trend.signum() * scale);
    }
    Ok(out)
}

/// A return model that does not beat the vol-timed momentum baseline has not
/// justified its complexity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Complexity {
    Justified,
    /// Flagged `complexity_unjustified` in the scorecard.
    Unjustified,
}

/// Compare per-bar Sharpe ratios (mean over standard deviation).
///
/// # Errors
/// Fewer than ten observations in either, or a zero-variance series.
pub fn complexity_vs_baseline(model: &[f64], baseline: &[f64]) -> Result<Complexity, ScoreError> {
    let sharpe = |x: &[f64]| -> Result<f64, ScoreError> {
        if x.len() < 10 {
            return Err(ScoreError::TooShort {
                need: 10,
                got: x.len(),
            });
        }
        finite(x)?;
        let m = mean(x);
        let sd = (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len() as f64 - 1.0)).sqrt();
        if sd <= 0.0 {
            return Err(ScoreError::Degenerate(
                "a return series with no variance has no Sharpe ratio",
            ));
        }
        Ok(m / sd)
    };
    Ok(if sharpe(model)? > sharpe(baseline)? {
        Complexity::Justified
    } else {
        Complexity::Unjustified
    })
}

// ── The usage rules ──────────────────────────────────────────────────────────

/// What a rule concluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RuleVerdict {
    Pass,
    Fail {
        reason: String,
    },
    /// The evidence is missing. This blocks: an unmeasured model is not a passed one.
    NotComputable {
        reason: String,
    },
}

impl RuleVerdict {
    #[must_use]
    pub const fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Return or direction forecasts: out-of-sample R-squared **strictly** above zero and
/// Clark-West p **strictly** under 0.05.
#[must_use]
pub fn return_forecast_rule(oos_r2: Option<f64>, clark_west_p: Option<f64>) -> RuleVerdict {
    let (Some(r2), Some(p)) = (oos_r2, clark_west_p) else {
        return RuleVerdict::NotComputable {
            reason:
                "a return forecast needs both an out-of-sample R-squared and a Clark-West p-value"
                    .into(),
        };
    };
    if !(r2.is_finite() && p.is_finite()) {
        return RuleVerdict::NotComputable {
            reason: "the return-forecast statistics are not finite".into(),
        };
    }
    if r2 <= 0.0 {
        return RuleVerdict::Fail {
            reason: format!("out-of-sample R-squared {r2:.4} is not above zero"),
        };
    }
    if p >= 0.05 {
        return RuleVerdict::Fail {
            reason: format!("Clark-West p {p:.4} is not under 0.05"),
        };
    }
    RuleVerdict::Pass
}

/// Volatility forecasts must beat HARQ on QLIKE: **strictly** lower loss.
#[must_use]
pub fn vol_forecast_rule(model_qlike: Option<f64>, harq_qlike: Option<f64>) -> RuleVerdict {
    let (Some(m), Some(h)) = (model_qlike, harq_qlike) else {
        return RuleVerdict::NotComputable {
            reason: "a volatility forecast is judged against HARQ, and one of the two QLIKE scores is missing".into(),
        };
    };
    if !(m.is_finite() && h.is_finite()) {
        return RuleVerdict::NotComputable {
            reason: "a QLIKE score is not finite".into(),
        };
    }
    if m < h {
        RuleVerdict::Pass
    } else {
        RuleVerdict::Fail {
            reason: format!("QLIKE {m:.4} does not beat HARQ's {h:.4}"),
        }
    }
}

/// A `var_95` / `es_95` consumer needs a passed ES backtest.
#[must_use]
pub fn tail_risk_rule(es: Option<EsVerdict>) -> RuleVerdict {
    match es {
        None => RuleVerdict::NotComputable {
            reason: "sizing on var_95/es_95 needs an ES backtest and none was run".into(),
        },
        Some(EsVerdict::Pass) => RuleVerdict::Pass,
        Some(EsVerdict::Fail) => RuleVerdict::Fail {
            reason: "the ES backtest rejected the tail forecast".into(),
        },
    }
}

/// What a bound forecast is used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ForecastUse {
    Return,
    Vol,
    TailRisk,
}

/// The measurements the rules read. Anything not measured is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Evidence {
    pub oos_r2: Option<f64>,
    pub clark_west_p: Option<f64>,
    pub model_qlike: Option<f64>,
    pub harq_qlike: Option<f64>,
    pub es_backtest: Option<EsVerdict>,
}

/// The verdict on binding a forecast into a strategy that enters Gate 3.
///
/// Private fields and a single constructor ([`judge_binding`]): there is no way to
/// hold an allowed binding that did not pass through every rule it is subject to.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BindingVerdict {
    rules: Vec<(ForecastUse, RuleVerdict)>,
}

impl BindingVerdict {
    /// True only if there is at least one rule and every one passed.
    #[must_use]
    pub fn allowed(&self) -> bool {
        !self.rules.is_empty() && self.rules.iter().all(|(_, v)| v.is_pass())
    }

    #[must_use]
    pub fn rules(&self) -> &[(ForecastUse, RuleVerdict)] {
        &self.rules
    }

    /// Every reason a binding is refused, for the refusal message.
    #[must_use]
    pub fn reasons(&self) -> Vec<String> {
        self.rules
            .iter()
            .filter_map(|(u, v)| match v {
                RuleVerdict::Pass => None,
                RuleVerdict::Fail { reason } | RuleVerdict::NotComputable { reason } => {
                    Some(format!("{u:?}: {reason}"))
                }
            })
            .collect()
    }
}

/// Judge a binding. A binding that declares **no use** is not allowed: there is
/// nothing to say it was checked.
#[must_use]
pub fn judge_binding(uses: &[ForecastUse], evidence: &Evidence) -> BindingVerdict {
    let mut rules = Vec::new();
    for u in uses {
        let v = match u {
            ForecastUse::Return => return_forecast_rule(evidence.oos_r2, evidence.clark_west_p),
            ForecastUse::Vol => vol_forecast_rule(evidence.model_qlike, evidence.harq_qlike),
            ForecastUse::TailRisk => tail_risk_rule(evidence.es_backtest),
        };
        rules.push((*u, v));
    }
    BindingVerdict { rules }
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::many_single_char_names)]
mod tests {
    use super::*;

    fn normal_series(seed: u64, n: usize, sd: f64) -> Vec<f64> {
        let mut r = DetRng::new(seed);
        (0..n).map(|_| sd * std_normal(&mut r)).collect()
    }

    // ── benchmark and R-squared ──

    #[test]
    fn the_expanding_mean_uses_only_the_past() {
        let y = [1.0, 2.0, 3.0, 4.0, 10.0];
        let (start, f) = expanding_mean_benchmark(&y, 2).unwrap();
        assert_eq!(start, 2);
        assert_eq!(f, vec![1.5, 2.0, 2.5]); // mean of y[..2], y[..3], y[..4]
        assert!(expanding_mean_benchmark(&y, 0).is_err());
        assert!(expanding_mean_benchmark(&y, 5).is_err());
    }

    #[test]
    fn oos_r2_is_one_for_a_perfect_model_and_zero_for_the_benchmark() {
        let y = normal_series(1, 200, 1.0);
        let (s, b) = expanding_mean_benchmark(&y, 20).unwrap();
        let actual = &y[s..];
        assert!((oos_r2(actual, actual, &b).unwrap() - 1.0).abs() < 1e-12);
        assert_eq!(oos_r2(actual, &b, &b).unwrap(), 0.0);
        // Worse than the benchmark is negative.
        let bad: Vec<f64> = b.iter().map(|x| x + 5.0).collect();
        assert!(oos_r2(actual, &bad, &b).unwrap() < 0.0);
    }

    #[test]
    fn oos_r2_matches_a_hand_computed_value() {
        // y = [1..10], model = y + 1 (SSE 10), benchmark = 5.5 (SSE 82.5).
        let y: Vec<f64> = (1..=10).map(f64::from).collect();
        let model: Vec<f64> = y.iter().map(|v| v + 1.0).collect();
        let bench = vec![5.5; 10];
        let r2 = oos_r2(&y, &model, &bench).unwrap();
        assert!((r2 - (1.0 - 10.0 / 82.5)).abs() < 1e-12);
    }

    #[test]
    fn inputs_that_cannot_be_scored_are_errors_not_numbers() {
        let y = vec![1.0; 12];
        assert!(matches!(
            oos_r2(&y, &y[..5], &y),
            Err(ScoreError::LengthMismatch { .. })
        ));
        assert!(matches!(
            oos_r2(&y[..5], &y[..5], &y[..5]),
            Err(ScoreError::TooShort { .. })
        ));
        let mut nan = y.clone();
        nan[3] = f64::NAN;
        assert_eq!(oos_r2(&nan, &y, &y), Err(ScoreError::NonFinite));
        assert!(
            matches!(oos_r2(&y, &y, &y), Err(ScoreError::Degenerate(_))),
            "exact benchmark"
        );
    }

    // ── Clark-West ──

    /// Expanding OLS-through-origin forecast of y_t from x_t, as in a real nested comparison.
    fn nested_forecasts(x: &[f64], y: &[f64], train: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let (mut sxy, mut sxx) = (0.0, 0.0);
        for i in 0..train {
            sxy += x[i] * y[i];
            sxx += x[i] * x[i];
        }
        let (mut act, mut restricted, mut unrestricted) = (vec![], vec![], vec![]);
        for t in train..y.len() {
            let b = if sxx > 0.0 { sxy / sxx } else { 0.0 };
            act.push(y[t]);
            restricted.push(0.0); // the zero-mean restriction the larger model nests
            unrestricted.push(b * x[t]);
            sxy += x[t] * y[t];
            sxx += x[t] * x[t];
        }
        (act, restricted, unrestricted)
    }

    #[test]
    fn clark_west_has_power_against_real_predictability_and_size_under_noise() {
        let mut power = 0;
        let mut size = 0;
        let trials = 60;
        for seed in 0..trials {
            let x = normal_series(1000 + seed, 400, 1.0);
            let noise = normal_series(5000 + seed, 400, 1.0);
            let signal: Vec<f64> = x.iter().zip(&noise).map(|(a, e)| 0.3 * a + e).collect();
            let (a, r, u) = nested_forecasts(&x, &signal, 100);
            if clark_west(&a, &r, &u, 0).unwrap().p_value < 0.05 {
                power += 1;
            }
            let (a, r, u) = nested_forecasts(&x, &noise, 100);
            if clark_west(&a, &r, &u, 0).unwrap().p_value < 0.05 {
                size += 1;
            }
        }
        assert!(power >= 52, "power {power}/{trials}");
        assert!(
            size <= 9,
            "false-positive rate {size}/{trials} is far above 5%"
        );
    }

    #[test]
    fn clark_west_refuses_what_it_cannot_judge() {
        let y = vec![1.0; 30];
        assert!(matches!(
            clark_west(&y[..5], &y[..5], &y[..5], 0),
            Err(ScoreError::TooShort { .. })
        ));
        assert!(matches!(
            clark_west(&y, &y, &y, 0),
            Err(ScoreError::Degenerate(_))
        ));
    }

    // ── QLIKE ──

    #[test]
    fn qlike_is_minimised_by_the_mean_variance_and_is_zero_when_exact() {
        let rv: Vec<f64> = (1..=40)
            .map(|i| 0.0001 * (1.0 + (f64::from(i) * 0.7).sin().abs() * 3.0))
            .collect();
        let m = mean(&rv);
        let at = |k: f64| qlike(&rv, &vec![m * k; rv.len()]).unwrap();
        assert!(at(1.0) < at(0.8));
        assert!(at(1.0) < at(1.25));
        assert_eq!(qlike(&rv, &rv).unwrap(), 0.0);
        // It is asymmetric: under-forecasting variance is punished more than over-forecasting.
        assert!(at(0.5) > at(2.0));
    }

    #[test]
    fn qlike_refuses_non_positive_variance() {
        assert!(matches!(
            qlike(&[0.0], &[1.0]),
            Err(ScoreError::NonPositive(_))
        ));
        assert!(matches!(
            qlike(&[1.0], &[-1.0]),
            Err(ScoreError::NonPositive(_))
        ));
        assert!(matches!(qlike(&[], &[]), Err(ScoreError::TooShort { .. })));
        assert_eq!(variance_mse(&[1.0, 3.0], &[2.0, 2.0]).unwrap(), 1.0);
    }

    // ── Mincer-Zarnowitz ──

    #[test]
    fn mincer_zarnowitz_recovers_a_scaling_artefact_and_recalibrates() {
        let f = normal_series(9, 500, 1.0);
        let noise = normal_series(10, 500, 0.1);
        // The truth is 2 + 0.5 f: the forecast is mis-scaled.
        let y: Vec<f64> = f
            .iter()
            .zip(&noise)
            .map(|(f, e)| 2.0 + 0.5 * f + e)
            .collect();
        let mz = mincer_zarnowitz(&y, &f).unwrap();
        assert!((mz.alpha - 2.0).abs() < 0.05, "alpha {}", mz.alpha);
        assert!((mz.beta - 0.5).abs() < 0.05, "beta {}", mz.beta);
        assert!(mz.p_value < 1e-6, "(0,1) is rejected: p {}", mz.p_value);
        assert!((mz.recalibrate(1.0) - 2.5).abs() < 0.1);
    }

    #[test]
    fn mincer_zarnowitz_accepts_an_unbiased_forecast() {
        let mut ok = 0;
        for seed in 0..40 {
            let f = normal_series(300 + seed, 400, 1.0);
            let e = normal_series(900 + seed, 400, 0.5);
            let y: Vec<f64> = f.iter().zip(&e).map(|(f, e)| f + e).collect();
            if mincer_zarnowitz(&y, &f).unwrap().p_value > 0.05 {
                ok += 1;
            }
        }
        assert!(
            ok >= 33,
            "an unbiased forecast was rejected too often: accepted {ok}/40"
        );
        assert!(matches!(
            mincer_zarnowitz(&[1.0; 20], &[2.0; 20]),
            Err(ScoreError::Degenerate(_))
        ));
    }

    // ── ES backtest ──

    #[test]
    fn a_calibrated_gaussian_es_forecast_passes_and_an_understated_tail_fails() {
        let n = 1500;
        let sigma = vec![0.02; n];
        let mu = vec![0.0; n];
        let mut rng = DetRng::new(77);
        let returns: Vec<f64> = (0..n).map(|_| 0.02 * std_normal(&mut rng)).collect();
        let good = es_backtest_gaussian(&returns, &mu, &sigma, 0.05, 400, 1).unwrap();
        assert_eq!(good.verdict, EsVerdict::Pass, "{good:?}");
        assert!(good.z2.abs() < 0.5);

        // The same returns judged against a forecast that claims half the risk.
        let thin = vec![0.01; n];
        let bad = es_backtest_gaussian(&returns, &mu, &thin, 0.05, 400, 1).unwrap();
        assert_eq!(bad.verdict, EsVerdict::Fail, "{bad:?}");
        assert!(
            bad.z2 > good.z2,
            "a positive statistic means the tail was understated"
        );
        assert!(bad.p_z2 < 0.05);
    }

    #[test]
    fn the_es_backtest_is_reproducible_and_refuses_bad_input() {
        let n = 300;
        let r = normal_series(4, n, 0.02);
        let (mu, sg) = (vec![0.0; n], vec![0.02; n]);
        let a = es_backtest_gaussian(&r, &mu, &sg, 0.05, 300, 5).unwrap();
        let b = es_backtest_gaussian(&r, &mu, &sg, 0.05, 300, 5).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            es_backtest_gaussian(&r, &mu, &sg, 0.7, 300, 5),
            Err(ScoreError::BadAlpha)
        );
        assert!(matches!(
            es_backtest_gaussian(&r[..10], &mu[..10], &sg[..10], 0.05, 300, 5),
            Err(ScoreError::TooShort { .. })
        ));
        assert!(matches!(
            es_backtest_gaussian(&r, &mu, &vec![0.0; n], 0.05, 300, 5),
            Err(ScoreError::NonPositive(_))
        ));
    }

    #[test]
    fn the_fissler_ziegel_loss_is_minimised_by_the_true_var_es_pair() {
        let n = 20_000;
        let sd = 0.02;
        let alpha = 0.05;
        let r = normal_series(21, n, sd);
        // The returns-convention pair for a zero-mean Gaussian.
        let (loss_var, loss_es) = gaussian_var_es(0.0, sd, alpha);
        let (v, e) = (-loss_var, -loss_es);
        let at = |kv: f64, ke: f64| {
            fissler_ziegel(&r, &vec![v * kv; n], &vec![e * ke; n], alpha).unwrap()
        };
        let truth = at(1.0, 1.0);
        for (kv, ke) in [
            (0.7, 0.7),
            (1.4, 1.4),
            (1.0, 0.85),
            (1.0, 1.3),
            (0.8, 1.0),
            (1.2, 1.0),
        ] {
            assert!(
                at(kv, ke) > truth,
                "({kv}, {ke}) {} should exceed truth {truth}",
                at(kv, ke)
            );
        }
    }

    #[test]
    fn the_fissler_ziegel_loss_refuses_a_positive_or_inverted_pair() {
        assert!(
            fissler_ziegel(&[0.1], &[0.02], &[-0.03], 0.05).is_err(),
            "VaR must be negative"
        );
        assert!(matches!(
            fissler_ziegel(&[0.1], &[-0.03], &[-0.02], 0.05),
            Err(ScoreError::Degenerate(_))
        ));
        assert_eq!(
            fissler_ziegel(&[0.1], &[-0.03], &[-0.04], 0.9),
            Err(ScoreError::BadAlpha)
        );
    }

    // ── utility, baseline ──

    #[test]
    fn utility_gain_is_positive_for_weights_that_know_the_sign() {
        let r = normal_series(31, 400, 0.01);
        let informed: Vec<f64> = r.iter().map(|x| x.signum()).collect();
        let flat = vec![0.5; r.len()];
        assert!(utility_gain(&r, &informed, &flat, 3.0).unwrap() > 0.0);
        assert!(utility_gain(&r, &flat, &informed, 3.0).unwrap() < 0.0);
        assert_eq!(utility_gain(&r, &flat, &flat, 3.0).unwrap(), 0.0);
        assert!(utility_gain(&r, &flat, &flat, 0.0).is_err());
    }

    #[test]
    fn the_baseline_never_reads_the_bar_it_trades() {
        let mut r = normal_series(41, 300, 0.01);
        let a = vol_timed_momentum(&r, 20, 20, 0.01, 2.0).unwrap();
        // Change the LAST return: every earlier position is untouched.
        let last = r.len() - 1;
        r[last] = 0.5;
        let b = vol_timed_momentum(&r, 20, 20, 0.01, 2.0).unwrap();
        assert_eq!(a[..a.len() - 1], b[..b.len() - 1]);
        // Only the last bar's own P&L changes, because the position at t used history before t.
        assert_ne!(a[a.len() - 1], b[b.len() - 1]);
    }

    #[test]
    fn the_baseline_earns_on_a_trending_series_and_refuses_too_little_history() {
        let r: Vec<f64> = (0..400)
            .map(|i| 0.001 + 0.0005 * (f64::from(i * 13 % 7) - 3.0) / 3.0)
            .collect();
        let s = vol_timed_momentum(&r, 10, 20, 0.001, 3.0).unwrap();
        assert!(mean(&s) > 0.0);
        assert!(matches!(
            vol_timed_momentum(&r[..15], 10, 20, 0.001, 3.0),
            Err(ScoreError::TooShort { .. })
        ));
    }

    #[test]
    fn a_model_that_loses_to_the_baseline_is_flagged() {
        let base: Vec<f64> = (0..50).map(|i| 0.001 + 0.0002 * f64::from(i % 5)).collect();
        let worse: Vec<f64> = (0..50)
            .map(|i| -0.001 + 0.0002 * f64::from(i % 5))
            .collect();
        assert_eq!(
            complexity_vs_baseline(&worse, &base).unwrap(),
            Complexity::Unjustified
        );
        assert_eq!(
            complexity_vs_baseline(&base, &worse).unwrap(),
            Complexity::Justified
        );
        assert_eq!(
            complexity_vs_baseline(&base, &base).unwrap(),
            Complexity::Unjustified,
            "a tie is not a win"
        );
    }

    // ── the usage rules ──

    #[test]
    fn the_return_rule_is_strict_at_both_boundaries() {
        assert!(return_forecast_rule(Some(0.01), Some(0.049)).is_pass());
        assert!(
            matches!(
                return_forecast_rule(Some(0.0), Some(0.01)),
                RuleVerdict::Fail { .. }
            ),
            "R2 = 0 fails"
        );
        assert!(
            matches!(
                return_forecast_rule(Some(0.01), Some(0.05)),
                RuleVerdict::Fail { .. }
            ),
            "p = 0.05 fails"
        );
        assert!(matches!(
            return_forecast_rule(Some(-0.2), Some(0.001)),
            RuleVerdict::Fail { .. }
        ));
        assert!(matches!(
            return_forecast_rule(None, Some(0.01)),
            RuleVerdict::NotComputable { .. }
        ));
        assert!(matches!(
            return_forecast_rule(Some(0.1), None),
            RuleVerdict::NotComputable { .. }
        ));
        assert!(matches!(
            return_forecast_rule(Some(f64::NAN), Some(0.01)),
            RuleVerdict::NotComputable { .. }
        ));
    }

    #[test]
    fn the_vol_rule_needs_a_strict_win_over_harq_and_a_missing_score_blocks() {
        assert!(vol_forecast_rule(Some(0.10), Some(0.11)).is_pass());
        assert!(
            matches!(
                vol_forecast_rule(Some(0.11), Some(0.11)),
                RuleVerdict::Fail { .. }
            ),
            "a tie is a loss"
        );
        assert!(matches!(
            vol_forecast_rule(Some(0.12), Some(0.11)),
            RuleVerdict::Fail { .. }
        ));
        assert!(matches!(
            vol_forecast_rule(Some(0.10), None),
            RuleVerdict::NotComputable { .. }
        ));
    }

    #[test]
    fn the_tail_rule_needs_a_passed_es_backtest() {
        assert!(tail_risk_rule(Some(EsVerdict::Pass)).is_pass());
        assert!(matches!(
            tail_risk_rule(Some(EsVerdict::Fail)),
            RuleVerdict::Fail { .. }
        ));
        assert!(matches!(
            tail_risk_rule(None),
            RuleVerdict::NotComputable { .. }
        ));
    }

    #[test]
    fn a_binding_is_allowed_only_if_every_rule_in_play_passed() {
        let good = Evidence {
            oos_r2: Some(0.02),
            clark_west_p: Some(0.01),
            model_qlike: Some(0.1),
            harq_qlike: Some(0.2),
            es_backtest: Some(EsVerdict::Pass),
        };
        let all = [ForecastUse::Return, ForecastUse::Vol, ForecastUse::TailRisk];
        assert!(judge_binding(&all, &good).allowed());

        // A vol model losing to HARQ is blocked from sizing (M6).
        let mut lose = good;
        lose.model_qlike = Some(0.3);
        let v = judge_binding(&[ForecastUse::Vol], &lose);
        assert!(!v.allowed());
        assert!(v.reasons()[0].contains("HARQ"));

        // A model without an ES backtest is blocked from ES sizing (M6).
        let mut none = good;
        none.es_backtest = None;
        assert!(!judge_binding(&[ForecastUse::TailRisk], &none).allowed());
        // ...but the same evidence is fine for a use that does not need it.
        assert!(judge_binding(&[ForecastUse::Return], &none).allowed());

        // Nothing declared is nothing checked.
        assert!(!judge_binding(&[], &good).allowed());
        // One failing rule among several refuses the lot.
        assert!(!judge_binding(&all, &lose).allowed());
        assert_eq!(judge_binding(&all, &lose).reasons().len(), 1);
    }
}
