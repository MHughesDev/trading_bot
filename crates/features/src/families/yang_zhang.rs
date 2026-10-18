//! Yang-Zhang volatility (S-5.14).
//!
//! Overnight variance, plus `k` times open-to-close variance, plus `(1 − k)`
//! times Rogers-Satchell, with the `k` that minimises the estimator's variance.
//! For an instrument that trades continuously the overnight term is the gap
//! between one bar's close and the next bar's open, which is near zero, and the
//! estimator collapses toward Rogers-Satchell — which is the correct behaviour
//! and not a degenerate one.
//!
//! It is the template's formula (`estimate_volatility.py::_range_estimators`),
//! written again here because the engine has no Python. The two are compared in
//! `mcp-server`'s `features_f2_reconciliation` over the same bars; this module's
//! tests pin the properties that make that comparison meaningful.
//!
//! Reported as a **per-bar standard deviation**, like `parkinson_vol_N` and its
//! siblings: the family is named `…_vol` and returns `sqrt(variance)`. The
//! template reports the variance under `variance_per_bar`, so the comparison is
//! `engine² == template`.

/// `ln(a / b)`, or `NaN` when either price is not positive.
fn ln_ratio(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        f64::NAN
    } else {
        (a / b).ln()
    }
}

/// Sample variance (ddof = 1). Zero for fewer than two values, as the template
/// does: a single overnight gap carries no variance, and the estimator then
/// rests on the other two terms.
#[allow(clippy::cast_precision_loss)]
fn sample_variance(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = v.iter().sum::<f64>() / v.len() as f64;
    v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64
}

/// The estimator's weight on open-to-close variance.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn k(n: usize) -> f64 {
    let n = n as f64;
    0.34 / (1.34 + (n + 1.0) / (n - 1.0))
}

/// Yang-Zhang volatility over the window, as a per-bar standard deviation.
///
/// Every slice holds the window **newest first** and all four have the same
/// length `n ≥ 2`. `NaN` for a window that is not usable: a non-positive price,
/// a bar whose high is below its low, or fewer than two bars. Never zero in
/// their place — a missing value that read as "no volatility" would size a
/// stop at nothing.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn yang_zhang_vol(open: &[f64], high: &[f64], low: &[f64], close: &[f64]) -> f64 {
    let n = close.len();
    if n < 2 || open.len() != n || high.len() != n || low.len() != n {
        return f64::NAN;
    }
    for b in 0..n {
        if open[b] <= 0.0 || high[b] <= 0.0 || low[b] <= 0.0 || close[b] <= 0.0 || high[b] < low[b]
        {
            return f64::NAN;
        }
    }

    // Rogers-Satchell over every bar: ho·(ho − co) + lo·(lo − co).
    let mut rs = 0.0;
    let mut open_to_close = Vec::with_capacity(n);
    for b in 0..n {
        let ho = ln_ratio(high[b], open[b]);
        let lo = ln_ratio(low[b], open[b]);
        let co = ln_ratio(close[b], open[b]);
        rs += ho * (ho - co) + lo * (lo - co);
        open_to_close.push(co);
    }
    let rs = rs / n as f64;

    // The overnight term pairs each bar's open with the *previous* bar's close.
    // Newest first, the previous bar of index `b` is `b + 1`.
    let overnight: Vec<f64> = (0..n - 1)
        .map(|b| ln_ratio(open[b], close[b + 1]))
        .collect();

    let kk = k(n);
    let yz = sample_variance(&overnight) + kk * sample_variance(&open_to_close) + (1.0 - kk) * rs;
    yz.max(0.0).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic OHLC series, newest first, with real ranges and gaps.
    fn bars(n: usize, gap: f64) -> (Vec<f64>, Vec<f64>, Vec<f64>, Vec<f64>) {
        let mut o = Vec::new();
        let mut h = Vec::new();
        let mut l = Vec::new();
        let mut c = Vec::new();
        let mut price = 100.0_f64;
        let mut state = 12_345_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for _ in 0..n {
            let open = price * (1.0 + gap * next());
            let close = open * (1.0 + 0.02 * next());
            let high = open.max(close) * (1.0 + 0.01 * next().abs());
            let low = open.min(close) * (1.0 - 0.01 * next().abs());
            o.push(open);
            h.push(high);
            l.push(low);
            c.push(close);
            price = close;
        }
        // The loop built oldest first; the engine's slices are newest first.
        for v in [&mut o, &mut h, &mut l, &mut c] {
            v.reverse();
        }
        (o, h, l, c)
    }

    /// The variance the template (`estimate_volatility._range_estimators`) gives
    /// for this series, computed once and pinned, so the formula is protected on
    /// a machine with no Python. The series is deterministic arithmetic, so it is
    /// the same in both languages to a few ulp; the live comparison is
    /// `mcp-server`'s `features_f2_reconciliation`.
    #[test]
    fn it_matches_the_template_on_a_pinned_series() {
        let t = |i: usize| i as f64;
        let n = 200;
        let (mut o, mut h, mut l, mut c) = (vec![], vec![], vec![], vec![]);
        for i in 0..n {
            let open = 100.0 * (1.0 + 0.01 * (0.3 * t(i)).sin());
            let close = open * (1.0 + 0.005 * (0.7 * t(i)).cos());
            h.push(open.max(close) * (1.0 + 0.002 * (1.0 + (1.1 * t(i)).sin())));
            l.push(open.min(close) * (1.0 - 0.002 * (1.0 + (0.9 * t(i)).cos())));
            o.push(open);
            c.push(close);
        }
        for v in [&mut o, &mut h, &mut l, &mut c] {
            v.reverse();
        }
        let sd = yang_zhang_vol(&o, &h, &l, &c);
        let want = 4.016_469_204_811_634e-5;
        assert!((sd * sd - want).abs() / want < 1e-9, "{} vs {want}", sd * sd);
    }

    #[test]
    fn a_flat_series_has_no_volatility() {
        let v = vec![100.0; 30];
        assert_eq!(yang_zhang_vol(&v, &v, &v, &v), 0.0);
    }

    #[test]
    fn k_is_the_published_weight() {
        // Yang & Zhang (2000): k = 0.34 / (1.34 + (n + 1) / (n − 1)).
        assert!((k(20) - 0.34 / (1.34 + 21.0 / 19.0)).abs() < 1e-15);
        assert!(k(2) < k(200), "more bars lean harder on open-to-close");
    }

    /// With no gap between one close and the next open the overnight variance is
    /// exactly zero and the estimate is `k·σ²(open→close) + (1−k)·RS`. Computed
    /// here by an independent loop, so the test would catch a wrong pairing.
    #[test]
    fn without_gaps_it_is_the_blend_of_open_to_close_and_rogers_satchell() {
        let (mut o, h, l, c) = bars(60, 0.0);
        // Force open[b] == close[b + 1] exactly (newest first).
        let n_pairs = o.len() - 1;
        o[..n_pairs].copy_from_slice(&c[1..=n_pairs]);
        let got = yang_zhang_vol(&o, &h, &l, &c);

        let n = c.len();
        let co: Vec<f64> = (0..n).map(|b| (c[b] / o[b]).ln()).collect();
        let m = co.iter().sum::<f64>() / n as f64;
        let var_c = co.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1) as f64;
        let rs: f64 = (0..n)
            .map(|b| {
                let ho = (h[b] / o[b]).ln();
                let lo = (l[b] / o[b]).ln();
                ho * (ho - co[b]) + lo * (lo - co[b])
            })
            .sum::<f64>()
            / n as f64;
        let kk = 0.34 / (1.34 + (n as f64 + 1.0) / (n as f64 - 1.0));
        let want = (kk * var_c + (1.0 - kk) * rs).max(0.0).sqrt();
        // The overnight term was built to be zero only up to rounding.
        assert!((got - want).abs() < 1e-9, "{got} vs {want}");
    }

    /// Overnight gaps are what Yang-Zhang adds over Rogers-Satchell, so a series
    /// with large gaps must read as more volatile than the same bars without.
    #[test]
    fn gaps_raise_it() {
        let (o0, h0, l0, c0) = bars(120, 0.0);
        let (o1, h1, l1, c1) = bars(120, 0.08);
        let calm = yang_zhang_vol(&o0, &h0, &l0, &c0);
        let gappy = yang_zhang_vol(&o1, &h1, &l1, &c1);
        assert!(gappy > calm, "{gappy} should exceed {calm}");
    }

    #[test]
    fn a_malformed_window_is_nan_and_never_zero() {
        let (o, mut h, l, c) = bars(30, 0.0);
        assert!(yang_zhang_vol(&o[..1], &h[..1], &l[..1], &c[..1]).is_nan());
        h[3] = l[3] * 0.5; // high below low
        assert!(yang_zhang_vol(&o, &h, &l, &c).is_nan());
        let (mut o, h, l, c) = bars(30, 0.0);
        o[2] = 0.0;
        assert!(yang_zhang_vol(&o, &h, &l, &c).is_nan());
        assert!(yang_zhang_vol(&o, &h, &l, &c[..29]).is_nan(), "ragged slices");
    }

    /// The overnight pairing is open of the newer bar against close of the older
    /// one. Reversing it (the easy mistake with newest-first slices) pairs each
    /// open with the *following* close and gives a different number.
    #[test]
    fn the_overnight_term_pairs_each_open_with_the_previous_close() {
        let (o, h, l, c) = bars(40, 0.05);
        let right = yang_zhang_vol(&o, &h, &l, &c);
        // Same bars, oldest first: the previous-close pairing is now wrong on
        // purpose, so the answer must move.
        let rev = |v: &[f64]| v.iter().rev().copied().collect::<Vec<_>>();
        let wrong = yang_zhang_vol(&rev(&o), &rev(&h), &rev(&l), &rev(&c));
        assert!((right - wrong).abs() > 1e-9, "{right} == {wrong}");
    }
}
