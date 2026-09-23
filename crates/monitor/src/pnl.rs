//! The P&L family (COMP-007 §3, §4; S-7.8).
//!
//! P&L is the *lagging* family and it is measured last on purpose (§P3). By the
//! time an equity curve says a strategy is broken it has been broken for a while
//! and has been paying for it, which is why execution quality and behaviour drift
//! are watched first. This module is what happens when those did not catch it.
//!
//! # The trap this module is shaped to avoid
//!
//! A control chart standardises its observations against a mean and a spread. If
//! that mean and spread come from the same window the chart is judging, the chart
//! never fires: a strategy losing money smoothly re-centres its own baseline every
//! cadence, the deviations stay near zero, and the monitor reports calm for
//! precisely as long as the decay is gradual. That is not a subtle bug — it is the
//! difference between a detector and a decoration.
//!
//! So [`ReturnBaseline`] can only be *declared*, from a named backtest or paper
//! run, with the annualisation convention that run used. There is no constructor
//! that takes the live series. The statistics below are computed from the live
//! series; the numbers they are measured *against* are not.

use chrono::NaiveDate;

use crate::baseline::{Cusum, CusumVerdict};
use crate::collect::{Measurement, Metric, Unobserved};
use crate::evaluate::MetricReading;

/// Trading days in a year, for an instrument with a session calendar.
pub const EQUITY_PERIODS_PER_YEAR: f64 = 252.0;

/// Days in a year, for a 24/7 market.
///
/// Separate from [`EQUITY_PERIODS_PER_YEAR`] because annualising a crypto series
/// by 252 understates its Sharpe by about 15%, and a live number quietly 15% below
/// the backtest it is compared with is a warning nobody can act on.
pub const CONTINUOUS_PERIODS_PER_YEAR: f64 = 365.0;

/// The rolling Sharpe windows COMP-007 §3 names.
pub const SHARPE_WINDOWS: [u32; 3] = [30, 60, 90];

/// One day's return, aligned on the date it closed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DailyReturn {
    pub date: NaiveDate,
    /// Simple return for the day, as a fraction — `0.004` is forty basis points.
    pub value: f64,
}

/// Why a baseline was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaselineRefusal {
    /// No run was named. A baseline whose provenance is unrecorded is a number
    /// somebody typed, and the whole difference between a control limit and a
    /// guess is that one of them can be traced to a measurement.
    UndeclaredProvenance,
    /// A spread of zero or worse. Standardising against it produces an infinity
    /// for every observation, which reads as a breach on every cadence.
    NoSpread,
    /// An annualisation convention of zero or worse.
    NoAnnualisation,
}

impl std::fmt::Display for BaselineRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UndeclaredProvenance => {
                f.write_str("a return baseline must name the run it was measured from")
            }
            Self::NoSpread => f.write_str("a return baseline must have a positive sigma"),
            Self::NoAnnualisation => {
                f.write_str("a return baseline must have a positive periods-per-year")
            }
        }
    }
}

impl std::error::Error for BaselineRefusal {}

/// The expected return distribution, declared at promotion.
///
/// Its fields are private and there is **no** constructor taking a series of
/// returns. That absence is the mechanism: the only way to obtain one is
/// [`ReturnBaseline::declared`], which demands the name of the run it came from,
/// so a baseline silently re-derived from the window being judged has nowhere to
/// come from.
#[derive(Clone, Debug, PartialEq)]
pub struct ReturnBaseline {
    mean: f64,
    sigma: f64,
    periods_per_year: f64,
    declared_from: String,
}

impl ReturnBaseline {
    /// Declare a baseline from a named backtest or paper run.
    pub fn declared(
        mean: f64,
        sigma: f64,
        periods_per_year: f64,
        declared_from: &str,
    ) -> Result<Self, BaselineRefusal> {
        if declared_from.trim().is_empty() {
            return Err(BaselineRefusal::UndeclaredProvenance);
        }
        if !sigma.is_finite() || sigma <= f64::EPSILON || !mean.is_finite() {
            return Err(BaselineRefusal::NoSpread);
        }
        if !periods_per_year.is_finite() || periods_per_year <= 0.0 {
            return Err(BaselineRefusal::NoAnnualisation);
        }
        Ok(Self {
            mean,
            sigma,
            periods_per_year,
            declared_from: declared_from.to_owned(),
        })
    }

    /// The run this baseline was measured from, for the event record.
    pub fn declared_from(&self) -> &str {
        &self.declared_from
    }

    pub fn periods_per_year(&self) -> f64 {
        self.periods_per_year
    }

    /// How many declared sigmas a day's return sits from the declared mean.
    pub fn standardise(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        Some((value - self.mean) / self.sigma)
    }
}

/// The P&L family for one strategy, measured over its live return series.
#[derive(Clone, Debug, PartialEq)]
pub struct PnlSample {
    /// The date of the most recent return in the series, so an event can say what
    /// the reading is as of.
    pub as_of: Option<NaiveDate>,
    returns: Vec<f64>,
    baseline: ReturnBaseline,
}

impl PnlSample {
    /// Measure a strategy's P&L family against its **declared** baseline.
    ///
    /// The series is sorted by date and de-duplicated on it: two rows for one day
    /// are a double-count, and a double-counted return moves both the Sharpe and
    /// the drawdown in whichever direction that day went.
    pub fn measure(returns: &[DailyReturn], baseline: ReturnBaseline) -> Self {
        let mut rows: Vec<DailyReturn> = returns
            .iter()
            .copied()
            .filter(|r| r.value.is_finite())
            .collect();
        rows.sort_by_key(|r| r.date);
        rows.dedup_by_key(|r| r.date);
        Self {
            as_of: rows.last().map(|r| r.date),
            returns: rows.iter().map(|r| r.value).collect(),
            baseline,
        }
    }

    pub fn baseline(&self) -> &ReturnBaseline {
        &self.baseline
    }

    pub fn len(&self) -> usize {
        self.returns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.returns.is_empty()
    }

    /// Annualised Sharpe over the most recent `days` returns.
    ///
    /// Unobserved on a series shorter than the window: a "30-day Sharpe" computed
    /// from nine days is not a 30-day Sharpe, and comparing it with a limit
    /// declared from a 30-day backtest statistic is comparing two different
    /// numbers. Unobserved too on a flat series, whose sigma is zero — for the
    /// same reason [`Ewma::z`](crate::baseline::Ewma::z) refuses one.
    pub fn sharpe(&self, days: u32) -> Measurement {
        let n = days as usize;
        if n == 0 {
            return Measurement::Unobserved(Unobserved::NoObservations);
        }
        if self.returns.len() < n {
            return Measurement::Unobserved(Unobserved::NoObservations);
        }
        let tail = &self.returns[self.returns.len() - n..];
        let mean = tail.iter().sum::<f64>() / n as f64;
        if n < 2 {
            return Measurement::Unobserved(Unobserved::NoSpread);
        }
        let var = tail.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0);
        let sd = var.sqrt();
        if !sd.is_finite() || sd <= f64::EPSILON {
            return Measurement::Unobserved(Unobserved::NoSpread);
        }
        Measurement::measured((mean / sd) * self.baseline.periods_per_year.sqrt(), n)
    }

    /// Maximum peak-to-trough drawdown over the series, as a positive percentage.
    ///
    /// Positive and upward-bounded so the limit reads the way an operator says it:
    /// "no more than 18%", declared from the backtest's 95th-percentile Monte
    /// Carlo drawdown. A strategy that only ever went up measures `0.0`, which is
    /// a measurement — the series existed and never fell.
    pub fn max_drawdown_pct(&self) -> Measurement {
        if self.returns.is_empty() {
            return Measurement::Unobserved(Unobserved::NoObservations);
        }
        let mut equity = 1.0_f64;
        let mut peak = 1.0_f64;
        let mut worst = 0.0_f64;
        for r in &self.returns {
            equity *= 1.0 + r;
            peak = peak.max(equity);
            if peak > 0.0 {
                worst = worst.max((peak - equity) / peak);
            }
        }
        Measurement::measured(worst * 100.0, self.returns.len())
    }

    /// Days since the equity curve last set a high, at the end of the series.
    pub fn time_under_water_days(&self) -> Measurement {
        if self.returns.is_empty() {
            return Measurement::Unobserved(Unobserved::NoObservations);
        }
        let mut equity = 1.0_f64;
        let mut peak = 1.0_f64;
        let mut since_peak = 0_u32;
        for r in &self.returns {
            equity *= 1.0 + r;
            if equity >= peak {
                peak = equity;
                since_peak = 0;
            } else {
                since_peak += 1;
            }
        }
        Measurement::measured(f64::from(since_peak), self.returns.len())
    }

    /// Feed the whole series through a CUSUM, standardised against the **declared**
    /// baseline.
    ///
    /// Returns the verdict and the chart's low statistic. The low statistic is the
    /// number the event record carries: "CUSUM breach" tells an operator nothing
    /// about how far past the decision interval the series went.
    pub fn run_cusum(&self, cusum: &mut Cusum) -> (CusumVerdict, Measurement) {
        if self.returns.is_empty() {
            return (
                CusumVerdict::Inside,
                Measurement::Unobserved(Unobserved::NoObservations),
            );
        }
        let mut verdict = CusumVerdict::Inside;
        let mut fed = 0usize;
        for r in &self.returns {
            let Some(z) = self.baseline.standardise(*r) else {
                continue;
            };
            fed += 1;
            let v = cusum.observe(z);
            if v != CusumVerdict::Inside {
                verdict = v;
            }
        }
        (verdict, Measurement::measured(cusum.low(), fed))
    }

    /// The readings, omitting every statistic that was not measured.
    ///
    /// `cusum_low` is emitted as an ordinary reading rather than acted on directly,
    /// so it passes through the declared limit and through dwell like every other
    /// estimate. COMP-007 §4.1 exempts exactly three things from dwell —
    /// reconciliation mismatch, lease loss and hard staleness — and a CUSUM is not
    /// one of them. Its family is [`Pnl`](crate::family::Family::Pnl), so a
    /// sustained breach of it halts the strategy through
    /// [`on_sustained`](crate::state::on_sustained) with no special case here.
    pub fn readings(&self, strategy_id: &str, cusum: &mut Cusum) -> Vec<MetricReading> {
        let (_, low) = self.run_cusum(cusum);
        let mut out: Vec<MetricReading> = Vec::new();
        for days in SHARPE_WINDOWS {
            let metric = match days {
                30 => Metric::Sharpe30d,
                60 => Metric::Sharpe60d,
                _ => Metric::Sharpe90d,
            };
            if let Some(r) = self.sharpe(days).reading(strategy_id, metric) {
                out.push(r);
            }
        }
        for (metric, m) in [
            (Metric::DrawdownPct, self.max_drawdown_pct()),
            (Metric::TimeUnderWaterDays, self.time_under_water_days()),
            (Metric::CusumLow, low),
        ] {
            if let Some(r) = m.reading(strategy_id, metric) {
                out.push(r);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::Family;

    fn day(i: i64) -> NaiveDate {
        NaiveDate::from_num_days_from_ce_opt(738_000 + i as i32).expect("valid date")
    }

    fn baseline() -> ReturnBaseline {
        ReturnBaseline::declared(0.002, 0.010, CONTINUOUS_PERIODS_PER_YEAR, "backtest-run-41")
            .expect("a declared baseline")
    }

    fn series(values: &[f64]) -> Vec<DailyReturn> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| DailyReturn {
                date: day(i as i64),
                value: *v,
            })
            .collect()
    }

    #[test]
    fn a_baseline_must_name_the_run_it_came_from() {
        // P2: a limit is set before the drawdown, from a named measurement. A
        // baseline with no provenance is a number somebody typed.
        assert_eq!(
            ReturnBaseline::declared(0.0, 0.01, 252.0, "   "),
            Err(BaselineRefusal::UndeclaredProvenance)
        );
        assert_eq!(
            ReturnBaseline::declared(0.0, 0.0, 252.0, "run-1"),
            Err(BaselineRefusal::NoSpread)
        );
        assert_eq!(
            ReturnBaseline::declared(0.0, 0.01, 0.0, "run-1"),
            Err(BaselineRefusal::NoAnnualisation)
        );
        assert!(ReturnBaseline::declared(0.0, 0.01, 252.0, "run-1").is_ok());
    }

    #[test]
    fn a_baseline_taken_from_the_window_it_judges_would_never_fire() {
        // The whole reason `ReturnBaseline` has no constructor taking a series.
        //
        // Here is a strategy losing eighty basis points a day, every day, for a
        // hundred days — the exact shape the P&L family exists to catch, and a
        // full sigma below the twenty-basis-point day the backtest declared.
        // Judged against a baseline re-derived from that same window, the CUSUM
        // never fires, because the decay has moved the mean it is measured
        // against. Judged against the baseline declared from the backtest, it
        // fires within a fortnight. Both charts see identical data.
        let values: Vec<f64> = (0..100)
            .map(|i| -0.008 + if i % 2 == 0 { 0.0005 } else { -0.0005 })
            .collect();
        let rows = series(&values);

        let window_mean = values.iter().sum::<f64>() / values.len() as f64;
        let window_var = values
            .iter()
            .map(|v| (v - window_mean).powi(2))
            .sum::<f64>()
            / (values.len() as f64 - 1.0);
        let self_referential = ReturnBaseline::declared(
            window_mean,
            window_var.sqrt(),
            CONTINUOUS_PERIODS_PER_YEAR,
            "the window it is judging",
        )
        .expect("a spread exists");

        let mut c = Cusum::default();
        let (verdict, _) = PnlSample::measure(&rows, self_referential).run_cusum(&mut c);
        assert_eq!(
            verdict,
            CusumVerdict::Inside,
            "a chart centred on its own window cannot see the shift that moved it"
        );

        let mut c = Cusum::default();
        let (verdict, low) = PnlSample::measure(&rows, baseline()).run_cusum(&mut c);
        assert!(
            verdict.is_breach(),
            "against the declared baseline the same series is a breach"
        );
        let l = low.value().expect("a fed chart has a low statistic");
        assert!(l < 0.0, "and the event must be able to name how far: {l}");
    }

    #[test]
    fn a_cusum_breach_reaches_the_halt_through_the_ordinary_state_machine() {
        // COMP-007 §4: a P&L CUSUM breach halts the strategy. There is no special
        // case for it in this module — `cusum_low` is in the Pnl family, and that
        // is what makes `on_sustained` halt rather than scale.
        assert_eq!(Metric::CusumLow.family(), Family::Pnl);
        let t = crate::state::on_sustained(
            crate::state::StrategyState::Nominal,
            crate::state::Severity::Breach(Metric::CusumLow.family()),
            crate::state::HaltPolicy::Flatten,
        );
        assert_eq!(t.to, crate::state::StrategyState::Halted);
    }

    #[test]
    fn a_thirty_day_sharpe_is_unobserved_on_nine_days_of_data() {
        // Not "provisional", not a number computed from what there is: a 30-day
        // Sharpe from nine days is a different statistic, and comparing it with a
        // limit declared from a 30-day backtest figure compares two things.
        let s = PnlSample::measure(&series(&[0.01; 9]), baseline());
        assert_eq!(
            s.sharpe(30),
            Measurement::Unobserved(Unobserved::NoObservations)
        );
        assert_eq!(s.sharpe(30).value(), None);
    }

    #[test]
    fn a_flat_series_has_no_sharpe_rather_than_an_infinite_one() {
        let s = PnlSample::measure(&series(&[0.001; 40]), baseline());
        assert_eq!(s.sharpe(30), Measurement::Unobserved(Unobserved::NoSpread));
    }

    #[test]
    fn a_sharpe_is_annualised_by_the_convention_the_baseline_declared() {
        // A crypto series annualised by 252 reads about 15% below the backtest it
        // is compared with, which is a warning nobody can act on. The convention
        // travels with the declaration so both sides use the same one.
        let values: Vec<f64> = (0..60)
            .map(|i| if i % 2 == 0 { 0.012 } else { 0.008 })
            .collect();
        let rows = series(&values);

        let crypto = PnlSample::measure(&rows, baseline())
            .sharpe(60)
            .value()
            .expect("sixty observations with spread");
        let equity_baseline =
            ReturnBaseline::declared(0.002, 0.010, EQUITY_PERIODS_PER_YEAR, "backtest-run-41")
                .expect("declared");
        let equity = PnlSample::measure(&rows, equity_baseline)
            .sharpe(60)
            .value()
            .expect("sixty observations with spread");

        let ratio = crypto / equity;
        let expected = (CONTINUOUS_PERIODS_PER_YEAR / EQUITY_PERIODS_PER_YEAR).sqrt();
        assert!(
            (ratio - expected).abs() < 1e-9,
            "got {ratio}, expected {expected}"
        );
    }

    #[test]
    fn a_drawdown_is_measured_peak_to_trough_and_reported_positive() {
        // Down 20% then up 10%: the trough is 20% below the peak, and a later
        // partial recovery does not shrink the worst drawdown the series had.
        let s = PnlSample::measure(&series(&[-0.20, 0.10]), baseline());
        let d = s.max_drawdown_pct().value().expect("two observations");
        assert!((d - 20.0).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn a_series_that_only_rose_measures_a_drawdown_of_zero() {
        // The distinction that matters: this is zero because it was measured, and
        // the series existed. An empty series is not.
        let s = PnlSample::measure(&series(&[0.01; 10]), baseline());
        let d = s.max_drawdown_pct().value().expect("ten observations");
        assert!(d.abs() < 1e-12);

        let empty = PnlSample::measure(&[], baseline());
        assert_eq!(
            empty.max_drawdown_pct(),
            Measurement::Unobserved(Unobserved::NoObservations)
        );
        assert_eq!(
            empty.time_under_water_days(),
            Measurement::Unobserved(Unobserved::NoObservations)
        );
    }

    #[test]
    fn a_strategy_with_no_returns_reports_nothing_rather_than_a_healthy_book() {
        // The dangerous direction on this surface. A strategy whose return series
        // failed to load must not read as "no drawdown, Sharpe fine".
        let empty = PnlSample::measure(&[], baseline());
        let mut c = Cusum::default();
        assert!(
            empty.readings("s-1", &mut c).is_empty(),
            "an unmeasured P&L family produces no readings at all"
        );
        assert!(empty.is_empty());
        assert_eq!(empty.as_of, None);
    }

    #[test]
    fn time_under_water_counts_from_the_last_high() {
        let s = PnlSample::measure(&series(&[0.05, -0.01, -0.01, -0.01]), baseline());
        let d = s
            .time_under_water_days()
            .value()
            .expect("four observations");
        assert!((d - 3.0).abs() < 1e-12, "got {d}");

        let at_high = PnlSample::measure(&series(&[-0.05, 0.10]), baseline());
        let d = at_high
            .time_under_water_days()
            .value()
            .expect("two observations");
        assert!(
            d.abs() < 1e-12,
            "a new high is zero days under water, got {d}"
        );
    }

    #[test]
    fn two_rows_for_one_day_are_not_counted_twice() {
        // A double-counted return moves both the Sharpe and the drawdown in
        // whichever direction that day went, and a duplicated bad day is the one
        // that would move them furthest.
        let mut rows = series(&[-0.10, 0.01]);
        rows.push(DailyReturn {
            date: day(0),
            value: -0.10,
        });
        let s = PnlSample::measure(&rows, baseline());
        assert_eq!(s.len(), 2);
        let d = s.max_drawdown_pct().value().expect("two observations");
        assert!((d - 10.0).abs() < 1e-9, "got {d}");
    }

    #[test]
    fn the_series_is_ordered_by_date_whatever_order_it_arrived_in() {
        // A drawdown computed over rows in arrival order is a drawdown of a path
        // that never happened.
        let ordered = PnlSample::measure(&series(&[-0.20, 0.10, 0.10]), baseline());
        let mut shuffled = series(&[-0.20, 0.10, 0.10]);
        shuffled.reverse();
        let out_of_order = PnlSample::measure(&shuffled, baseline());
        assert_eq!(ordered.max_drawdown_pct(), out_of_order.max_drawdown_pct());
        assert_eq!(ordered.as_of, out_of_order.as_of);
    }

    #[test]
    fn a_stationary_series_at_the_declared_mean_does_not_trip() {
        // The false-positive side. A monitor that halts a strategy behaving
        // exactly as declared is the alarm-fatigue failure of §4.1.
        let values: Vec<f64> = (0..200)
            .map(|i| 0.002 + if i % 2 == 0 { 0.004 } else { -0.004 })
            .collect();
        let s = PnlSample::measure(&series(&values), baseline());
        let mut c = Cusum::default();
        let (verdict, _) = s.run_cusum(&mut c);
        assert_eq!(verdict, CusumVerdict::Inside);
    }

    #[test]
    fn a_measured_pnl_family_produces_readings_in_the_pnl_family() {
        let values: Vec<f64> = (0..95)
            .map(|i| if i % 3 == 0 { 0.004 } else { -0.001 })
            .collect();
        let s = PnlSample::measure(&series(&values), baseline());
        let mut c = Cusum::default();
        let rs = s.readings("s-1", &mut c);
        assert!(!rs.is_empty(), "ninety-five days must produce readings");
        for r in &rs {
            assert_eq!(r.family, Family::Pnl, "{}", r.metric);
            assert_eq!(r.strategy_id, "s-1");
        }
        let names: Vec<&str> = rs.iter().map(|r| r.metric.as_str()).collect();
        for expected in [
            Metric::Sharpe30d,
            Metric::Sharpe60d,
            Metric::Sharpe90d,
            Metric::DrawdownPct,
            Metric::TimeUnderWaterDays,
            Metric::CusumLow,
        ] {
            assert!(names.contains(&expected.as_str()), "missing {expected}");
        }
    }
}
