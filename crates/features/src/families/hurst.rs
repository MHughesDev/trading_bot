//! The Hurst exponent by rescaled range (S-5.14).
//!
//! Rescaled range over dyadic block sizes, then the slope of `ln(R/S)` on
//! `ln(block size)`. R/S rather than DFA because it needs no detrending choice,
//! and a detrending choice is a knob an agent could turn until it liked the
//! answer — the template's reason (`detect_regime.py::_hurst`), kept.
//!
//! Above about 0.55 the increments persist (trending); below about 0.45 they
//! reverse (mean-reverting); around 0.5 they are a random walk.
//!
//! # It takes increments, not prices
//!
//! The input is the bar **returns**. The cumulative deviation is formed inside
//! the estimator, so handing it the price path integrates twice and returns a
//! value near 1.0 for every series ever measured — which reads as "strongly
//! trending" for all of them. The caller passes log returns, oldest first.

/// The fewest returns the estimator will speak about, and the template's floor.
pub const MIN_RETURNS: usize = 128;

/// The smallest block size, in returns.
const MIN_BLOCK: usize = 16;

/// Hurst exponent of `returns` (oldest first), or `NaN` when the series is too
/// short or too degenerate to say.
///
/// `NaN` and not a default of 0.5: a missing value that read as "random walk"
/// would pass every regime gate in the direction of doing nothing, silently.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn hurst_rs(returns: &[f64]) -> f64 {
    let n = returns.len();
    if n < MIN_RETURNS || returns.iter().any(|x| !x.is_finite()) {
        return f64::NAN;
    }
    // A series that never moved has no range to rescale. Without this guard the
    // block mean of identical values is not always bit-identical to them, the
    // "deviations" are rounding residue, and the slope of the residue is a number
    // that looks like an exponent. (The template has the same hole on the same
    // input; it differs from this only there.)
    if returns.iter().all(|x| *x == returns[0]) {
        return f64::NAN;
    }

    let mut sizes: Vec<f64> = Vec::new();
    let mut values: Vec<f64> = Vec::new();
    let mut size = MIN_BLOCK;
    while size <= n / 4 {
        let blocks = n / size;
        let mut rescaled: Vec<f64> = Vec::new();
        for b in 0..blocks {
            let chunk = &returns[b * size..(b + 1) * size];
            let mean = chunk.iter().sum::<f64>() / size as f64;
            // Cumulative deviation from the block mean; the range is taken over
            // the partial sums as numpy's `cumsum` yields them (no leading zero).
            let mut running = 0.0;
            let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
            for x in chunk {
                running += x - mean;
                lo = lo.min(running);
                hi = hi.max(running);
            }
            let spread = hi - lo;
            let var =
                chunk.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (size - 1) as f64;
            let s = var.sqrt();
            if s > 0.0 && spread > 0.0 {
                rescaled.push(spread / s);
            }
        }
        if !rescaled.is_empty() {
            sizes.push((size as f64).ln());
            values.push((rescaled.iter().sum::<f64>() / rescaled.len() as f64).ln());
        }
        size *= 2;
    }
    if sizes.len() < 3 {
        return f64::NAN;
    }
    slope(&sizes, &values)
}

/// Least-squares slope of `y` on `x`.
#[allow(clippy::cast_precision_loss)]
fn slope(x: &[f64], y: &[f64]) -> f64 {
    let m = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / m, y.iter().sum::<f64>() / m);
    let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let sxx: f64 = x.iter().map(|a| (a - mx) * (a - mx)).sum();
    sxy / sxx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(seed: u64) -> impl FnMut() -> f64 {
        let mut s = seed;
        move || {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            // 12 uniforms ≈ a normal.
            let mut acc = 0.0;
            let mut t = s;
            for _ in 0..12 {
                t = t
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                acc += (t >> 11) as f64 / (1u64 << 53) as f64;
            }
            acc - 6.0
        }
    }

    #[test]
    fn a_short_series_is_nan_not_a_default() {
        assert!(hurst_rs(&vec![0.01; MIN_RETURNS - 1]).is_nan());
    }

    #[test]
    fn a_constant_series_is_nan() {
        // Zero spread everywhere: every block is skipped, so no sizes remain.
        assert!(hurst_rs(&vec![0.01; 512]).is_nan());
    }

    #[test]
    fn a_non_finite_return_is_nan() {
        let mut r: Vec<f64> = (0..300).map(|i| f64::from(i % 7) * 0.001 - 0.003).collect();
        r[10] = f64::NAN;
        assert!(hurst_rs(&r).is_nan());
    }

    /// Independent noise is a random walk in price: H near one half. R/S is
    /// biased upward on short samples, so the band is wide and says so.
    #[test]
    fn independent_increments_read_as_a_random_walk() {
        let mut g = lcg(7);
        let r: Vec<f64> = (0..4096).map(|_| 0.01 * g()).collect();
        let h = hurst_rs(&r);
        assert!((0.4..0.65).contains(&h), "{h}");
    }

    #[test]
    fn persistent_increments_read_as_trending() {
        // A slow-moving common component under noise: increments of one sign for
        // long stretches.
        let mut g = lcg(11);
        let mut drift = 0.0;
        let r: Vec<f64> = (0..4096)
            .map(|_| {
                drift = 0.97 * drift + 0.01 * g();
                drift + 0.002 * g()
            })
            .collect();
        let h = hurst_rs(&r);
        assert!(h > 0.7, "{h}");
    }

    #[test]
    fn reversing_increments_read_as_mean_reverting() {
        let mut g = lcg(13);
        let mut last = 0.0;
        let r: Vec<f64> = (0..4096)
            .map(|_| {
                let x = 0.01 * g() - 0.9 * last;
                last = x;
                x
            })
            .collect();
        let h = hurst_rs(&r);
        assert!(h < 0.4, "{h}");
    }

    /// The exact value the template (`detect_regime._hurst`) gives for this
    /// series, computed once and pinned. The qualitative tests above cannot see a
    /// change of divisor (`n − 1` against `n` in the block deviation moves the
    /// answer by a few percent, inside every band); this one can, and the
    /// cross-language reconciliation in `mcp-server` is the live version of it.
    /// The series is deterministic arithmetic, so it is the same in both languages
    /// to a few ulp.
    #[test]
    fn it_matches_the_template_on_a_pinned_series() {
        let r: Vec<f64> = (0..1024)
            .map(|i| {
                let i = f64::from(i);
                (i * 0.37).sin() * 0.01 + (i * 1.31).cos() * 0.004 + (i * 0.011).sin() * 0.003
            })
            .collect();
        let want = 0.466_537_127_684_570_1;
        let got = hurst_rs(&r);
        assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    }

    /// The slope is a regression, so a series scaled by a constant has the same
    /// exponent: R/S is scale-free.
    #[test]
    fn it_is_scale_free() {
        let mut g = lcg(5);
        let r: Vec<f64> = (0..1024).map(|_| g()).collect();
        let scaled: Vec<f64> = r.iter().map(|x| x * 1000.0).collect();
        assert!((hurst_rs(&r) - hurst_rs(&scaled)).abs() < 1e-9);
    }
}
