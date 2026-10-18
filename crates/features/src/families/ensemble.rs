//! Lookback ensembles (S-5.14; DATA-006 §5 "Ensemble", AGENT-011 §2 F2).
//!
//! An equal-weight mean of one feature over several lookbacks. A rule that hangs
//! on a single window — `zscore_20 < -2` — is a rule whose result depends on the
//! 20, and the FEAT-004 §6 move `lookback_ensemble` replaces it with the mean
//! over three nearby windows so that it does not. DATA-006 writes it over any
//! feature `f`; the registry cannot (a feature cannot take a feature as a
//! parameter — see `families/mod.rs`), so it is registered for the two bases a
//! strategy actually ensembles: the z-score and the simple return.
//!
//! Each term is the same arithmetic as the registered single-window family, over
//! a prefix of the same newest-first window, so an ensemble of one lookback is
//! exactly `zscore_n` / `returns_n`. That is tested, because it is the property
//! that keeps this from being a second implementation of either.

/// Sample mean.
#[allow(clippy::cast_precision_loss)]
fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Sample standard deviation (ddof = 1), `NaN` for fewer than two values —
/// `runtime::sample_std`'s definition.
#[allow(clippy::cast_precision_loss)]
fn sample_std(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return f64::NAN;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

/// How many sigma the decision close is from its `n`-bar mean — `zscore_n`.
fn zscore(closes: &[f64], n: usize) -> f64 {
    let Some(w) = closes.get(..n) else {
        return f64::NAN;
    };
    let sd = sample_std(w);
    if sd == 0.0 || sd.is_nan() {
        f64::NAN
    } else {
        (w[0] - mean(w)) / sd
    }
}

/// Simple return over `n` bars — `returns_n`.
fn simple_return(closes: &[f64], n: usize) -> f64 {
    match closes.get(n) {
        Some(base) if *base != 0.0 => closes[0] / base - 1.0,
        _ => f64::NAN,
    }
}

/// The equal-weight mean of `zscore` over `lookbacks`.
///
/// `closes` is newest first and at least `max(lookbacks)` long. `NaN` when any
/// term is `NaN`: a mean over the terms that happened to exist would be a
/// different number wearing the same name.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn mean_zscore(closes: &[f64], lookbacks: &[usize]) -> f64 {
    if lookbacks.is_empty() {
        return f64::NAN;
    }
    lookbacks.iter().map(|n| zscore(closes, *n)).sum::<f64>() / lookbacks.len() as f64
}

/// The equal-weight mean of simple returns over `lookbacks`. `closes` is newest
/// first and at least `max(lookbacks) + 1` long.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn mean_return(closes: &[f64], lookbacks: &[usize]) -> f64 {
    if lookbacks.is_empty() {
        return f64::NAN;
    }
    lookbacks
        .iter()
        .map(|n| simple_return(closes, *n))
        .sum::<f64>()
        / lookbacks.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closes() -> Vec<f64> {
        // Newest first.
        (0..80)
            .map(|i| 100.0 + (f64::from(i) * 0.7).sin() * 3.0 - f64::from(i) * 0.05)
            .collect()
    }

    #[test]
    fn an_ensemble_of_one_lookback_is_the_single_window_family() {
        let c = closes();
        assert_eq!(mean_zscore(&c, &[20]), zscore(&c, 20));
        assert_eq!(mean_return(&c, &[5]), simple_return(&c, 5));
    }

    #[test]
    fn it_is_the_equal_weight_mean() {
        let c = closes();
        let want = (zscore(&c, 10) + zscore(&c, 20) + zscore(&c, 40)) / 3.0;
        assert!((mean_zscore(&c, &[10, 20, 40]) - want).abs() < 1e-15);
        let want = (simple_return(&c, 1) + simple_return(&c, 5) + simple_return(&c, 20)) / 3.0;
        assert!((mean_return(&c, &[1, 5, 20]) - want).abs() < 1e-15);
    }

    #[test]
    fn the_order_of_the_lookbacks_does_not_matter() {
        let c = closes();
        let a = mean_zscore(&c, &[10, 20, 40]);
        let b = mean_zscore(&c, &[40, 10, 20]);
        assert!((a - b).abs() < 1e-12);
    }

    #[test]
    fn a_term_that_cannot_be_computed_makes_the_whole_thing_nan() {
        let c = closes();
        // A flat window has no sigma.
        let flat = vec![100.0; 80];
        assert!(mean_zscore(&flat, &[10, 20, 40]).is_nan());
        // A window longer than the data.
        assert!(mean_zscore(&c, &[10, 20, 200]).is_nan());
        assert!(mean_return(&c, &[1, 5, 500]).is_nan());
        assert!(mean_zscore(&c, &[]).is_nan());
    }
}
