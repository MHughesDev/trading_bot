//! EMA(n) — the one implementation (AT-15).
//!
//! Windowed, not recursive: an EMA carried forward from the first bar ever seen
//! depends on unbounded history, which no declared lookback can describe and no
//! windowed view can enforce (INV-13). This EMA is seeded at the oldest bar of a
//! window of `EMA_LOOKBACK_PERIODS × n` bars; the seed's residual weight is
//! (1 − 2/(n+1))^(5n) ≈ e^−10 ≈ 4.5·10⁻⁵.
//!
//! [`ema_try_fold`] is the kernel. Every exponential average in the crate —
//! `ema_N`, the MACD lines, the Keltner centre — calls it rather than writing the
//! recurrence again, because INV-14 is about one *implementation*, not one
//! *feature name*: two spellings of the recurrence would drift in the last bit
//! and nothing would say which produced a stored value.

use dataplane::feature::{Feature, FeatureDef, FeatureError, WindowedFrame};

/// Algorithm version — increment when the computation logic changes.
pub const EMA_FEATURE_VERSION: u32 = 2;

/// Bars of window per period.
pub const EMA_LOOKBACK_PERIODS: usize = 5;

/// The smoothing constant for `period`.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn smoothing(period: usize) -> f64 {
    2.0 / (period as f64 + 1.0)
}

/// The exponential recurrence over `n` values read **oldest first** through
/// `at`, seeded at `at(0)`. `NaN` for `n == 0`.
///
/// The one place the recurrence is written, and allocation-free so the batch
/// path can call it once per bar. The iteration order is part of the contract: a
/// different order is a different `f64`, and stored values are compared bit for
/// bit by the consistency harness.
///
/// # Errors
/// Whatever `at` returns.
pub fn ema_try_fold<E>(
    period: usize,
    n: usize,
    mut at: impl FnMut(usize) -> Result<f64, E>,
) -> Result<f64, E> {
    if n == 0 {
        return Ok(f64::NAN);
    }
    let k = smoothing(period);
    let mut v = at(0)?;
    for i in 1..n {
        v = at(i)? * k + v * (1.0 - k);
    }
    Ok(v)
}

/// [`ema_try_fold`] over a slice given oldest first.
#[must_use]
pub fn ema_from_oldest(values: &[f64], period: usize) -> f64 {
    match ema_try_fold::<std::convert::Infallible>(period, values.len(), |i| Ok(values[i])) {
        Ok(v) => v,
    }
}

pub struct Ema {
    def: FeatureDef,
    period: usize,
}

impl Ema {
    /// # Panics
    /// `period` must be at least 1; names are validated before construction.
    #[must_use]
    pub fn new(period: usize) -> Self {
        assert!(period >= 1, "EMA period must be at least 1");
        let lookback = u32::try_from(period * EMA_LOOKBACK_PERIODS).unwrap_or(u32::MAX);
        Self {
            def: crate::runtime::definition(
                &format!("ema_{period}"),
                "ema",
                EMA_FEATURE_VERSION,
                lookback,
                include_str!("ema.rs"),
            ),
            period,
        }
    }
}

impl Feature for Ema {
    fn def(&self) -> &FeatureDef {
        &self.def
    }

    fn compute(&self, frame: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        let n = frame.lookback() as usize;
        ema_try_fold(self.period, n, |i| frame.close(n - 1 - i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The refactor that made the recurrence shareable must not have moved a
    /// single bit: the kernel is the loop the feature used to run inline.
    #[test]
    fn the_kernel_is_the_recurrence_it_replaced() {
        let values: Vec<f64> = (0..40)
            .map(|i| (f64::from(i) * 0.31).sin() * 3.0 + 100.0)
            .collect();
        for period in [1usize, 3, 7, 21] {
            #[allow(clippy::cast_precision_loss)]
            let k = 2.0 / (period as f64 + 1.0);
            let mut v = values[0];
            for &x in &values[1..] {
                v = x * k + v * (1.0 - k);
            }
            assert_eq!(
                ema_from_oldest(&values, period).to_bits(),
                v.to_bits(),
                "period {period}"
            );
        }
    }

    #[test]
    fn an_empty_window_is_nan_rather_than_zero() {
        assert!(ema_from_oldest(&[], 7).is_nan());
    }
}
