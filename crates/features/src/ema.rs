//! EMA(n) — the one implementation (AT-15).
//!
//! Windowed, not recursive: an EMA carried forward from the first bar ever seen
//! depends on unbounded history, which no declared lookback can describe and no
//! windowed view can enforce (INV-13). This EMA is seeded at the oldest bar of a
//! window of `EMA_LOOKBACK_PERIODS × n` bars; the seed's residual weight is
//! (1 − 2/(n+1))^(5n) ≈ e^−10 ≈ 4.5·10⁻⁵.

use dataplane::feature::{Feature, FeatureDef, FeatureError, WindowedFrame};

/// Algorithm version — increment when the computation logic changes.
pub const EMA_FEATURE_VERSION: u32 = 2;

/// Bars of window per period.
pub const EMA_LOOKBACK_PERIODS: usize = 5;

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
            def: crate::runtime::definition(&format!("ema_{period}"), "ema", EMA_FEATURE_VERSION, lookback, include_str!("ema.rs")),
            period,
        }
    }
}

impl Feature for Ema {
    fn def(&self) -> &FeatureDef {
        &self.def
    }

    #[allow(clippy::cast_precision_loss)]
    fn compute(&self, frame: &WindowedFrame<'_>) -> Result<f64, FeatureError> {
        let k = 2.0 / (self.period as f64 + 1.0);
        let oldest = frame.lookback() as usize - 1;
        let mut v = frame.close(oldest)?;
        for back in (0..oldest).rev() {
            v = frame.close(back)? * k + v * (1.0 - k);
        }
        Ok(v)
    }
}
