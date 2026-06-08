/// Algorithm version — increment when the computation logic changes.
pub const RSI_FEATURE_VERSION: u32 = 1;

/// Incremental RSI(n) using Wilder's smoothing method.
///
/// Returns `None` until `period + 1` prices have been provided (the initial SMA
/// seed requires `period` changes; the first Wilder step requires one more price).
#[derive(Clone, Debug)]
pub struct Rsi {
    period: usize,
    prev_price: Option<f64>,
    avg_gain: f64,
    avg_loss: f64,
    seed_gains: f64,
    seed_losses: f64,
    seed_count: usize,
    initialized: bool,
}

impl Rsi {
    pub fn new(period: usize) -> Self {
        assert!(period >= 2, "RSI period must be at least 2");
        Self {
            period,
            prev_price: None,
            avg_gain: 0.0,
            avg_loss: 0.0,
            seed_gains: 0.0,
            seed_losses: 0.0,
            seed_count: 0,
            initialized: false,
        }
    }

    /// Update with a new close price.
    ///
    /// Returns RSI ∈ [0, 100] once enough data exists; `None` otherwise.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&mut self, price: f64) -> Option<f64> {
        let prev = self.prev_price.replace(price)?;
        let change = price - prev;
        let gain = change.max(0.0);
        let loss = (-change).max(0.0);

        if !self.initialized {
            self.seed_gains += gain;
            self.seed_losses += loss;
            self.seed_count += 1;

            if self.seed_count == self.period {
                self.avg_gain = self.seed_gains / self.period as f64;
                self.avg_loss = self.seed_losses / self.period as f64;
                self.initialized = true;
                return Some(Self::rsi(self.avg_gain, self.avg_loss));
            }
            return None;
        }

        // Wilder's smoothing: new_avg = (prev_avg * (n - 1) + current) / n
        let n = self.period as f64;
        self.avg_gain = (self.avg_gain * (n - 1.0) + gain) / n;
        self.avg_loss = (self.avg_loss * (n - 1.0) + loss) / n;
        Some(Self::rsi(self.avg_gain, self.avg_loss))
    }

    fn rsi(avg_gain: f64, avg_loss: f64) -> f64 {
        if avg_loss == 0.0 {
            return 100.0;
        }
        100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_before_period_plus_one_prices() {
        let mut rsi = Rsi::new(3);
        // Need prev_price + period changes = 4 prices total before first value
        assert!(rsi.update(100.0).is_none()); // sets prev_price only
        assert!(rsi.update(101.0).is_none()); // change 1
        assert!(rsi.update(102.0).is_none()); // change 2
        // 4th price completes the seed (period=3 changes)
        assert!(rsi.update(103.0).is_some());
    }

    #[test]
    fn rsi_in_range() {
        let mut rsi = Rsi::new(14);
        let prices: Vec<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
        for &p in &prices {
            if let Some(v) = rsi.update(p) {
                assert!((0.0..=100.0).contains(&v), "RSI out of range: {v}");
            }
        }
    }

    #[test]
    fn deterministic_across_runs() {
        let prices: Vec<f64> = (0..20).map(|i| 100.0 + (i as f64) * 0.5).collect();
        let run = || {
            let mut r = Rsi::new(14);
            prices.iter().filter_map(|&p| r.update(p)).collect::<Vec<_>>()
        };
        let a = run();
        let b = run();
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b.iter()) {
            assert!((x - y).abs() < 1e-12);
        }
    }
}
