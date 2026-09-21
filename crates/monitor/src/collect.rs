//! The measurement layer (COMP-007 §3; S-7.5, S-7.7).
//!
//! Everything downstream of this module decides. This module is the part that
//! *measures*: it turns a window's worth of orders and fills into the readings the
//! [`Evaluator`](crate::evaluate::Evaluator) judges against the limits declared at
//! promotion.
//!
//! # Two rules this module exists to keep
//!
//! **An absent measurement is never a zero.** A strategy that placed no orders this
//! window has no fill rate. Reporting `0.0` there would read, on the dashboard and
//! in the event row, as "measured, and it was terrible" — or worse, a metric whose
//! limit is an upper bound would read as "measured, and it was fine". Both are
//! statements nobody made. So every statistic here is a [`Measurement`], which is
//! either a number *and the count of observations behind it*, or an explicit
//! [`Measurement::Unobserved`] carrying why. Only the first kind becomes a
//! [`MetricReading`].
//!
//! **A limit is declared before the drawdown, never after it.** Nothing in this
//! module can see a [`LimitBook`](crate::limits::LimitBook), and nothing in it
//! derives a threshold. A family that computed its band from the same window it
//! was judging would be a family that never fires: the breach moves the band, the
//! band clears the breach, and the monitor reports calm for as long as the
//! degradation is smooth. The readings go out; the comparison happens against a
//! row somebody declared, from a named run, before any of this.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};

use crate::evaluate::MetricReading;
use crate::family::Family;

/// Every metric the collector can emit.
///
/// A closed enum rather than a free string, for the same reason [`Family`] is one:
/// a limit declared against `fill_rate` and a reading emitted as `fillRate` do not
/// fail — they produce [`Outcome::Undeclared`](crate::evaluate::Outcome::Undeclared)
/// for ever, which is a metric that is watched by nothing and says so nowhere the
/// operator looks. The string is written once, here, and both sides read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Metric {
    // ── Execution quality (COMP-007 §3) ──
    FillRate,
    SlippageBpsP50,
    SlippageBpsP95,
    LatencyP95Ms,
    PartialFillShare,
    // ── Behaviour ──
    SignalsPerHour,
    HoldingPeriodP50Secs,
    OrdersPerMinute,
    CancelRatio,
    PositionCount,
    // ── P&L (see `crate::pnl`) ──
    Sharpe30d,
    Sharpe60d,
    Sharpe90d,
    DrawdownPct,
    TimeUnderWaterDays,
    CusumLow,
}

impl Metric {
    /// The string the `monitor.limits` rows and the event records speak.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FillRate => "fill_rate",
            Self::SlippageBpsP50 => "slippage_bps_p50",
            Self::SlippageBpsP95 => "slippage_bps_p95",
            Self::LatencyP95Ms => "latency_p95_ms",
            Self::PartialFillShare => "partial_fill_share",
            Self::SignalsPerHour => "signals_per_hour",
            Self::HoldingPeriodP50Secs => "holding_period_p50_secs",
            Self::OrdersPerMinute => "orders_per_minute",
            Self::CancelRatio => "cancel_ratio",
            Self::PositionCount => "position_count",
            Self::Sharpe30d => "sharpe_30d",
            Self::Sharpe60d => "sharpe_60d",
            Self::Sharpe90d => "sharpe_90d",
            Self::DrawdownPct => "drawdown_pct",
            Self::TimeUnderWaterDays => "time_under_water_days",
            Self::CusumLow => "cusum_low",
        }
    }

    /// The family this metric is scored under (§4.1's precision report).
    pub fn family(self) -> Family {
        match self {
            Self::FillRate
            | Self::SlippageBpsP50
            | Self::SlippageBpsP95
            | Self::LatencyP95Ms
            | Self::PartialFillShare => Family::Execution,
            Self::SignalsPerHour
            | Self::HoldingPeriodP50Secs
            | Self::OrdersPerMinute
            | Self::CancelRatio
            | Self::PositionCount => Family::Behaviour,
            Self::Sharpe30d
            | Self::Sharpe60d
            | Self::Sharpe90d
            | Self::DrawdownPct
            | Self::TimeUnderWaterDays
            | Self::CusumLow => Family::Pnl,
        }
    }

    /// Every metric, so a test can assert the catalogue is complete and a caller
    /// declaring limits can enumerate what there is to declare.
    pub fn all() -> [Self; 16] {
        [
            Self::FillRate,
            Self::SlippageBpsP50,
            Self::SlippageBpsP95,
            Self::LatencyP95Ms,
            Self::PartialFillShare,
            Self::SignalsPerHour,
            Self::HoldingPeriodP50Secs,
            Self::OrdersPerMinute,
            Self::CancelRatio,
            Self::PositionCount,
            Self::Sharpe30d,
            Self::Sharpe60d,
            Self::Sharpe90d,
            Self::DrawdownPct,
            Self::TimeUnderWaterDays,
            Self::CusumLow,
        ]
    }
}

impl std::fmt::Display for Metric {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Metric {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::all()
            .into_iter()
            .find(|m| m.as_str() == s)
            .ok_or_else(|| format!("unknown monitor metric {s:?}"))
    }
}

/// Why a statistic has no value this window.
///
/// Carried rather than collapsed to `None`, because "this strategy placed no
/// orders" and "the venue reported no slippage model for any of these fills" are
/// different problems and the operator's next action differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unobserved {
    /// Nothing happened in the window that this statistic is computed over.
    NoObservations,
    /// Things happened, but none of them carried the input this statistic needs —
    /// for example fills with no recorded fill-model prediction to compare with.
    NoQualifyingObservations,
    /// The window has no duration, so a rate over it is undefined rather than
    /// infinite.
    ZeroLengthWindow,
    /// The statistic exists but has no spread to express it against — a constant
    /// series, whose sigma is zero.
    NoSpread,
}

impl Unobserved {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoObservations => "no_observations",
            Self::NoQualifyingObservations => "no_qualifying_observations",
            Self::ZeroLengthWindow => "zero_length_window",
            Self::NoSpread => "no_spread",
        }
    }
}

impl std::fmt::Display for Unobserved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A statistic, or an honest statement that there is none.
///
/// Deliberately not `Option<f64>` and deliberately without a `Default`, an
/// `unwrap_or_default` shape or a `From<Measurement> for f64`: the only way to get
/// a number out is [`Measurement::value`], which hands back an `Option` the caller
/// has to handle, and the only way to turn one into a reading the monitor acts on
/// is [`Measurement::reading`], which produces nothing when nothing was measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Measurement {
    /// A number, and how many observations stand behind it.
    ///
    /// The count travels with the value because a fill rate computed from two
    /// orders is not a fill rate, and a caller that wants to require a floor
    /// before acting needs the denominator to require it against.
    Measured { value: f64, observations: usize },
    /// Nothing was measured. **Not** zero.
    Unobserved(Unobserved),
}

impl Measurement {
    /// A measured value, refusing a non-finite one.
    ///
    /// A NaN or an infinity here is a computation that failed, and recording it as
    /// a reading would hand `ControlLimit::breached_by` a value it treats as a
    /// breach — an alarm caused by arithmetic rather than by the book.
    pub fn measured(value: f64, observations: usize) -> Self {
        if observations == 0 {
            return Self::Unobserved(Unobserved::NoObservations);
        }
        if !value.is_finite() {
            return Self::Unobserved(Unobserved::NoQualifyingObservations);
        }
        Self::Measured {
            value,
            observations,
        }
    }

    /// A ratio. A zero denominator is unobserved, never zero.
    pub fn ratio(numerator: usize, denominator: usize) -> Self {
        if denominator == 0 {
            return Self::Unobserved(Unobserved::NoObservations);
        }
        Self::measured(numerator as f64 / denominator as f64, denominator)
    }

    /// A rate per unit of time. A zero-length window is unobserved, never
    /// infinite.
    pub fn rate(count: usize, window: &Window, per: Duration) -> Self {
        let secs = window.duration().num_seconds();
        let per_secs = per.num_seconds();
        if secs <= 0 || per_secs <= 0 {
            return Self::Unobserved(Unobserved::ZeroLengthWindow);
        }
        if count == 0 {
            // A window in which the strategy placed no orders *is* a measured
            // rate of zero: the window happened, and nothing came out of it. That
            // is exactly the reading a `signals_per_hour` lower limit exists to
            // catch — a strategy that has silently stopped trading — so
            // suppressing it here would hide the failure the metric was declared
            // for.
            return Self::Measured {
                value: 0.0,
                observations: 0,
            };
        }
        Self::measured(count as f64 * (per_secs as f64 / secs as f64), count)
    }

    /// The `p`-th percentile of a sample, by nearest rank. `p` in `[0, 100]`.
    pub fn percentile(sample: &[f64], p: f64) -> Self {
        let mut xs: Vec<f64> = sample.iter().copied().filter(|x| x.is_finite()).collect();
        if xs.is_empty() {
            return Self::Unobserved(Unobserved::NoQualifyingObservations);
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = xs.len();
        let rank = ((p.clamp(0.0, 100.0) / 100.0) * n as f64).ceil().max(1.0) as usize;
        Self::measured(xs[rank.min(n) - 1], n)
    }

    /// The number, when there is one.
    pub fn value(self) -> Option<f64> {
        match self {
            Self::Measured { value, .. } => Some(value),
            Self::Unobserved(_) => None,
        }
    }

    /// How many observations stand behind this statistic. Zero when unobserved —
    /// which is the one place a zero here is the truth.
    pub fn observations(self) -> usize {
        match self {
            Self::Measured { observations, .. } => observations,
            Self::Unobserved(_) => 0,
        }
    }

    pub fn is_measured(self) -> bool {
        matches!(self, Self::Measured { .. })
    }

    /// Turn this statistic into a reading the evaluator will judge.
    ///
    /// `None` when nothing was measured: an unobserved metric must reach neither
    /// the limit check nor the dwell tracker. Feeding dwell a fabricated "inside"
    /// reading for a metric nobody measured would quietly bank recovery progress
    /// for a strategy the monitor has not looked at.
    pub fn reading(self, strategy_id: &str, metric: Metric) -> Option<MetricReading> {
        self.value()
            .map(|v| MetricReading::new(strategy_id, metric.as_str(), metric.family(), v))
    }
}

/// The closed interval a set of readings describes.
///
/// Carried explicitly rather than implied by "the last cadence", because the event
/// record has to be able to say *over what* a fill rate of 0.41 was measured, and
/// because a collector that fell behind and covered nine minutes must not report
/// its order rate as though it covered one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

impl Window {
    /// The window ending at `to` and running back `length`.
    pub fn ending_at(to: DateTime<Utc>, length: Duration) -> Self {
        Self {
            from: to - length,
            to,
        }
    }

    pub fn duration(&self) -> Duration {
        self.to - self.from
    }

    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.from && at <= self.to
    }
}

/// How an order ended, from the monitor's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderOutcome {
    /// Fully filled.
    Filled,
    /// Some quantity filled and then the order stopped working — in practice a
    /// cancel after a partial fill. Counts as a fill for the partial-fill share
    /// **and** as a cancel for the cancel ratio, because it is both, and folding
    /// it into one of them loses whichever question the other was asking.
    PartiallyFilled,
    /// Cancelled with nothing filled.
    Cancelled,
    Rejected,
    /// Still working. Counts toward neither the fill rate's numerator nor its
    /// denominator: an order that has not finished is not a miss, and counting it
    /// as one makes a busy strategy look like a broken one.
    Working,
}

impl OrderOutcome {
    /// Whether the order has finished and can therefore be scored.
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Working)
    }

    /// Whether any quantity was filled.
    pub fn filled_any(self) -> bool {
        matches!(self, Self::Filled | Self::PartiallyFilled)
    }

    /// Whether the order was cancelled, filled or not.
    pub fn was_cancelled(self) -> bool {
        matches!(self, Self::Cancelled | Self::PartiallyFilled)
    }

    /// Classify a stored order from its lifecycle state and how many fills it
    /// has.
    ///
    /// The fill count is needed because `orders.state` alone cannot answer it: a
    /// partially filled order that is then cancelled is stored as `cancelled`,
    /// and reading that as an unfilled cancel would hide every partial fill the
    /// venue gave and overstate the cancel ratio's unfilled share.
    pub fn observed(state: domain::order::OrderState, fills: usize) -> Self {
        use domain::order::OrderState as S;
        match state {
            S::Filled => Self::Filled,
            S::Cancelled if fills > 0 => Self::PartiallyFilled,
            S::Cancelled => Self::Cancelled,
            S::Rejected => Self::Rejected,
            // `PartiallyFilled` in `domain` means "filled some, *still open*", so
            // it is still working from the monitor's point of view.
            S::PartiallyFilled | S::Accepted | S::Submitted => Self::Working,
        }
    }
}

/// One order the monitor observed, with everything the execution and behaviour
/// families need and nothing else.
#[derive(Clone, Debug, PartialEq)]
pub struct OrderObservation {
    pub strategy_id: String,
    pub instrument_id: String,
    pub placed_at: DateTime<Utc>,
    pub outcome: OrderOutcome,
    /// When the first fill arrived, for the latency statistic.
    pub first_fill_at: Option<DateTime<Utc>>,
    /// Realised slippage against the fill model's prediction, in basis points.
    ///
    /// `None` when no prediction was recorded for this order — which is not zero
    /// slippage, and is why the slippage statistics report
    /// [`Unobserved::NoQualifyingObservations`] rather than a comfortable number
    /// when the fill model is not wired up.
    pub slippage_vs_model_bps: Option<f64>,
}

impl OrderObservation {
    /// Milliseconds from placement to first fill, when both are known.
    ///
    /// A negative interval is discarded rather than clamped: a fill timestamped
    /// before its order is a clock or an ingest fault, and a zero would fold it
    /// into the latency distribution as the fastest fill of the window.
    pub fn latency_ms(&self) -> Option<f64> {
        let first = self.first_fill_at?;
        let ms = (first - self.placed_at).num_milliseconds();
        if ms < 0 {
            return None;
        }
        Some(ms as f64)
    }
}

/// A round trip that closed inside the window, for the holding-period statistic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HoldingPeriod {
    pub opened_at: DateTime<Utc>,
    pub closed_at: DateTime<Utc>,
}

impl HoldingPeriod {
    pub fn seconds(&self) -> Option<f64> {
        let s = (self.closed_at - self.opened_at).num_seconds();
        if s < 0 {
            return None;
        }
        Some(s as f64)
    }
}

/// The execution-quality family for one strategy over one window (COMP-007 §3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExecutionSample {
    pub window: Window,
    pub fill_rate: Measurement,
    pub partial_fill_share: Measurement,
    pub latency_p95_ms: Measurement,
    pub slippage_bps_p50: Measurement,
    pub slippage_bps_p95: Measurement,
}

impl ExecutionSample {
    /// Measure one strategy's execution quality over `window`.
    ///
    /// Orders outside the window are ignored rather than trusted: a collector
    /// handed a wider slice than it asked for must not silently widen the window
    /// the reading claims to describe.
    pub fn measure(window: Window, orders: &[OrderObservation]) -> Self {
        let inside: Vec<&OrderObservation> = orders
            .iter()
            .filter(|o| window.contains(o.placed_at))
            .collect();

        let terminal: Vec<&&OrderObservation> =
            inside.iter().filter(|o| o.outcome.is_terminal()).collect();
        let filled = terminal
            .iter()
            .filter(|o| o.outcome == OrderOutcome::Filled)
            .count();
        let partial = terminal
            .iter()
            .filter(|o| o.outcome == OrderOutcome::PartiallyFilled)
            .count();

        let latencies: Vec<f64> = inside.iter().filter_map(|o| o.latency_ms()).collect();
        let slippages: Vec<f64> = inside
            .iter()
            .filter_map(|o| o.slippage_vs_model_bps)
            .filter(|s| s.is_finite())
            .collect();

        Self {
            window,
            fill_rate: Measurement::ratio(filled, terminal.len()),
            // Denominator is the orders that filled at all, not every terminal
            // order: "what share of the fills were partial" is the question, and
            // dividing by rejected and cancelled orders too would make the share
            // fall whenever the venue rejected more, which is the opposite of what
            // the metric is for.
            partial_fill_share: Measurement::ratio(partial, filled + partial),
            latency_p95_ms: Measurement::percentile(&latencies, 95.0),
            slippage_bps_p50: Measurement::percentile(&slippages, 50.0),
            slippage_bps_p95: Measurement::percentile(&slippages, 95.0),
        }
    }

    /// The readings, omitting every statistic that was not measured.
    pub fn readings(&self, strategy_id: &str) -> Vec<MetricReading> {
        [
            (Metric::FillRate, self.fill_rate),
            (Metric::PartialFillShare, self.partial_fill_share),
            (Metric::LatencyP95Ms, self.latency_p95_ms),
            (Metric::SlippageBpsP50, self.slippage_bps_p50),
            (Metric::SlippageBpsP95, self.slippage_bps_p95),
        ]
        .into_iter()
        .filter_map(|(m, v)| v.reading(strategy_id, m))
        .collect()
    }
}

/// The behaviour family for one strategy over one window (COMP-007 §3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BehaviourSample {
    pub window: Window,
    pub orders_per_minute: Measurement,
    pub cancel_ratio: Measurement,
    pub signals_per_hour: Measurement,
    pub holding_period_p50_secs: Measurement,
    pub position_count: Measurement,
}

impl BehaviourSample {
    /// Measure one strategy's behaviour over `window`.
    ///
    /// `signals` and `positions_held` are both `Option` for the same reason, and
    /// it is the reason this module exists. `Some(0)` signals is a strategy that
    /// emitted none — the reading a `signals_per_hour` floor is declared to catch.
    /// `None` is a caller that does not collect signal counts at all. Passing `0`
    /// for the second would manufacture the first, and the monitor would raise an
    /// alarm about a strategy nobody had looked at. Likewise a position count is a
    /// *snapshot*: `Some(0)` is a flat book, `None` is a snapshot that was not
    /// taken, and collapsing the two turns a failed read of the position table
    /// into a flat book.
    pub fn measure(
        window: Window,
        orders: &[OrderObservation],
        signals: Option<usize>,
        holding_periods: &[HoldingPeriod],
        positions_held: Option<usize>,
    ) -> Self {
        let inside: Vec<&OrderObservation> = orders
            .iter()
            .filter(|o| window.contains(o.placed_at))
            .collect();
        let terminal = inside.iter().filter(|o| o.outcome.is_terminal()).count();
        let cancelled = inside
            .iter()
            .filter(|o| o.outcome.was_cancelled())
            .count();
        let holds: Vec<f64> = holding_periods.iter().filter_map(HoldingPeriod::seconds).collect();

        Self {
            window,
            orders_per_minute: Measurement::rate(inside.len(), &window, Duration::minutes(1)),
            cancel_ratio: Measurement::ratio(cancelled, terminal),
            signals_per_hour: signals.map_or(
                Measurement::Unobserved(Unobserved::NoObservations),
                |n| Measurement::rate(n, &window, Duration::hours(1)),
            ),
            holding_period_p50_secs: Measurement::percentile(&holds, 50.0),
            position_count: positions_held
                .map_or(Measurement::Unobserved(Unobserved::NoObservations), |n| {
                    // One observation: the snapshot itself. Zero positions is a
                    // measurement; no snapshot is not.
                    Measurement::Measured {
                        value: n as f64,
                        observations: 1,
                    }
                }),
        }
    }

    pub fn readings(&self, strategy_id: &str) -> Vec<MetricReading> {
        [
            (Metric::OrdersPerMinute, self.orders_per_minute),
            (Metric::CancelRatio, self.cancel_ratio),
            (Metric::SignalsPerHour, self.signals_per_hour),
            (Metric::HoldingPeriodP50Secs, self.holding_period_p50_secs),
            (Metric::PositionCount, self.position_count),
        ]
        .into_iter()
        .filter_map(|(m, v)| v.reading(strategy_id, m))
        .collect()
    }
}

/// Split a window's observations by strategy, so one collector pass covers the
/// whole book.
///
/// A `BTreeMap` rather than a `HashMap` so the cadence evaluates strategies in a
/// stable order: two runs over the same data must produce the same event sequence,
/// or the audit trail is not reproducible.
pub fn by_strategy(orders: Vec<OrderObservation>) -> BTreeMap<String, Vec<OrderObservation>> {
    let mut out: BTreeMap<String, Vec<OrderObservation>> = BTreeMap::new();
    for o in orders {
        out.entry(o.strategy_id.clone()).or_default().push(o);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn t(mins: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + mins * 60, 0).expect("valid timestamp")
    }

    fn window() -> Window {
        Window {
            from: t(0),
            to: t(60),
        }
    }

    fn order(mins: i64, outcome: OrderOutcome) -> OrderObservation {
        OrderObservation {
            strategy_id: "s".to_owned(),
            instrument_id: "BTC-USD".to_owned(),
            placed_at: t(mins),
            outcome,
            first_fill_at: None,
            slippage_vs_model_bps: None,
        }
    }

    #[test]
    fn every_metric_round_trips_through_its_string_and_names_a_family() {
        // The drift this guards: a limit declared as `fill_rate` and a reading
        // emitted as `fillRate` do not error — they produce `Undeclared` for ever.
        let mut seen: Vec<&'static str> = Vec::new();
        for m in Metric::all() {
            assert_eq!(Metric::from_str(m.as_str()), Ok(m));
            assert!(!seen.contains(&m.as_str()), "duplicate metric name {m}");
            seen.push(m.as_str());
        }
        assert_eq!(seen.len(), 16, "the catalogue must cover every metric");
    }

    #[test]
    fn every_collector_metric_belongs_to_a_family_the_promotion_gate_requires() {
        // COMP-007 §3 orders the families inputs-first, and `REQUIRED_FOR_LIVE`
        // encodes that. A metric in a family outside it would be a measurement no
        // live strategy is obliged to have a limit for.
        for m in Metric::all() {
            assert!(
                crate::limits::REQUIRED_FOR_LIVE.contains(&m.family()),
                "{m} is in the {} family, which no live strategy must declare",
                m.family()
            );
        }
    }

    #[test]
    fn a_strategy_that_placed_no_orders_has_no_fill_rate_rather_than_a_fill_rate_of_zero() {
        // The rule this whole module exists for. A zero here has an upper-bounded
        // metric reading as healthy and a lower-bounded one firing an alarm about
        // a window in which nothing happened; both are statements nobody made.
        let s = ExecutionSample::measure(window(), &[]);
        assert_eq!(
            s.fill_rate,
            Measurement::Unobserved(Unobserved::NoObservations)
        );
        assert_eq!(s.fill_rate.value(), None);
        assert!(
            s.readings("s").is_empty(),
            "an unmeasured window must produce no readings at all"
        );
    }

    #[test]
    fn an_unmeasured_statistic_never_reaches_the_evaluator() {
        // Belt and braces on the same rule, one layer down: not only is the value
        // absent, the reading is too, so dwell never sees a fabricated "inside"
        // observation for a metric nobody measured.
        let m = Measurement::Unobserved(Unobserved::NoQualifyingObservations);
        assert!(m.reading("s", Metric::SlippageBpsP50).is_none());
        assert_eq!(m.observations(), 0);
        assert!(!m.is_measured());
    }

    #[test]
    fn a_fill_rate_counts_only_orders_that_finished() {
        // An order still working is not a miss. Counting it as one makes a busy
        // strategy look broken every cadence, which is the alarm-fatigue failure
        // §4.1 is about.
        let orders = vec![
            order(1, OrderOutcome::Filled),
            order(2, OrderOutcome::Filled),
            order(3, OrderOutcome::Cancelled),
            order(4, OrderOutcome::Working),
            order(5, OrderOutcome::Working),
        ];
        let s = ExecutionSample::measure(window(), &orders);
        let Measurement::Measured {
            value,
            observations,
        } = s.fill_rate
        else {
            panic!("three terminal orders is a measurement");
        };
        assert!((value - 2.0 / 3.0).abs() < 1e-12, "got {value}");
        assert_eq!(observations, 3, "the two working orders are not in the denominator");
    }

    #[test]
    fn a_reading_outside_the_window_is_not_folded_into_it() {
        let mut orders = vec![order(10, OrderOutcome::Filled)];
        orders.push(order(600, OrderOutcome::Rejected));
        let s = ExecutionSample::measure(window(), &orders);
        assert_eq!(s.fill_rate.observations(), 1, "only the in-window order counts");
    }

    #[test]
    fn a_partial_fill_share_divides_by_the_fills_not_by_the_rejections() {
        let orders = vec![
            order(1, OrderOutcome::Filled),
            order(2, OrderOutcome::PartiallyFilled),
            order(3, OrderOutcome::Rejected),
            order(4, OrderOutcome::Rejected),
        ];
        let s = ExecutionSample::measure(window(), &orders);
        let v = s.partial_fill_share.value().expect("two fills is a measurement");
        assert!((v - 0.5).abs() < 1e-12, "got {v}");
    }

    #[test]
    fn slippage_with_no_fill_model_prediction_is_unobserved_and_not_zero() {
        // The dangerous direction on this surface: a fill model that is not wired
        // up would otherwise report perfect agreement with itself.
        let orders = vec![order(1, OrderOutcome::Filled), order(2, OrderOutcome::Filled)];
        let s = ExecutionSample::measure(window(), &orders);
        assert_eq!(
            s.slippage_bps_p50,
            Measurement::Unobserved(Unobserved::NoQualifyingObservations)
        );
        assert_eq!(s.slippage_bps_p95.value(), None);
    }

    #[test]
    fn slippage_percentiles_are_measured_from_the_orders_that_carry_a_prediction() {
        let mut orders = Vec::new();
        for (i, bps) in [1.0, 2.0, 3.0, 40.0].into_iter().enumerate() {
            let mut o = order(i as i64 + 1, OrderOutcome::Filled);
            o.slippage_vs_model_bps = Some(bps);
            orders.push(o);
        }
        orders.push(order(9, OrderOutcome::Filled)); // no prediction
        let s = ExecutionSample::measure(window(), &orders);
        assert_eq!(s.slippage_bps_p50.observations(), 4, "the fifth order carries no prediction");
        let p95 = s.slippage_bps_p95.value().expect("four observations");
        assert!((p95 - 40.0).abs() < 1e-12, "got {p95}");
    }

    #[test]
    fn a_fill_stamped_before_its_order_is_discarded_rather_than_counted_as_instant() {
        let mut o = order(10, OrderOutcome::Filled);
        o.first_fill_at = Some(t(9));
        assert_eq!(o.latency_ms(), None);
        let s = ExecutionSample::measure(window(), &[o]);
        assert_eq!(
            s.latency_p95_ms,
            Measurement::Unobserved(Unobserved::NoQualifyingObservations),
            "a clock fault must not become the fastest fill of the window"
        );
    }

    #[test]
    fn latency_p95_is_the_slow_tail_and_not_the_median() {
        let mut orders = Vec::new();
        for i in 0..20 {
            let mut o = order(i, OrderOutcome::Filled);
            // Eighteen fast fills and a slow tail: a p95 that reported the median
            // would call this venue healthy.
            let lag_ms = if i >= 18 { 5_000 } else { 50 };
            o.first_fill_at = Some(o.placed_at + Duration::milliseconds(lag_ms));
            orders.push(o);
        }
        let s = ExecutionSample::measure(window(), &orders);
        let p95 = s.latency_p95_ms.value().expect("twenty observations");
        assert!((p95 - 5_000.0).abs() < 1e-9, "got {p95}");
    }

    #[test]
    fn a_rate_over_a_zero_length_window_is_unobserved_rather_than_infinite() {
        let w = Window { from: t(5), to: t(5) };
        let b = BehaviourSample::measure(w, &[order(5, OrderOutcome::Filled)], Some(3), &[], None);
        assert_eq!(
            b.orders_per_minute,
            Measurement::Unobserved(Unobserved::ZeroLengthWindow)
        );
        assert_eq!(b.signals_per_hour.value(), None);
    }

    #[test]
    fn a_strategy_that_emitted_no_signals_reports_a_rate_of_zero() {
        // The one place a zero is the measurement rather than its absence: the
        // window happened and nothing came out of it, which is precisely what a
        // `signals_per_hour` lower limit is declared to catch. Suppressing it
        // would hide a strategy that has silently stopped trading.
        let b = BehaviourSample::measure(window(), &[], Some(0), &[], None);
        let v = b.signals_per_hour.value().expect("a rate of zero is a measurement");
        assert!(v.abs() < f64::EPSILON);
        assert!(
            b.readings("s").iter().any(|r| r.metric == Metric::SignalsPerHour.as_str()),
            "and it must reach the evaluator"
        );
    }

    #[test]
    fn an_hourly_rate_is_scaled_to_the_window_the_collector_actually_covered() {
        // A collector that fell behind and covered nine minutes must not report
        // its rate as though it covered one cadence.
        let w = Window { from: t(0), to: t(30) };
        let b = BehaviourSample::measure(w, &[], Some(6), &[], None);
        let v = b.signals_per_hour.value().expect("measured");
        assert!((v - 12.0).abs() < 1e-9, "six signals in half an hour is 12/h, got {v}");
    }

    #[test]
    fn no_position_snapshot_is_not_a_flat_book() {
        let none = BehaviourSample::measure(window(), &[], Some(0), &[], None);
        assert_eq!(none.position_count.value(), None);

        let flat = BehaviourSample::measure(window(), &[], Some(0), &[], Some(0));
        let v = flat.position_count.value().expect("a taken snapshot of zero is a measurement");
        assert!(v.abs() < f64::EPSILON);
    }

    #[test]
    fn a_partly_filled_order_that_was_cancelled_is_both_a_fill_and_a_cancel() {
        // `orders.state` stores a partially filled order that is then cancelled as
        // `cancelled`. Reading that as an unfilled cancel hides every partial fill
        // the venue gave; reading it only as a partial fill loses the cancel.
        use domain::order::OrderState as S;
        assert_eq!(OrderOutcome::observed(S::Cancelled, 2), OrderOutcome::PartiallyFilled);
        assert_eq!(OrderOutcome::observed(S::Cancelled, 0), OrderOutcome::Cancelled);
        assert_eq!(OrderOutcome::observed(S::Filled, 3), OrderOutcome::Filled);
        assert_eq!(OrderOutcome::observed(S::Rejected, 0), OrderOutcome::Rejected);
        // Still open, whatever it has filled so far.
        assert_eq!(OrderOutcome::observed(S::PartiallyFilled, 1), OrderOutcome::Working);
        assert_eq!(OrderOutcome::observed(S::Submitted, 0), OrderOutcome::Working);
        assert_eq!(OrderOutcome::observed(S::Accepted, 0), OrderOutcome::Working);

        assert!(OrderOutcome::PartiallyFilled.was_cancelled());
        assert!(OrderOutcome::PartiallyFilled.filled_any());
        assert!(!OrderOutcome::Rejected.was_cancelled());
        assert!(!OrderOutcome::Working.is_terminal());

        let orders = vec![
            order(1, OrderOutcome::Filled),
            order(2, OrderOutcome::PartiallyFilled),
        ];
        let b = BehaviourSample::measure(window(), &orders, None, &[], None);
        let v = b.cancel_ratio.value().expect("two terminal orders");
        assert!((v - 0.5).abs() < 1e-12, "got {v}");
    }

    #[test]
    fn a_cancel_ratio_needs_terminal_orders_to_divide_by() {
        let b = BehaviourSample::measure(window(), &[order(1, OrderOutcome::Working)], None, &[], None);
        assert_eq!(
            b.cancel_ratio,
            Measurement::Unobserved(Unobserved::NoObservations)
        );
    }

    #[test]
    fn a_holding_period_median_ignores_a_round_trip_that_closed_before_it_opened() {
        let holds = vec![
            HoldingPeriod { opened_at: t(0), closed_at: t(10) },
            HoldingPeriod { opened_at: t(0), closed_at: t(20) },
            HoldingPeriod { opened_at: t(30), closed_at: t(29) },
        ];
        let b = BehaviourSample::measure(window(), &[], None, &holds, None);
        assert_eq!(b.holding_period_p50_secs.observations(), 2);
    }

    #[test]
    fn readings_carry_the_family_the_precision_report_scores_them_under() {
        let orders = vec![order(1, OrderOutcome::Filled), order(2, OrderOutcome::Cancelled)];
        let s = ExecutionSample::measure(window(), &orders);
        let rs = s.readings("s-1");
        assert!(!rs.is_empty(), "two terminal orders must produce at least one reading");
        for r in &rs {
            assert_eq!(r.family, Family::Execution, "{}", r.metric);
            assert_eq!(r.strategy_id, "s-1");
        }
    }

    #[test]
    fn the_collector_splits_the_book_in_a_stable_order() {
        // Two runs over the same data must produce the same event sequence, or
        // the audit trail is not reproducible.
        let mut orders = Vec::new();
        for id in ["s-c", "s-a", "s-b", "s-a"] {
            let mut o = order(1, OrderOutcome::Filled);
            o.strategy_id = id.to_owned();
            orders.push(o);
        }
        let split = by_strategy(orders);
        let ids: Vec<&str> = split.keys().map(String::as_str).collect();
        assert_eq!(ids, ["s-a", "s-b", "s-c"]);
        assert_eq!(split["s-a"].len(), 2);
    }

    #[test]
    fn a_non_finite_statistic_is_refused_rather_than_recorded_as_a_breach() {
        // `ControlLimit::breached_by` treats a NaN as outside, correctly — but an
        // alarm caused by arithmetic rather than by the book is the monitor
        // crying wolf about itself.
        assert!(!Measurement::measured(f64::NAN, 10).is_measured());
        assert!(!Measurement::measured(f64::INFINITY, 10).is_measured());
        assert!(!Measurement::measured(1.0, 0).is_measured());
    }
}
