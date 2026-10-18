//! Fractional differencing of the log close (S-5.14; López de Prado, AFML ch. 5).
//!
//! An ordinary difference (`d = 1`) makes a price stationary by throwing away
//! its memory; a fractional one (`0 < d < 1`) makes it nearly stationary while
//! keeping most of the long-run level information a model wants. The weights
//! are the binomial series of `(1 − B)^d`:
//!
//! ```text
//! w₀ = 1,   wₖ = −wₖ₋₁ · (d − k + 1) / k
//! ```
//!
//! and the value at the decision bar is `Σₖ wₖ · ln(closeₜ₋ₖ)` over a fixed
//! window of `n` bars. A fixed window, not the expanding one: the engine's
//! contract is that a feature is a function of exactly its declared lookback
//! (INV-13), and the expanding form would read further back the longer the
//! series ran.
//!
//! `d = 0` returns the log close itself; `d = 1` returns the one-bar log return
//! (the weights are `[1, −1, 0, 0, …]`). Both are tested, because they are the
//! two ends the middle has to join.

/// The weights for `n` terms. `w[0] = 1`.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn weights(d: f64, n: usize) -> Vec<f64> {
    let mut w = Vec::with_capacity(n);
    if n == 0 {
        return w;
    }
    w.push(1.0);
    for k in 1..n {
        let prev = w[k - 1];
        w.push(-prev * (d - k as f64 + 1.0) / k as f64);
    }
    w
}

/// Fractional difference of order `d` of the log close, over the window.
///
/// `closes` is **newest first** (`closes[0]` is the decision bar), and its
/// length is the window `n`. `NaN` when any close in the window is not
/// positive, or the window is empty.
#[must_use]
pub fn fracdiff_log_close(closes: &[f64], d: f64) -> f64 {
    if closes.is_empty() || closes.iter().any(|c| *c <= 0.0 || !c.is_finite()) {
        return f64::NAN;
    }
    weights(d, closes.len())
        .iter()
        .zip(closes)
        .map(|(w, c)| w * c.ln())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_weights_are_the_binomial_series() {
        let w = weights(0.4, 5);
        assert_eq!(w[0], 1.0);
        assert!((w[1] - -0.4).abs() < 1e-15, "w1 = -d");
        assert!((w[2] - 0.4 * (0.4 - 1.0) / 2.0).abs() < 1e-15, "w2 = d(d-1)/2");
        // For 0 < d < 1 every weight after the first is negative: the memory
        // decays without changing sign.
        assert!(w[1..].iter().all(|x| *x < 0.0), "{w:?}");
        // …and they shrink toward zero.
        assert!(w[4].abs() < w[3].abs());
    }

    #[test]
    fn d_zero_is_the_log_close_and_d_one_is_the_log_return() {
        let closes = [105.0, 100.0, 98.0, 97.0];
        assert!((fracdiff_log_close(&closes, 0.0) - closes[0].ln()).abs() < 1e-15);
        let one = fracdiff_log_close(&closes, 1.0);
        assert!((one - (closes[0] / closes[1]).ln()).abs() < 1e-12, "{one}");
    }

    /// The point of the transform: a trending price is not stationary, its
    /// fractional difference nearly is, and it still correlates with the level.
    /// Checked on a deterministic random walk with drift: the variance of the
    /// transformed series is far below the variance of the log price.
    #[test]
    fn it_tames_a_trending_price_without_flattening_it() {
        let mut s = 99_u64;
        let mut price = 100.0_f64.ln();
        let log_prices: Vec<f64> = (0..2000)
            .map(|_| {
                s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                price += 0.001 + 0.01 * ((s >> 11) as f64 / (1u64 << 53) as f64 - 0.5);
                price
            })
            .collect();
        let n = 64;
        let series: Vec<f64> = (n..log_prices.len())
            .map(|t| {
                let window: Vec<f64> = (0..n).map(|k| log_prices[t - k].exp()).collect();
                fracdiff_log_close(&window, 0.4)
            })
            .collect();
        let level = &log_prices[n..];
        let var = |v: &[f64]| {
            let m = v.iter().sum::<f64>() / v.len() as f64;
            v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64
        };
        assert!(var(&series) < 0.2 * var(level), "{} vs {}", var(&series), var(level));
        // Not flat: it still moves with the level (positive correlation).
        let (ml, ms) = (
            level.iter().sum::<f64>() / level.len() as f64,
            series.iter().sum::<f64>() / series.len() as f64,
        );
        let cov: f64 = level.iter().zip(&series).map(|(a, b)| (a - ml) * (b - ms)).sum();
        assert!(cov > 0.0, "it kept no memory of the level");
    }

    #[test]
    fn a_bad_close_is_nan() {
        assert!(fracdiff_log_close(&[], 0.4).is_nan());
        assert!(fracdiff_log_close(&[100.0, 0.0, 99.0], 0.4).is_nan());
        assert!(fracdiff_log_close(&[100.0, -1.0], 0.4).is_nan());
    }
}
