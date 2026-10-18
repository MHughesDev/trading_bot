//! Coverage change-point detection for the quality monitor (Set N N-4.1, MD-11).
//!
//! The adaptive conformal wrapper itself (the Gibbs-Candes update of the working miscoverage
//! level, and the reset of its score window when a shift is detected) lives in the trainer,
//! `apps/model-trainer/app/calibration.py`, beside the models it wraps. What the *platform*
//! owns is deciding, from the stream of forecasts it has already scored, that a model's
//! coverage has shifted: a two-sided CUSUM on the miss indicator.
//!
//! The same parameters are used in both places (`DEFAULT_K`, `DEFAULT_H`), so the monitor's
//! alert and the wrapper's reset fire on the same observation.
//!
//! # The statistic
//!
//! With nominal miscoverage `alpha` (0.10 for a 90% interval), `x_t = 1{y_t outside the
//! interval} - alpha` has mean zero when the interval is calibrated. The upper CUSUM
//! `g+_t = max(0, g+_{t-1} + x_t - k)` rises when the interval is too narrow (more misses than
//! nominal) and the lower `g-_t = max(0, g-_{t-1} - x_t - k)` when it is too wide. Either
//! crossing `h` is a change point; the detector then restarts from zero.
//!
//! This is a *detector*, not a verdict: it says the coverage process changed and where, and
//! the caller decides what to do (the monitor raises a `coverage_shift` alert, whose
//! consequences are the existing alert -> retrain/recalibrate path).

/// Reference value: half the smallest shift in miss rate worth detecting.
pub const DEFAULT_K: f64 = 0.05;
/// Decision threshold, in units of miss-rate mass.
pub const DEFAULT_H: f64 = 4.0;

/// Which way the coverage moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shift {
    /// More misses than nominal: the interval became too narrow.
    Undercover,
    /// Fewer misses than nominal: the interval became too wide.
    Overcover,
}

/// A detected change point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    /// Index (in the order observations were fed) at which the statistic crossed `h`.
    pub at: usize,
    pub shift: Shift,
}

/// A two-sided CUSUM on interval misses.
#[derive(Clone, Debug)]
pub struct CoverageCusum {
    alpha: f64,
    k: f64,
    h: f64,
    up: f64,
    down: f64,
    seen: usize,
}

impl CoverageCusum {
    /// # Panics
    /// If `alpha` is outside `(0, 1)` or `k`/`h` is not positive: a detector built on such
    /// parameters would never or always fire, and that is a programming error.
    #[must_use]
    pub fn new(alpha: f64, k: f64, h: f64) -> Self {
        assert!(alpha > 0.0 && alpha < 1.0, "alpha must be in (0, 1)");
        assert!(k > 0.0 && h > 0.0, "k and h must be positive");
        Self {
            alpha,
            k,
            h,
            up: 0.0,
            down: 0.0,
            seen: 0,
        }
    }

    /// The default detector for a 90% interval.
    #[must_use]
    pub fn for_90() -> Self {
        Self::new(0.10, DEFAULT_K, DEFAULT_H)
    }

    /// Feed one observation: `covered` is whether the realised value fell inside the interval.
    /// Returns the change point if this observation crossed the threshold, and restarts.
    pub fn step(&mut self, covered: bool) -> Option<Change> {
        let miss = if covered { 0.0 } else { 1.0 };
        let x = miss - self.alpha;
        self.up = (self.up + x - self.k).max(0.0);
        self.down = (self.down - x - self.k).max(0.0);
        let at = self.seen;
        self.seen += 1;
        let shift = if self.up > self.h {
            Some(Shift::Undercover)
        } else if self.down > self.h {
            Some(Shift::Overcover)
        } else {
            None
        };
        if shift.is_some() {
            self.up = 0.0;
            self.down = 0.0;
        }
        shift.map(|shift| Change { at, shift })
    }

    /// Run over a whole series and return every change point, in order.
    #[must_use]
    pub fn scan(alpha: f64, k: f64, h: f64, covered: &[bool]) -> Vec<Change> {
        let mut d = Self::new(alpha, k, h);
        covered.iter().filter_map(|&c| d.step(c)).collect()
    }
}

/// Rolling coverage over a window: the fraction covered among the last `window` observations
/// at each index (the first `window - 1` indices use what exists).
#[must_use]
pub fn rolling_coverage(covered: &[bool], window: usize) -> Vec<f64> {
    let window = window.max(1);
    let mut out = Vec::with_capacity(covered.len());
    let mut hits = 0usize;
    for (i, &c) in covered.iter().enumerate() {
        hits += usize::from(c);
        if i >= window {
            hits -= usize::from(covered[i - window]);
        }
        let n = (i + 1).min(window);
        out.push(hits as f64 / n as f64);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic miss pattern at a given rate: every `1/rate`-th observation misses.
    fn series(n: usize, miss_every: usize) -> Vec<bool> {
        (0..n).map(|i| (i + 1) % miss_every != 0).collect()
    }

    #[test]
    fn a_calibrated_series_does_not_fire() {
        // One miss in ten is exactly nominal for a 90% interval.
        let covered = series(2_000, 10);
        assert!(CoverageCusum::scan(0.10, DEFAULT_K, DEFAULT_H, &covered).is_empty());
    }

    #[test]
    fn a_shift_to_undercoverage_fires_within_the_stated_lag() {
        // 400 calibrated observations, then one miss in three from index 400.
        let mut covered = series(400, 10);
        covered.extend(series(400, 3));
        let changes = CoverageCusum::scan(0.10, DEFAULT_K, DEFAULT_H, &covered);
        let first = changes.first().expect("the shift is detected");
        assert_eq!(first.shift, Shift::Undercover);
        assert!(
            first.at >= 400,
            "no false alarm before the shift: {}",
            first.at
        );
        // Excess miss mass per observation is ~0.233 - 0.05 = 0.18, so h = 4 needs ~22.
        assert!(
            first.at - 400 <= 40,
            "detected {} observations late",
            first.at - 400
        );
    }

    #[test]
    fn a_shift_to_overcoverage_is_a_different_shift() {
        let mut covered = series(300, 10);
        covered.extend(vec![true; 300]); // never misses: far too wide
        let changes = CoverageCusum::scan(0.10, DEFAULT_K, DEFAULT_H, &covered);
        assert_eq!(changes.first().map(|c| c.shift), Some(Shift::Overcover));
    }

    #[test]
    fn the_detector_restarts_after_a_change() {
        // Sustained undercoverage fires repeatedly, not once forever.
        let covered = series(600, 3);
        let n = CoverageCusum::scan(0.10, DEFAULT_K, DEFAULT_H, &covered).len();
        assert!(n >= 3, "expected repeated detections, got {n}");
    }

    #[test]
    fn rolling_coverage_uses_only_the_window() {
        let covered = [true, true, false, false, true];
        let r = rolling_coverage(&covered, 2);
        assert_eq!(r, vec![1.0, 1.0, 0.5, 0.0, 0.5]);
    }
}
