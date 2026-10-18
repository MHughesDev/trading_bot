//! Baselines: the EWMA band the execution and behaviour families are judged
//! against, and the CUSUM that watches the return series (COMP-007 §3).

/// Exponentially weighted mean and variance.
///
/// Exponential rather than a fixed window because the thing being estimated is
/// "what does this strategy normally do *now*", and a strategy's normal changes:
/// venue fee tiers move, liquidity shifts, the book grows. A 30-day flat window
/// gives a month-old regime the same vote as this morning.
#[derive(Clone, Copy, Debug)]
pub struct Ewma {
    alpha: f64,
    mean: f64,
    var: f64,
    count: u64,
}

impl Ewma {
    /// `alpha` is the weight on the newest observation, in `(0, 1]`.
    #[must_use]
    pub fn new(alpha: f64) -> Self {
        Self {
            alpha: alpha.clamp(f64::MIN_POSITIVE, 1.0),
            mean: 0.0,
            var: 0.0,
            count: 0,
        }
    }

    /// A baseline seeded from a completed paper or backtest run, which is where
    /// COMP-007 §3 says the default limits come from.
    #[must_use]
    pub fn seeded(alpha: f64, mean: f64, sigma: f64) -> Self {
        Self {
            alpha: alpha.clamp(f64::MIN_POSITIVE, 1.0),
            mean,
            var: sigma * sigma,
            // Two, not one. `z` refuses a band when `count < 2`, and rightly:
            // a sigma inferred from a single observation is zero. But a seeded
            // band was not inferred from anything — it was supplied, which is
            // the whole point of seeding. At `count: 1` every seeded baseline
            // returned `None` from `z`, so a monitor started from a known prior
            // measured nothing until it had collected its own history, which is
            // exactly the window seeding exists to cover.
            count: 2,
        }
    }

    pub fn update(&mut self, x: f64) {
        if self.count == 0 {
            self.mean = x;
            self.var = 0.0;
            self.count = 1;
            return;
        }
        let diff = x - self.mean;
        let incr = self.alpha * diff;
        self.mean += incr;
        self.var = (1.0 - self.alpha) * (self.var + diff * incr);
        self.count = self.count.saturating_add(1);
    }

    #[must_use]
    pub fn mean(&self) -> f64 {
        self.mean
    }

    #[must_use]
    pub fn sigma(&self) -> f64 {
        self.var.max(0.0).sqrt()
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.count
    }

    /// How many standard deviations `x` sits from the mean.
    ///
    /// `None` when there is no spread to measure — a single observation, or a
    /// perfectly constant series. Returning `0.0` there would read as "right on
    /// the baseline", and an unmeasured band would quietly clear everything; the
    /// caller is made to decide what to do about not knowing.
    /// Whether a spread exists to be measured against.
    ///
    /// The question is about the *spread*, not the sample count. A baseline seeded
    /// from a completed paper run has one observation here and a perfectly good
    /// sigma; a live series that has taken fifty identical readings has fifty and
    /// none. Gating on the count got the seeded case wrong — which is the case
    /// COMP-007 §3 says every default limit comes from.
    #[must_use]
    pub fn z(&self, x: f64) -> Option<f64> {
        let s = self.sigma();
        if !s.is_finite() || s <= f64::EPSILON {
            return None;
        }
        Some((x - self.mean) / s)
    }
}

/// Which way a CUSUM has drifted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CusumVerdict {
    /// Within the decision interval.
    Inside,
    /// The series has shifted **up** — the strategy is doing better than its
    /// baseline. Recorded as a warning, not a breach: halting a strategy for
    /// making more money than expected is the fastest way to teach an operator
    /// that the monitor's halts are not worth reading. It is still worth knowing,
    /// because a sudden upward shift is often a sign the fill model is wrong.
    UpperShift,
    /// The series has shifted **down**. A breach.
    LowerShift,
}

impl CusumVerdict {
    /// Whether this verdict is a §4 breach (and therefore a halt).
    #[must_use]
    pub fn is_breach(self) -> bool {
        matches!(self, Self::LowerShift)
    }
}

/// Two-sided CUSUM on a standardised series.
///
/// Sensitive to a small, persistent shift in the mean — which is what a strategy
/// that has quietly stopped working looks like, and is exactly what a threshold on
/// the level of the series is bad at. A drawdown limit fires after the loss; a
/// CUSUM fires while it is accumulating.
#[derive(Clone, Copy, Debug)]
pub struct Cusum {
    /// Slack, in standard deviations. Shifts smaller than this are ignored.
    k: f64,
    /// Decision interval, in standard deviations.
    h: f64,
    high: f64,
    low: f64,
}

impl Default for Cusum {
    /// `k = 0.5`, `h = 5.0`: the textbook pairing for detecting a one-sigma shift
    /// with a long in-control run length. A tighter `h` fires sooner and, on an
    /// intraday return series, fires constantly.
    fn default() -> Self {
        Self::new(0.5, 5.0)
    }
}

impl Cusum {
    #[must_use]
    pub fn new(k: f64, h: f64) -> Self {
        Self {
            k,
            h,
            high: 0.0,
            low: 0.0,
        }
    }

    /// Feed one standardised observation.
    pub fn observe(&mut self, z: f64) -> CusumVerdict {
        self.high = (self.high + z - self.k).max(0.0);
        self.low = (self.low + z + self.k).min(0.0);
        if self.low <= -self.h {
            return CusumVerdict::LowerShift;
        }
        if self.high >= self.h {
            return CusumVerdict::UpperShift;
        }
        CusumVerdict::Inside
    }

    /// Reset after a breach has been acted on, so one shift is one alarm.
    pub fn reset(&mut self) {
        self.high = 0.0;
        self.low = 0.0;
    }

    #[must_use]
    pub fn high(&self) -> f64 {
        self.high
    }

    #[must_use]
    pub fn low(&self) -> f64 {
        self.low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_observation_has_no_band() {
        let mut e = Ewma::new(0.1);
        e.update(1.0);
        assert_eq!(e.z(5.0), None, "one point cannot say how far five is");
    }

    #[test]
    fn a_constant_series_refuses_to_pretend_it_has_a_band() {
        // The adversarial case for the whole ±3σ design: a metric that has never
        // moved has sigma zero, and dividing by it would make every reading either
        // infinitely far out or exactly on the mean. Both are lies, and the second
        // one silently clears every breach.
        let mut e = Ewma::new(0.2);
        for _ in 0..50 {
            e.update(0.9);
        }
        assert_eq!(e.z(0.1), None);
    }

    #[test]
    fn a_seeded_baseline_measures_distance_in_sigmas() {
        let e = Ewma::seeded(0.1, 0.90, 0.02);
        let z = e.z(0.84).expect("seeded baselines have a band");
        assert!(
            (z + 3.0).abs() < 1e-9,
            "0.84 is three sigma below 0.90, got {z}"
        );
    }

    #[test]
    fn the_mean_tracks_a_moved_series() {
        let mut e = Ewma::new(0.3);
        for _ in 0..100 {
            e.update(1.0);
        }
        for _ in 0..100 {
            e.update(5.0);
        }
        assert!(
            (e.mean() - 5.0).abs() < 0.01,
            "mean {} did not follow",
            e.mean()
        );
    }

    #[test]
    fn a_stationary_series_does_not_trip_the_cusum() {
        // Alternating ±0.4 sigma: nothing is wrong, and a detector that alarms
        // here would alarm on every strategy every day.
        let mut c = Cusum::default();
        for i in 0..1000 {
            let z = if i % 2 == 0 { 0.4 } else { -0.4 };
            assert_eq!(c.observe(z), CusumVerdict::Inside, "tripped at {i}");
        }
    }

    #[test]
    fn a_persistent_downward_shift_is_caught() {
        let mut c = Cusum::default();
        let mut fired = None;
        for i in 0..100 {
            if c.observe(-1.0).is_breach() {
                fired = Some(i);
                break;
            }
        }
        let i = fired.expect("a sustained one-sigma drop must be caught");
        assert!(
            i < 20,
            "took {i} observations, which is too slow to be useful"
        );
    }

    #[test]
    fn an_upward_shift_is_a_warning_and_not_a_halt() {
        // Halting a strategy for outperforming is how a monitor loses its
        // audience. Recorded, because it usually means the fill model is wrong.
        let mut c = Cusum::default();
        let mut verdict = CusumVerdict::Inside;
        for _ in 0..50 {
            verdict = c.observe(1.0);
            if verdict != CusumVerdict::Inside {
                break;
            }
        }
        assert_eq!(verdict, CusumVerdict::UpperShift);
        assert!(!verdict.is_breach());
    }

    #[test]
    fn a_reset_makes_one_shift_one_alarm() {
        let mut c = Cusum::default();
        while !c.observe(-1.0).is_breach() {}
        c.reset();
        assert_eq!(c.observe(-1.0), CusumVerdict::Inside);
    }
}
