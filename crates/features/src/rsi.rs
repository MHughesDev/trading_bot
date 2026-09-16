//! RSI(n), Wilder smoothing — the one implementation (AT-15).
//!
//! Windowed for the same reason as the EMA: Wilder's average is recursive, so it is
//! seeded at the oldest bar of a `RSI_LOOKBACK_PERIODS × n + 1` bar window — a simple
//! average over the first `n` changes, then Wilder steps to the decision bar.
//!
//! [`WilderAverage`] is the kernel. `atr_N` is Wilder's average of the true range
//! and uses it unchanged; writing the recurrence a second time there would be the
//! INV-14 violation that AT-15 exists to catch, even though the *feature* differs.

use dataplane::feature::{Feature, FeatureDef, FeatureError, WindowedFrame};

/// Algorithm version — increment when the computation logic changes.
pub const RSI_FEATURE_VERSION: u32 = 2;

/// Bars of window per period (plus one for the first change).
pub const RSI_LOOKBACK_PERIODS: usize = 5;

/// Wilder's running average: a simple mean of the first `period` samples, then
/// `avg = (avg * (n - 1) + x) / n` for every sample after.
///
/// Fed one sample at a time, oldest first, so the caller decides what a sample is
/// — a gain, a loss, or a true range.
#[derive(Clone, Copy, Debug)]
pub struct WilderAverage {
    period: f64,
    seen: usize,
    n: usize,
    avg: f64,
}

impl WilderAverage {
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn new(period: usize) -> Self {
        Self { period: period as f64, seen: 0, n: period, avg: 0.0 }
    }

    /// Push one sample, oldest first.
    pub fn push(&mut self, x: f64) {
        if self.seen < self.n {
            self.avg += x / self.period;
        } else {
            self.avg = (self.avg * (self.period - 1.0) + x) / self.period;
        }
        self.seen += 1;
    }

    /// The average, or `NaN` before `period` samples have been seen.
    #[must_use]
    pub fn value(&self) -> f64 {
        if self.seen < self.n {
            f64::NAN
        } else {
            self.avg
        }
    }
}

pub struct Rsi {
    def: FeatureDef,
    period: usize,
}

impl Rsi {
    /// # Panics
    /// `period` must be at least 2; names are validated before construction.
    #[must_use]
    pub fn new(period: usize) -> Self {
        assert!(period >= 2, "RSI period must be at least 2");
        let lookback = u32::try_from(period * RSI_LOOKBACK_PERIODS + 1).unwrap_or(u32::MAX);
        Self {
            def: crate::runtime::definition(&format!("rsi_{period}"), "rsi", RSI_FEATURE_VERSION, lookback, include_str!("rsi.rs")),
            period,
        }
    }
}

impl Feature for Rsi {
    fn def(&self) -> &FeatureDef {
        &self.def
    }

    fn compute(&self, frame: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        let oldest = frame.lookback() as usize - 1;
        let (mut gains, mut losses) = (WilderAverage::new(self.period), WilderAverage::new(self.period));
        let mut prev = frame.close(oldest)?;
        for back in (0..oldest).rev() {
            let c = frame.close(back)?;
            let change = c - prev;
            prev = c;
            gains.push(change.max(0.0));
            losses.push((-change).max(0.0));
        }
        let (avg_gain, avg_loss) = (gains.value(), losses.value());
        if avg_loss == 0.0 {
            return Ok(100.0);
        }
        Ok(100.0 - 100.0 / (1.0 + avg_gain / avg_loss))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Extracting the kernel must not have moved a bit: it is the inline loop the
    /// feature used to run, sample for sample.
    #[test]
    fn the_kernel_is_the_recurrence_it_replaced() {
        let samples: Vec<f64> = (0..30).map(|i| ((i * 7) % 11) as f64 * 0.25).collect();
        for period in [2usize, 5, 14] {
            #[allow(clippy::cast_precision_loss)]
            let n = period as f64;
            let mut avg = 0.0;
            for (i, &x) in samples.iter().enumerate() {
                if i < period {
                    avg += x / n;
                } else {
                    avg = (avg * (n - 1.0) + x) / n;
                }
            }
            let mut w = WilderAverage::new(period);
            for &x in &samples {
                w.push(x);
            }
            assert_eq!(w.value().to_bits(), avg.to_bits(), "period {period}");
        }
    }

    /// Before `period` samples the average is not yet defined. Returning the
    /// partial sum instead would be a value computed from a window that does not
    /// exist, which is the one thing INV-13 forbids.
    #[test]
    fn a_partial_window_is_nan_rather_than_a_partial_average() {
        let mut w = WilderAverage::new(4);
        w.push(1.0);
        w.push(1.0);
        assert!(w.value().is_nan());
        w.push(1.0);
        w.push(1.0);
        assert!((w.value() - 1.0).abs() < f64::EPSILON);
    }
}
