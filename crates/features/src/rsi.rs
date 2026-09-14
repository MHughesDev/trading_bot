//! RSI(n), Wilder smoothing — the one implementation (AT-15).
//!
//! Windowed for the same reason as the EMA: Wilder's average is recursive, so it is
//! seeded at the oldest bar of a `RSI_LOOKBACK_PERIODS × n + 1` bar window — a simple
//! average over the first `n` changes, then Wilder steps to the decision bar.

use dataplane::feature::{Feature, FeatureDef, FeatureError, WindowedFrame};

/// Algorithm version — increment when the computation logic changes.
pub const RSI_FEATURE_VERSION: u32 = 2;

/// Bars of window per period (plus one for the first change).
pub const RSI_LOOKBACK_PERIODS: usize = 5;

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

    #[allow(clippy::cast_precision_loss)]
    fn compute(&self, frame: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        let n = self.period as f64;
        let oldest = frame.lookback() as usize - 1;
        let (mut avg_gain, mut avg_loss) = (0.0, 0.0);
        let mut prev = frame.close(oldest)?;
        for (i, back) in (0..oldest).rev().enumerate() {
            let c = frame.close(back)?;
            let change = c - prev;
            prev = c;
            let (gain, loss) = (change.max(0.0), (-change).max(0.0));
            if i < self.period {
                avg_gain += gain / n;
                avg_loss += loss / n;
            } else {
                avg_gain = (avg_gain * (n - 1.0) + gain) / n;
                avg_loss = (avg_loss * (n - 1.0) + loss) / n;
            }
        }
        if avg_loss == 0.0 {
            return Ok(100.0);
        }
        Ok(100.0 - 100.0 / (1.0 + avg_gain / avg_loss))
    }
}
