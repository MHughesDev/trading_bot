/// Algorithm version — increment when the computation logic changes.
pub const EMA_FEATURE_VERSION: u32 = 1;

/// Incremental EMA(n) using the standard smoothing multiplier k = 2 / (n + 1).
///
/// Seeded on the very first sample (no warm-up); every subsequent call returns
/// an updated value.
#[derive(Clone, Debug)]
pub struct Ema {
    period: usize,
    value: Option<f64>,
}

impl Ema {
    pub fn new(period: usize) -> Self {
        assert!(period >= 1, "EMA period must be at least 1");
        Self {
            period,
            value: None,
        }
    }

    /// Update with a new sample value; returns the current EMA.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&mut self, value: f64) -> f64 {
        let k = 2.0 / (self.period as f64 + 1.0);
        let ema = match self.value {
            None => value,
            Some(prev) => value * k + prev * (1.0 - k),
        };
        self.value = Some(ema);
        ema
    }

    pub fn value(&self) -> Option<f64> {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_update_seeds_to_price() {
        let mut ema = Ema::new(5);
        assert_eq!(ema.update(100.0), 100.0);
    }

    #[test]
    fn convergence_toward_new_price() {
        let mut ema = Ema::new(3);
        // Seed at 100
        ema.update(100.0);
        // EMA should move toward 200
        let v = ema.update(200.0);
        assert!(v > 100.0 && v < 200.0);
    }

    #[test]
    fn deterministic_across_runs() {
        let prices = [10.0, 11.0, 10.5, 11.2, 10.8];
        let run = || {
            let mut e = Ema::new(3);
            prices.iter().map(|&p| e.update(p)).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }
}
