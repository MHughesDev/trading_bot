//! Quality tests for analysis-code skills (Set R-a RA-3.5; AGENT-003 §6.3 step 6).
//!
//! A skill that transforms a series is only as good as its refusal to look ahead and
//! its refusal to find structure that is not there. Both are properties that can be
//! *measured by running it*:
//!
//! * [`truncation_check`] runs the skill on the full series and again on the series
//!   cut at several points, and requires every output up to each cut to be identical.
//!   A skill that back-fills a gap, normalises by the full-sample mean or smooths with a
//!   centred window changes an *earlier* answer when later data arrives, and the report
//!   names the first timestamp whose answer changed (AGENT-003 acceptance 4).
//! * [`noise_null`] runs it on white noise and counts how often it reports significance.
//!   A test with a 5% nominal level that finds structure in more than 7% of noise series
//!   is not a 5% test ([`noise_null_verdict`]).
//!
//! Both run the skill through [`crate::runner`], so the sandbox that bounds an
//! agent-authored script (no network, bounded memory and time) bounds these runs too.
//! The deterministic part, the generator of noise and the pass rule, is pure and tested
//! without Python; the end-to-end run is in `tests/quality_live.rs` and needs `python`.

use serde_json::{json, Value};

use crate::runner::{run, RunnerConfig};
use crate::{Outcome, Skill};

/// A false-positive rate above this, at a 5% nominal level, fails (AGENT-003 §6.3).
pub const MAX_NOISE_FALSE_POSITIVE_RATE: f64 = 0.07;
/// The nominal level the skill's p-value is judged at.
pub const NOMINAL_LEVEL: f64 = 0.05;

/// Which payload keys a series skill reads and writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesSpec {
    pub ts_key: String,
    pub series_key: String,
    /// The output key holding one value per input row (truncation).
    pub out_key: String,
    /// The output key holding a p-value (noise null).
    pub p_key: String,
}

impl Default for SeriesSpec {
    fn default() -> Self {
        Self {
            ts_key: "ts".into(),
            series_key: "values".into(),
            out_key: "result".into(),
            p_key: "p_value".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum QualityError {
    #[error("the skill did not run cleanly ({0}); a test of what it computes needs it to compute")]
    DidNotRun(String),
    #[error("the skill's output `{0}` is not a list of numbers aligned to its input rows")]
    BadOutput(String),
    #[error("{0}")]
    BadInput(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TruncationReport {
    pub passed: bool,
    pub cuts_checked: usize,
    /// The timestamp whose answer changed when later data was removed.
    pub first_differing_ts: Option<i64>,
    pub cut_ts: Option<i64>,
    pub full: Option<f64>,
    pub truncated: Option<f64>,
}

fn column(result: &Value, key: &str) -> Result<Vec<Option<f64>>, QualityError> {
    let arr = result
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| QualityError::BadOutput(key.to_string()))?;
    arr.iter()
        .map(|v| match v {
            Value::Null => Ok(None),
            other => other
                .as_f64()
                .map(|x| if x.is_nan() { None } else { Some(x) })
                .ok_or_else(|| QualityError::BadOutput(key.to_string())),
        })
        .collect()
}

async fn run_series(
    skill: &Skill,
    cfg: &RunnerConfig,
    spec: &SeriesSpec,
    ts: &[i64],
    x: &[f64],
) -> Result<Value, QualityError> {
    let payload = json!({ spec.ts_key.as_str(): ts, spec.series_key.as_str(): x });
    let done = run(skill, &payload, cfg).await;
    if done.record.outcome != Outcome::Ok {
        return Err(QualityError::DidNotRun(format!(
            "{:?}",
            done.record.outcome
        )));
    }
    done.result
        .ok_or_else(|| QualityError::DidNotRun("no result".into()))
}

/// Cut points spread across the series, leaving a warm-up and a tail.
#[must_use]
pub fn default_cuts(n: usize, k: usize) -> Vec<usize> {
    if n < 20 || k == 0 {
        return Vec::new();
    }
    let (lo, hi) = (n / 5, n - 2);
    (1..=k).map(|i| lo + (hi - lo) * i / (k + 1)).collect()
}

/// Run the skill on `x` and on each prefix `x[..=cut]` and require every answer up to
/// the cut to be identical.
///
/// # Errors
/// [`QualityError`] when the skill does not run or does not return an aligned list.
pub async fn truncation_check(
    skill: &Skill,
    cfg: &RunnerConfig,
    spec: &SeriesSpec,
    ts: &[i64],
    x: &[f64],
    cuts: &[usize],
) -> Result<TruncationReport, QualityError> {
    if ts.len() != x.len() || cuts.is_empty() || cuts.iter().any(|c| *c + 1 >= x.len()) {
        return Err(QualityError::BadInput(
            "the series and its cut points must be aligned and inside it".into(),
        ));
    }
    let full = column(&run_series(skill, cfg, spec, ts, x).await?, &spec.out_key)?;
    if full.len() != x.len() {
        return Err(QualityError::BadOutput(spec.out_key.clone()));
    }
    for &c in cuts {
        let part = column(
            &run_series(skill, cfg, spec, &ts[..=c], &x[..=c]).await?,
            &spec.out_key,
        )?;
        if part.len() != c + 1 {
            return Err(QualityError::BadOutput(spec.out_key.clone()));
        }
        for i in 0..=c {
            if full[i] != part[i] {
                return Ok(TruncationReport {
                    passed: false,
                    cuts_checked: cuts.len(),
                    first_differing_ts: Some(ts[i]),
                    cut_ts: Some(ts[c]),
                    full: full[i],
                    truncated: part[i],
                });
            }
        }
    }
    Ok(TruncationReport {
        passed: true,
        cuts_checked: cuts.len(),
        first_differing_ts: None,
        cut_ts: None,
        full: None,
        truncated: None,
    })
}

/// A seeded standard-normal series (xorshift64* and Box-Muller), the same on every
/// machine, so a skill's noise verdict is reproducible.
#[must_use]
pub fn white_noise(n: usize, seed: u64) -> Vec<f64> {
    let mut s = seed ^ 0x9E37_79B9_7F4A_7C15 | 1;
    let mut next = move || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..n)
        .map(|_| {
            let (u1, u2) = (next().max(1e-12), next());
            (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct NoiseReport {
    pub trials: u32,
    pub significant: u32,
    pub false_positive_rate: f64,
    pub passed: bool,
}

/// The pass rule: at most 7% significant on noise at a 5% nominal level.
#[must_use]
pub fn noise_null_verdict(significant: u32, trials: u32) -> NoiseReport {
    let rate = if trials == 0 {
        f64::NAN
    } else {
        f64::from(significant) / f64::from(trials)
    };
    NoiseReport {
        trials,
        significant,
        false_positive_rate: rate,
        passed: trials > 0 && rate <= MAX_NOISE_FALSE_POSITIVE_RATE,
    }
}

/// Run the skill on `trials` independent white-noise series of length `n`.
///
/// # Errors
/// [`QualityError`] when the skill does not run or returns no p-value.
pub async fn noise_null(
    skill: &Skill,
    cfg: &RunnerConfig,
    spec: &SeriesSpec,
    trials: u32,
    n: usize,
    seed: u64,
) -> Result<NoiseReport, QualityError> {
    let ts: Vec<i64> = (0..n as i64).collect();
    let mut significant = 0;
    for t in 0..trials {
        let x = white_noise(n, seed.wrapping_add(u64::from(t) * 7919));
        let result = run_series(skill, cfg, spec, &ts, &x).await?;
        let p = result
            .get(&spec.p_key)
            .and_then(Value::as_f64)
            .ok_or_else(|| QualityError::BadOutput(spec.p_key.clone()))?;
        if p < NOMINAL_LEVEL {
            significant += 1;
        }
    }
    Ok(noise_null_verdict(significant, trials))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noise_generator_is_reproducible_and_looks_standard_normal() {
        let a = white_noise(4000, 5);
        assert_eq!(a, white_noise(4000, 5));
        assert_ne!(a, white_noise(4000, 6));
        let m = a.iter().sum::<f64>() / a.len() as f64;
        let v = a.iter().map(|x| (x - m).powi(2)).sum::<f64>() / a.len() as f64;
        assert!(m.abs() < 0.06, "{m}");
        assert!((v - 1.0).abs() < 0.08, "{v}");
    }

    #[test]
    fn a_five_percent_test_passes_and_a_ten_percent_one_fails() {
        assert!(noise_null_verdict(25, 500).passed, "5.0%");
        assert!(
            noise_null_verdict(35, 500).passed,
            "7.0% is at the limit and allowed"
        );
        assert!(!noise_null_verdict(36, 500).passed, "7.2%");
        let bad = noise_null_verdict(50, 500);
        assert!(!bad.passed);
        assert!((bad.false_positive_rate - 0.10).abs() < 1e-12);
        assert!(!noise_null_verdict(0, 0).passed, "no trials is not a pass");
    }

    #[test]
    fn cuts_leave_a_warm_up_and_a_tail_and_short_series_get_none() {
        let c = default_cuts(100, 4);
        assert_eq!(c.len(), 4);
        assert!(c.iter().all(|&i| i >= 20 && i + 1 < 100));
        assert!(c.windows(2).all(|w| w[0] < w[1]));
        assert!(default_cuts(10, 4).is_empty());
    }
}
