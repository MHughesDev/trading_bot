//! Layer 1 research strategies: a target-position series, the Gate-0 truncation
//! test and a bar evaluator (Set M, FEAT-004 §2, FEAT-005 §4.2).
//!
//! # What a position series is, and what it is not
//!
//! A research strategy is a function from data to a *target position at each
//! decision time*. The platform runs it, converts the position changes into fills
//! under an explicit execution policy and cost model, and computes the P&L
//! itself. It never accepts a P&L from the strategy (ST-02): [`PositionSeries`]
//! has no such field and `deny_unknown_fields` makes a client that sends one an
//! error rather than a value that is read and ignored.
//!
//! # The truncation test
//!
//! The one way a free-form function can lie is to look at data it should not have
//! had: a `bfill`, a full-sample z-score, a centred window. All of them produce a
//! series that looks like alpha and is look-ahead. The test that catches them
//! asks a single question: *does the function give the same earlier answers when
//! the later data does not exist?* [`truncation_points`] picks K stratified,
//! seeded cut points; the caller re-runs the function on data ending at each; and
//! [`check_truncation`] requires every position at or before the cut to be
//! **exactly** the one the full run produced, with NaN equal to NaN. A failure
//! names the first timestamp that differs, because that is where the function
//! first used the future.
//!
//! # The evaluator
//!
//! [`evaluate`] is deliberately the simplest honest evaluator: decide on bar *t*'s
//! close, fill at bar *t+1*'s open (FEAT-005 §3), pay a fee and a half-spread on
//! every fill. It is also the shape of the vectorised tier 0 (M-3.2), and like it
//! it is never used for a gate verdict on its own.
//!
//! Orders are emitted only when the effective target changes. A target of 0.5 of
//! equity that drifts to 0.55 because the price rose is not a decision, and
//! rebalancing it every bar would charge costs for a choice nobody made.

use serde::{Deserialize, Serialize};

use crate::rng::DetRng;

/// The stable code a failing truncation test carries into a gate verdict.
pub const TRUNCATION_CODE: &str = "leakage.position_truncation_mismatch";

/// The code a series carries when the truncation test was not run at all.
pub const TRUNCATION_NOT_RUN_CODE: &str = "leakage.position_truncation_not_run";

/// How a target position is expressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionUnits {
    /// A fraction of equity, in `[-1, 1]`.
    Fraction,
    /// Units of the instrument.
    Units,
}

/// One target. `None` is NaN on the wire, which JSON cannot carry: no change.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionPoint {
    /// The decision time, nanoseconds since the epoch, bar-close stamped.
    pub ts_ns: i64,
    /// The target, or `None` for "no change".
    pub target: Option<f64>,
}

impl PositionPoint {
    /// Build a point, mapping NaN to "no change" as FEAT-004 §2.1 requires.
    #[must_use]
    pub fn new(ts_ns: i64, target: f64) -> Self {
        Self {
            ts_ns,
            target: if target.is_nan() { None } else { Some(target) },
        }
    }
}

/// A research strategy's output. There is no P&L field, by construction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionSeries {
    pub position_units: PositionUnits,
    /// Explicit, default false (FEAT-004 §2.1).
    #[serde(default)]
    pub allow_short: bool,
    pub points: Vec<PositionPoint>,
}

/// Why a series is refused, naming the first offender.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum PositionSeriesError {
    #[error("the series has no points")]
    Empty,
    #[error(
        "point {index} at {ts_ns} is not after the one before it; the index must strictly increase"
    )]
    NotIncreasing { index: usize, ts_ns: i64 },
    #[error(
        "{ts_ns} is not a decision time; the output index must be a subset of the decision times"
    )]
    NotADecisionTime { ts_ns: i64 },
    #[error("the target at {ts_ns} is not finite")]
    NonFinite { ts_ns: i64 },
    #[error("the target {value} at {ts_ns} is outside [{low}, {high}]")]
    OutOfBounds {
        ts_ns: i64,
        value: f64,
        low: f64,
        high: f64,
    },
    #[error("the target {value} at {ts_ns} is short and `allow_short` is false")]
    ShortNotAllowed { ts_ns: i64, value: f64 },
}

impl PositionSeries {
    /// Check the validity rules. `decision_times`, when given, must contain every
    /// point's timestamp.
    ///
    /// # Errors
    /// The first violation, in timestamp order.
    pub fn validate(&self, decision_times: Option<&[i64]>) -> Result<(), PositionSeriesError> {
        if self.points.is_empty() {
            return Err(PositionSeriesError::Empty);
        }
        let mut prev: Option<i64> = None;
        for (index, p) in self.points.iter().enumerate() {
            if prev.is_some_and(|q| p.ts_ns <= q) {
                return Err(PositionSeriesError::NotIncreasing {
                    index,
                    ts_ns: p.ts_ns,
                });
            }
            prev = Some(p.ts_ns);
            if let Some(times) = decision_times {
                if times.binary_search(&p.ts_ns).is_err() {
                    return Err(PositionSeriesError::NotADecisionTime { ts_ns: p.ts_ns });
                }
            }
            let Some(v) = p.target else { continue };
            if !v.is_finite() {
                return Err(PositionSeriesError::NonFinite { ts_ns: p.ts_ns });
            }
            let (low, high) = match self.position_units {
                PositionUnits::Fraction => (-1.0, 1.0),
                PositionUnits::Units => (f64::MIN, f64::MAX),
            };
            if v < low || v > high {
                return Err(PositionSeriesError::OutOfBounds {
                    ts_ns: p.ts_ns,
                    value: v,
                    low,
                    high,
                });
            }
            if v < 0.0 && !self.allow_short {
                return Err(PositionSeriesError::ShortNotAllowed {
                    ts_ns: p.ts_ns,
                    value: v,
                });
            }
        }
        Ok(())
    }

    /// The target in force at each point: "no change" carries the previous
    /// target forward, and before the first explicit one the target is flat.
    #[must_use]
    pub fn effective(&self) -> Vec<(i64, f64)> {
        let mut cur = 0.0;
        self.points
            .iter()
            .map(|p| {
                if let Some(v) = p.target {
                    cur = v;
                }
                (p.ts_ns, cur)
            })
            .collect()
    }
}

// ── The truncation test ──────────────────────────────────────────────────────

/// K cut points, one per equal slice of the decision times, seeded.
///
/// Stratified rather than uniform: independent uniform draws cluster, and a
/// cluster leaves a whole regime without a cut. The final decision time is never
/// a cut — truncating there removes nothing and compares a series with itself,
/// which would pass for the wrong reason. If there are fewer candidate cuts than
/// `n`, every candidate is used.
#[must_use]
pub fn truncation_points(seed: u64, n: usize, decision_times: &[i64]) -> Vec<i64> {
    // Candidate indices are 0..m, excluding the final decision time.
    let m = decision_times.len().saturating_sub(1);
    if n == 0 || m == 0 {
        return Vec::new();
    }
    if m <= n {
        return decision_times[..m].to_vec();
    }
    let mut rng = DetRng::new(seed);
    (0..n)
        .map(|k| {
            let lo = k * m / n;
            let hi = (k + 1) * m / n;
            decision_times[lo + rng.below(hi - lo)]
        })
        .collect()
}

/// A re-run of the function on data ending at `cut_ns`.
#[derive(Clone, Debug, PartialEq)]
pub struct TruncationCase {
    pub cut_ns: i64,
    pub truncated: PositionSeries,
}

/// Where the function first used the future.
#[derive(Clone, Debug, PartialEq)]
pub struct TruncationFailure {
    pub cut_ns: i64,
    pub first_differing_ns: i64,
    pub full: Option<f64>,
    pub truncated: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TruncationVerdict {
    Pass {
        cuts_checked: usize,
    },
    Fail(TruncationFailure),
    /// No case was supplied. An unchecked series is not a passed one.
    NotComputable {
        reason: String,
    },
}

impl TruncationVerdict {
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self, Self::Pass { .. })
    }

    /// The Gate-0 flag this verdict raises, if any.
    ///
    /// A failure raises [`TRUNCATION_CODE`] naming the first differing timestamp, and
    /// so does **a test that was never run**: an unchecked series is not a passed
    /// one, so the gate gets a flag for it too, with its own code. Only a pass raises
    /// nothing. This is the value a gate worker attaches to a `position_series` run
    /// beside `leakage.model_trained_past_decision` (M-1.7).
    #[must_use]
    pub fn gate_flag(&self) -> Option<crate::run::Flag> {
        match self {
            Self::Pass { .. } => None,
            Self::Fail(_) => self.flag_detail().map(|detail| crate::run::Flag {
                code: TRUNCATION_CODE.into(),
                detail,
            }),
            Self::NotComputable { reason } => Some(crate::run::Flag {
                code: TRUNCATION_NOT_RUN_CODE.into(),
                detail: format!("the truncation test did not run: {reason}"),
            }),
        }
    }

    /// The Gate-0 flag text for a failure.
    #[must_use]
    pub fn flag_detail(&self) -> Option<String> {
        match self {
            Self::Fail(f) => Some(format!(
                "first at {}: re-run on data ending at {} gave {:?}, the full run gave {:?}; the \
                 function read data after its decision time",
                chrono::DateTime::from_timestamp_nanos(f.first_differing_ns).to_rfc3339(),
                chrono::DateTime::from_timestamp_nanos(f.cut_ns).to_rfc3339(),
                f.truncated,
                f.full
            )),
            _ => None,
        }
    }
}

/// Exact equality of two targets, `None == None` (NaN == NaN).
#[allow(clippy::float_cmp)] // bit-exact by definition: this is the determinism check
fn same(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Compare each truncated re-run with the full run on every timestamp at or
/// before its cut.
///
/// Exact, not within a tolerance: a function that is causal returns the same
/// bits, and a tolerance is a way of calling a small look-ahead acceptable.
#[must_use]
pub fn check_truncation(full: &PositionSeries, cases: &[TruncationCase]) -> TruncationVerdict {
    if cases.is_empty() {
        return TruncationVerdict::NotComputable {
            reason: "no truncation case was run".into(),
        };
    }
    let mut ordered: Vec<&TruncationCase> = cases.iter().collect();
    ordered.sort_by_key(|c| c.cut_ns);
    for case in ordered {
        let full_before: Vec<&PositionPoint> = full
            .points
            .iter()
            .filter(|p| p.ts_ns <= case.cut_ns)
            .collect();
        let trunc_before: Vec<&PositionPoint> = case
            .truncated
            .points
            .iter()
            .filter(|p| p.ts_ns <= case.cut_ns)
            .collect();
        let mut i = 0;
        let mut j = 0;
        while i < full_before.len() || j < trunc_before.len() {
            match (full_before.get(i), trunc_before.get(j)) {
                (Some(a), Some(b)) if a.ts_ns == b.ts_ns => {
                    if !same(a.target, b.target) {
                        return TruncationVerdict::Fail(TruncationFailure {
                            cut_ns: case.cut_ns,
                            first_differing_ns: a.ts_ns,
                            full: a.target,
                            truncated: b.target,
                        });
                    }
                    i += 1;
                    j += 1;
                }
                // A decision time one run has and the other lacks is a difference.
                (Some(a), Some(b)) if a.ts_ns < b.ts_ns => {
                    return TruncationVerdict::Fail(TruncationFailure {
                        cut_ns: case.cut_ns,
                        first_differing_ns: a.ts_ns,
                        full: a.target,
                        truncated: None,
                    });
                }
                (Some(_) | None, Some(b)) => {
                    return TruncationVerdict::Fail(TruncationFailure {
                        cut_ns: case.cut_ns,
                        first_differing_ns: b.ts_ns,
                        full: None,
                        truncated: b.target,
                    });
                }
                (Some(a), None) => {
                    return TruncationVerdict::Fail(TruncationFailure {
                        cut_ns: case.cut_ns,
                        first_differing_ns: a.ts_ns,
                        full: a.target,
                        truncated: None,
                    });
                }
                (None, None) => break,
            }
        }
    }
    TruncationVerdict::Pass {
        cuts_checked: cases.len(),
    }
}

// ── Bars, costs and the evaluator ────────────────────────────────────────────

/// A bar, stamped at its close. A decision made on it fills at the **next** bar's open.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar {
    pub ts_ns: i64,
    pub open: f64,
    pub close: f64,
}

/// Run the strategy function on bars up to each cut and compare with the full run.
///
/// `f` receives only the bars it is allowed to see. The Rust form of the Python
/// harness in `apps/skill-runner/research_strategy.py`, used to test the
/// comparison against functions with a known defect.
pub fn truncation_test<F>(f: F, bars: &[Bar], seed: u64, k: usize) -> TruncationVerdict
where
    F: Fn(&[Bar]) -> PositionSeries,
{
    let full = f(bars);
    let times: Vec<i64> = bars.iter().map(|b| b.ts_ns).collect();
    let cases: Vec<TruncationCase> = truncation_points(seed, k, &times)
        .into_iter()
        .map(|cut| {
            let end = bars.partition_point(|b| b.ts_ns <= cut);
            TruncationCase {
                cut_ns: cut,
                truncated: f(&bars[..end]),
            }
        })
        .collect();
    check_truncation(&full, &cases)
}

/// What a fill costs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Costs {
    /// Per fill, on notional, in basis points.
    pub fee_bps: f64,
    /// Added against the trade's direction, in basis points of the open.
    pub half_spread_bps: f64,
}

/// What the evaluator is told besides the series and the bars.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvalConfig {
    pub initial_equity: f64,
    pub costs: Costs,
    /// Orders are rounded toward zero to a multiple of this. `None` is unrounded.
    pub lot_size: Option<f64>,
}

/// Square-root market impact (FEAT-005 §5): a fill of notional `Q` moves its price against
/// itself by `y * sigma * sqrt(Q / V)`, as a fraction of the open.
///
/// `sigma` and `volume_usd` must be in **one** period (per-bar volatility with per-bar dollar
/// volume, or daily with daily): the law is a statement about one horizon, and mixing two is
/// the usual way to get an impact that is off by `sqrt(bars per day)`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SqrtImpact {
    pub y: f64,
    pub sigma: f64,
    pub volume_usd: f64,
}

impl SqrtImpact {
    /// The price move as a fraction of the open for an order of `notional_usd`.
    #[must_use]
    pub fn rate(&self, notional_usd: f64) -> f64 {
        if self.volume_usd <= 0.0 {
            return 0.0;
        }
        self.y * self.sigma * (notional_usd / self.volume_usd).sqrt()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fill {
    /// The bar whose close the decision was made on.
    pub decided_ns: i64,
    /// The bar whose open it filled at.
    pub filled_bar_ns: i64,
    /// Signed: positive buys.
    pub qty: f64,
    pub price: f64,
    pub fee: f64,
    /// What the half-spread cost against the open.
    pub spread_cost: f64,
    /// What square-root impact cost against the open. Zero unless an impact was supplied.
    pub impact_cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PositionRun {
    pub fills: Vec<Fill>,
    /// Mark-to-close equity after every bar.
    pub equity: Vec<(i64, f64)>,
    pub final_equity: f64,
    pub total_fees: f64,
    pub total_spread_cost: f64,
    pub total_impact_cost: f64,
    /// A decision on the last bar has no next open to fill at.
    pub unfilled_last_decision: bool,
    /// The first bar at which equity was not positive; trading stops there.
    pub ruined_at_ns: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum EvalError {
    #[error("series: {0}")]
    Series(#[from] PositionSeriesError),
    #[error("there are no bars")]
    NoBars,
    #[error("bar {index} at {ts_ns} is not after the one before it")]
    BarsNotIncreasing { index: usize, ts_ns: i64 },
    #[error("bar at {ts_ns} has a non-positive or non-finite price")]
    BadPrice { ts_ns: i64 },
    #[error("initial equity must be positive and finite")]
    BadEquity,
    #[error("a cost or lot size is negative or non-finite")]
    BadCost,
}

/// Run a validated series over bars.
///
/// # Errors
/// An invalid series, unordered or non-positive bars, or an invalid config.
pub fn evaluate(
    series: &PositionSeries,
    bars: &[Bar],
    cfg: &EvalConfig,
) -> Result<PositionRun, EvalError> {
    evaluate_with_impact(series, bars, cfg, None)
}

/// [`evaluate`] with square-root impact charged on every fill (Set M M-2.1).
///
/// # Errors
/// As [`evaluate`], and a negative or non-finite impact parameter.
pub fn evaluate_with_impact(
    series: &PositionSeries,
    bars: &[Bar],
    cfg: &EvalConfig,
    impact: Option<SqrtImpact>,
) -> Result<PositionRun, EvalError> {
    if impact.is_some_and(|i| ![i.y, i.sigma, i.volume_usd].iter().all(|x| x.is_finite() && *x >= 0.0)) {
        return Err(EvalError::BadCost);
    }
    if bars.is_empty() {
        return Err(EvalError::NoBars);
    }
    if !(cfg.initial_equity.is_finite() && cfg.initial_equity > 0.0) {
        return Err(EvalError::BadEquity);
    }
    let bad_cost = |x: f64| !x.is_finite() || x < 0.0;
    if bad_cost(cfg.costs.fee_bps)
        || bad_cost(cfg.costs.half_spread_bps)
        || cfg.lot_size.is_some_and(|l| !(l.is_finite() && l > 0.0))
    {
        return Err(EvalError::BadCost);
    }
    for (index, b) in bars.iter().enumerate() {
        if !(b.open.is_finite() && b.open > 0.0 && b.close.is_finite() && b.close > 0.0) {
            return Err(EvalError::BadPrice { ts_ns: b.ts_ns });
        }
        if index > 0 && b.ts_ns <= bars[index - 1].ts_ns {
            return Err(EvalError::BarsNotIncreasing {
                index,
                ts_ns: b.ts_ns,
            });
        }
    }
    let times: Vec<i64> = bars.iter().map(|b| b.ts_ns).collect();
    series.validate(Some(&times))?;
    let targets = series.effective();

    let mut cash = cfg.initial_equity;
    let mut qty = 0.0_f64;
    let mut prev_target = 0.0_f64;
    let mut ti = 0usize;
    let mut pending: Option<(i64, f64)> = None; // (decided_ns, desired qty)
    let mut fills = Vec::new();
    let mut equity = Vec::with_capacity(bars.len());
    let mut total_fees = 0.0;
    let mut total_spread = 0.0;
    let mut total_impact = 0.0;
    let mut ruined_at_ns = None;
    let fee_rate = cfg.costs.fee_bps / 1e4;
    let spread_rate = cfg.costs.half_spread_bps / 1e4;

    for bar in bars {
        // 1. Fill what was decided on the previous bar, at this bar's open.
        if let Some((decided_ns, desired)) = pending.take() {
            let mut delta = desired - qty;
            if let Some(lot) = cfg.lot_size {
                delta = (delta / lot).trunc() * lot;
            }
            if delta != 0.0 {
                let side = delta.signum();
                let impact_rate = impact.map_or(0.0, |i| i.rate(delta.abs() * bar.open));
                let price = bar.open * (1.0 + side * (spread_rate + impact_rate));
                let notional = delta.abs() * price;
                let fee = notional * fee_rate;
                let spread_cost = delta.abs() * bar.open * spread_rate;
                let impact_cost = delta.abs() * bar.open * impact_rate;
                cash -= delta * price + fee;
                qty += delta;
                total_fees += fee;
                total_spread += spread_cost;
                total_impact += impact_cost;
                fills.push(Fill {
                    decided_ns,
                    filled_bar_ns: bar.ts_ns,
                    qty: delta,
                    price,
                    fee,
                    spread_cost,
                    impact_cost,
                });
            }
        }
        // 2. Mark to this close.
        let eq = cash + qty * bar.close;
        equity.push((bar.ts_ns, eq));
        if eq <= 0.0 && ruined_at_ns.is_none() {
            ruined_at_ns = Some(bar.ts_ns);
        }
        // 3. Decide on this close.
        while ti < targets.len() && targets[ti].0 < bar.ts_ns {
            ti += 1;
        }
        let target_now = (ti < targets.len() && targets[ti].0 == bar.ts_ns).then(|| targets[ti].1);
        if ruined_at_ns.is_some() {
            if qty != 0.0 {
                pending = Some((bar.ts_ns, 0.0));
            }
            continue;
        }
        if let Some(target) = target_now {
            #[allow(clippy::float_cmp)] // a changed target, exactly, is a trade
            let changed = target != prev_target;
            if changed {
                prev_target = target;
                let desired = match series.position_units {
                    PositionUnits::Fraction => target * eq / bar.close,
                    PositionUnits::Units => target,
                };
                pending = Some((bar.ts_ns, desired));
            }
        }
    }
    let final_equity = equity.last().map_or(cfg.initial_equity, |e| e.1);
    Ok(PositionRun {
        fills,
        equity,
        final_equity,
        total_fees,
        total_spread_cost: total_spread,
        total_impact_cost: total_impact,
        unfilled_last_decision: pending.is_some(),
        ruined_at_ns,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn pts(v: &[(i64, f64)]) -> Vec<PositionPoint> {
        v.iter().map(|(t, x)| PositionPoint::new(*t, *x)).collect()
    }

    fn series(v: &[(i64, f64)]) -> PositionSeries {
        PositionSeries {
            position_units: PositionUnits::Fraction,
            allow_short: false,
            points: pts(v),
        }
    }

    /// A deterministic random-walk-ish bar series.
    fn bars(n: usize) -> Vec<Bar> {
        let mut p = 100.0_f64;
        (0..n)
            .map(|i| {
                // A small gap, so a bar's open is never exactly the previous close and a
                // fill at the wrong price cannot hide behind an equal one.
                let open = p * (1.0 + 0.001 * ((i % 2) as f64 + 1.0));
                p = open * (1.0 + 0.01 * (((i * 7 + 3) % 11) as f64 - 5.0) / 5.0);
                Bar {
                    ts_ns: (i as i64 + 1) * 60_000_000_000,
                    open,
                    close: p,
                }
            })
            .collect()
    }

    // ── validity ──

    #[test]
    fn each_validity_rule_refuses_and_names_the_first_offender() {
        let times = [1, 2, 3, 4];
        assert_eq!(series(&[]).validate(None), Err(PositionSeriesError::Empty));
        assert_eq!(
            series(&[(2, 0.1), (2, 0.2)]).validate(None),
            Err(PositionSeriesError::NotIncreasing { index: 1, ts_ns: 2 })
        );
        assert_eq!(
            series(&[(3, 0.1), (2, 0.2)]).validate(None),
            Err(PositionSeriesError::NotIncreasing { index: 1, ts_ns: 2 })
        );
        assert_eq!(
            series(&[(1, 0.1), (9, 0.2)]).validate(Some(&times)),
            Err(PositionSeriesError::NotADecisionTime { ts_ns: 9 })
        );
        assert_eq!(
            series(&[(1, f64::INFINITY)]).validate(None),
            Err(PositionSeriesError::NonFinite { ts_ns: 1 })
        );
        assert!(matches!(
            series(&[(1, 0.2), (2, 1.5)]).validate(None),
            Err(PositionSeriesError::OutOfBounds { ts_ns: 2, .. })
        ));
        assert_eq!(
            series(&[(1, 0.2), (2, -0.5)]).validate(None),
            Err(PositionSeriesError::ShortNotAllowed {
                ts_ns: 2,
                value: -0.5
            })
        );
        let mut s = series(&[(1, 0.2), (2, -0.5)]);
        s.allow_short = true;
        assert_eq!(s.validate(Some(&times)), Ok(()));
    }

    #[test]
    fn nan_means_no_change_and_carries_the_previous_target() {
        let s = series(&[(1, 0.5), (2, f64::NAN), (3, 0.25), (4, f64::NAN)]);
        assert_eq!(s.points[1].target, None);
        assert_eq!(s.validate(None), Ok(()));
        let eff: Vec<f64> = s.effective().into_iter().map(|(_, v)| v).collect();
        assert_eq!(eff, vec![0.5, 0.5, 0.25, 0.25]);
        // Before the first explicit target the strategy is flat.
        let lead = series(&[(1, f64::NAN), (2, 0.4)]);
        assert_eq!(lead.effective()[0].1, 0.0);
    }

    #[test]
    fn a_pnl_field_is_an_error_not_an_ignored_value() {
        let with_pnl = r#"{"position_units":"fraction","points":[],"pnl":[1.0]}"#;
        assert!(serde_json::from_str::<PositionSeries>(with_pnl).is_err());
        let point_pnl = r#"{"ts_ns":1,"target":0.1,"pnl":3.0}"#;
        assert!(serde_json::from_str::<PositionPoint>(point_pnl).is_err());
        let ok = r#"{"position_units":"fraction","points":[{"ts_ns":1,"target":null}]}"#;
        let s: PositionSeries = serde_json::from_str(ok).unwrap();
        assert_eq!(s.points[0].target, None);
        assert!(!s.allow_short);
    }

    // ── truncation points ──

    #[test]
    fn cut_points_are_seeded_stratified_and_never_the_last_bar() {
        let times: Vec<i64> = (0..1000).collect();
        let a = truncation_points(7, 8, &times);
        assert_eq!(a, truncation_points(7, 8, &times), "same seed, same cuts");
        assert_ne!(
            a,
            truncation_points(8, 8, &times),
            "another seed, other cuts"
        );
        assert_eq!(a.len(), 8);
        // One cut in each eighth of the first 999 candidates.
        for (k, cut) in a.iter().enumerate() {
            let lo = (k * 999 / 8) as i64;
            let hi = ((k + 1) * 999 / 8) as i64;
            assert!(
                *cut >= lo && *cut < hi,
                "cut {k} = {cut} outside [{lo}, {hi})"
            );
        }
        assert!(a.iter().all(|c| *c != 999), "the final bar is never a cut");
        // Few candidates: every candidate, still excluding the last.
        assert_eq!(truncation_points(1, 8, &[10, 20, 30]), vec![10, 20]);
        assert!(truncation_points(1, 8, &[10]).is_empty());
        assert!(truncation_points(1, 0, &times).is_empty());
    }

    #[test]
    fn cuts_match_the_python_harness() {
        // Pinned from `apps/skill-runner/research_strategy.py::truncation_points(7, 8, range(1000))`
        // and held by `test_cut_points_match_the_rust_implementation`: one generator (SplitMix64)
        // and one slicing rule in both languages, so a re-run in either picks the same bars.
        let times: Vec<i64> = (0..1000).collect();
        assert_eq!(
            truncation_points(7, 8, &times),
            vec![5, 133, 368, 478, 515, 630, 802, 966]
        );
    }

    // ── truncation verdicts ──

    fn case(cut: i64, v: &[(i64, f64)]) -> TruncationCase {
        TruncationCase {
            cut_ns: cut,
            truncated: series(v),
        }
    }

    #[test]
    fn a_difference_by_one_ulp_before_the_cut_fails_and_names_the_timestamp() {
        let full = series(&[(1, 0.5), (2, 0.25), (3, 0.75), (4, 0.1)]);
        let bumped = f64::from_bits(0.25_f64.to_bits() + 1);
        let v = check_truncation(&full, &[case(3, &[(1, 0.5), (2, bumped), (3, 0.75)])]);
        match v {
            TruncationVerdict::Fail(f) => {
                assert_eq!(f.first_differing_ns, 2);
                assert_eq!(f.cut_ns, 3);
                assert_eq!(f.full, Some(0.25));
                assert_eq!(f.truncated, Some(bumped));
            }
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn nan_against_nan_passes_and_nan_against_a_number_fails() {
        let full = series(&[(1, 0.5), (2, f64::NAN), (3, 0.75)]);
        let same = check_truncation(&full, &[case(3, &[(1, 0.5), (2, f64::NAN), (3, 0.75)])]);
        assert_eq!(same, TruncationVerdict::Pass { cuts_checked: 1 });
        let diff = check_truncation(&full, &[case(3, &[(1, 0.5), (2, 0.5), (3, 0.75)])]);
        assert!(matches!(diff, TruncationVerdict::Fail(f) if f.first_differing_ns == 2));
    }

    #[test]
    fn a_difference_only_after_the_cut_is_not_a_failure() {
        let full = series(&[(1, 0.5), (2, 0.25), (3, 0.75)]);
        // The truncated run ends at 2 and, by construction, has no point at 3.
        let v = check_truncation(&full, &[case(2, &[(1, 0.5), (2, 0.25)])]);
        assert_eq!(v, TruncationVerdict::Pass { cuts_checked: 1 });
    }

    #[test]
    fn a_missing_or_extra_decision_time_before_the_cut_is_a_difference() {
        let full = series(&[(1, 0.5), (2, 0.25), (3, 0.75)]);
        let missing = check_truncation(&full, &[case(3, &[(1, 0.5), (3, 0.75)])]);
        assert!(matches!(missing, TruncationVerdict::Fail(f) if f.first_differing_ns == 2));
        let extra = check_truncation(&full, &[case(3, &[(1, 0.5), (2, 0.25), (3, 0.75)])]);
        assert!(extra.passed());
        let extra_pt = check_truncation(
            &series(&[(1, 0.5), (3, 0.75)]),
            &[case(3, &[(1, 0.5), (2, 0.25), (3, 0.75)])],
        );
        assert!(matches!(extra_pt, TruncationVerdict::Fail(f) if f.first_differing_ns == 2));
    }

    #[test]
    fn only_a_pass_raises_no_gate_flag() {
        let full = series(&[(1, 0.5), (2, 0.25), (3, 0.75)]);
        let pass = check_truncation(&full, &[case(3, &[(1, 0.5), (2, 0.25), (3, 0.75)])]);
        assert!(pass.gate_flag().is_none());

        let fail = check_truncation(&full, &[case(3, &[(1, 0.5), (2, 0.5), (3, 0.75)])]);
        let flag = fail.gate_flag().expect("a failure raises a flag");
        assert_eq!(flag.code, TRUNCATION_CODE);
        assert!(
            flag.detail.contains("1970-01-01T00:00:00.000000002"),
            "names the first differing timestamp: {}",
            flag.detail
        );

        let never = check_truncation(&full, &[]);
        let flag = never
            .gate_flag()
            .expect("an unrun test is not a passed one");
        assert_eq!(flag.code, TRUNCATION_NOT_RUN_CODE);
        assert_ne!(flag.code, TRUNCATION_CODE);
    }

    #[test]
    fn no_case_is_not_a_pass() {
        let v = check_truncation(&series(&[(1, 0.5)]), &[]);
        assert!(matches!(v, TruncationVerdict::NotComputable { .. }));
        assert!(!v.passed());
        assert!(v.flag_detail().is_none());
    }

    // ── truncation test against functions with a known defect ──

    /// Causal: long when the close is above the previous close.
    fn causal(bars: &[Bar]) -> PositionSeries {
        let mut out = vec![PositionPoint::new(bars[0].ts_ns, f64::NAN)];
        for w in bars.windows(2) {
            out.push(PositionPoint::new(
                w[1].ts_ns,
                if w[1].close > w[0].close { 1.0 } else { 0.0 },
            ));
        }
        PositionSeries {
            position_units: PositionUnits::Fraction,
            allow_short: false,
            points: out,
        }
    }

    /// Look-ahead: long when the NEXT close is higher — a `shift(-1)`.
    fn peeking(bars: &[Bar]) -> PositionSeries {
        let n = bars.len();
        let pts = (0..n)
            .map(|i| {
                // On a truncated run the last bar has no next close; the function
                // falls back to 0, which is exactly how a real shift(-1) behaves.
                let up = i + 1 < n && bars[i + 1].close > bars[i].close;
                PositionPoint::new(bars[i].ts_ns, if up { 1.0 } else { 0.0 })
            })
            .collect();
        PositionSeries {
            position_units: PositionUnits::Fraction,
            allow_short: false,
            points: pts,
        }
    }

    /// Full-sample normalisation: the target scales by the whole window's max.
    fn full_sample_scaled(bars: &[Bar]) -> PositionSeries {
        let max = bars.iter().map(|b| b.close).fold(f64::MIN, f64::max);
        let pts = bars
            .iter()
            .map(|b| PositionPoint::new(b.ts_ns, (b.close / max).clamp(0.0, 1.0)))
            .collect();
        PositionSeries {
            position_units: PositionUnits::Fraction,
            allow_short: false,
            points: pts,
        }
    }

    #[test]
    fn a_causal_function_passes_and_a_peeking_one_fails_naming_where() {
        let b = bars(400);
        assert_eq!(
            truncation_test(causal, &b, 11, 8),
            TruncationVerdict::Pass { cuts_checked: 8 }
        );
        match truncation_test(peeking, &b, 11, 8) {
            TruncationVerdict::Fail(f) => {
                // The shift(-1) differs at the bar just before each cut.
                assert!(f.first_differing_ns <= f.cut_ns);
                assert!(f.first_differing_ns >= f.cut_ns - 60_000_000_000);
            }
            other => panic!("a shift(-1) must fail, got {other:?}"),
        }
        assert!(
            matches!(
                truncation_test(full_sample_scaled, &b, 11, 8),
                TruncationVerdict::Fail(_)
            ),
            "a full-sample normalisation must fail"
        );
    }

    // ── the evaluator ──

    fn cfg(fee: f64, half: f64) -> EvalConfig {
        EvalConfig {
            initial_equity: 10_000.0,
            costs: Costs {
                fee_bps: fee,
                half_spread_bps: half,
            },
            lot_size: None,
        }
    }

    fn always_long(bars: &[Bar]) -> PositionSeries {
        series(
            &bars
                .iter()
                .enumerate()
                .map(|(i, b)| (b.ts_ns, if i == 1 { 1.0 } else { f64::NAN }))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn every_fill_is_at_a_bars_open_and_never_the_decision_bars_close() {
        let b = bars(50);
        let run = evaluate(&always_long(&b), &b, &cfg(0.0, 0.0)).unwrap();
        assert_eq!(run.fills.len(), 1);
        let f = &run.fills[0];
        assert_eq!(f.decided_ns, b[1].ts_ns);
        assert_eq!(f.filled_bar_ns, b[2].ts_ns, "the next bar");
        assert_eq!(f.price, b[2].open, "at its open");
        assert_ne!(f.price, b[1].close);
        // Sized from the decision bar's close and the equity then.
        let expected = 10_000.0 / b[1].close;
        assert!((f.qty - expected).abs() < 1e-9);
    }

    #[test]
    fn the_same_series_and_bars_give_byte_identical_results() {
        let b = bars(300);
        let s = causal(&b);
        let r1 = evaluate(&s, &b, &cfg(10.0, 2.0)).unwrap();
        let r2 = evaluate(&s, &b, &cfg(10.0, 2.0)).unwrap();
        assert_eq!(r1, r2);
        let bits =
            |r: &PositionRun| -> Vec<u64> { r.equity.iter().map(|e| e.1.to_bits()).collect() };
        assert_eq!(bits(&r1), bits(&r2));
    }

    #[test]
    fn doubling_the_fee_strictly_lowers_final_equity_on_a_series_that_trades() {
        let b = bars(300);
        let s = causal(&b);
        let a = evaluate(&s, &b, &cfg(10.0, 0.0)).unwrap();
        let c = evaluate(&s, &b, &cfg(20.0, 0.0)).unwrap();
        assert!(!a.fills.is_empty());
        assert!(c.final_equity < a.final_equity);
        assert!(c.total_fees > a.total_fees);
        let spread = evaluate(&s, &b, &cfg(10.0, 5.0)).unwrap();
        assert!(spread.final_equity < a.final_equity);
        assert!(spread.total_spread_cost > 0.0);
    }

    #[test]
    fn a_flat_series_pays_nothing_and_holds_its_equity() {
        let b = bars(40);
        let s = series(&b.iter().map(|x| (x.ts_ns, 0.0)).collect::<Vec<_>>());
        let run = evaluate(&s, &b, &cfg(50.0, 50.0)).unwrap();
        assert!(run.fills.is_empty());
        assert_eq!(run.final_equity, 10_000.0);
        assert_eq!(run.total_fees, 0.0);
    }

    #[test]
    fn a_decision_on_the_last_bar_has_no_open_to_fill_at() {
        let b = bars(5);
        let s = series(&[(b[4].ts_ns, 1.0)]);
        let run = evaluate(&s, &b, &cfg(0.0, 0.0)).unwrap();
        assert!(run.fills.is_empty());
        assert!(run.unfilled_last_decision);
    }

    #[test]
    fn a_target_that_does_not_change_does_not_trade() {
        let b = bars(30);
        let s = series(&b.iter().map(|x| (x.ts_ns, 0.5)).collect::<Vec<_>>());
        let run = evaluate(&s, &b, &cfg(10.0, 0.0)).unwrap();
        assert_eq!(run.fills.len(), 1, "one entry; drift is not a decision");
    }

    #[test]
    fn lots_round_toward_zero_and_a_sub_lot_order_is_skipped() {
        let b = bars(10);
        let mut c = cfg(0.0, 0.0);
        c.lot_size = Some(1.0);
        let s = series(&[(b[1].ts_ns, 1.0)]);
        let run = evaluate(&s, &b, &c).unwrap();
        assert_eq!(run.fills[0].qty.fract(), 0.0);
        c.lot_size = Some(1.0e9);
        let none = evaluate(&s, &b, &c).unwrap();
        assert!(none.fills.is_empty());
    }

    #[test]
    fn short_is_refused_unless_declared() {
        let b = bars(10);
        let s = series(&[(b[1].ts_ns, -0.5)]);
        assert!(matches!(
            evaluate(&s, &b, &cfg(0.0, 0.0)),
            Err(EvalError::Series(
                PositionSeriesError::ShortNotAllowed { .. }
            ))
        ));
        let mut s2 = s;
        s2.allow_short = true;
        let run = evaluate(&s2, &b, &cfg(0.0, 0.0)).unwrap();
        assert!(run.fills[0].qty < 0.0);
    }

    #[test]
    fn bad_inputs_are_errors_not_numbers() {
        let b = bars(5);
        let s = series(&[(b[0].ts_ns, 0.5)]);
        let mut bad = b.clone();
        bad[2].open = -1.0;
        assert!(matches!(
            evaluate(&s, &bad, &cfg(0.0, 0.0)),
            Err(EvalError::BadPrice { .. })
        ));
        let mut c = cfg(0.0, 0.0);
        c.initial_equity = 0.0;
        assert_eq!(evaluate(&s, &b, &c), Err(EvalError::BadEquity));
        assert_eq!(evaluate(&s, &[], &cfg(0.0, 0.0)), Err(EvalError::NoBars));
        assert!(matches!(
            evaluate(&s, &b, &cfg(-1.0, 0.0)),
            Err(EvalError::BadCost)
        ));
        let mut unordered = b.clone();
        unordered.swap(1, 2);
        assert!(matches!(
            evaluate(&s, &unordered, &cfg(0.0, 0.0)),
            Err(EvalError::BarsNotIncreasing { .. })
        ));
    }

    #[test]
    fn a_buy_and_hold_with_no_costs_earns_the_price_return_on_the_invested_equity() {
        let b = bars(60);
        let s = series(&[(b[0].ts_ns, 1.0)]);
        let run = evaluate(&s, &b, &cfg(0.0, 0.0)).unwrap();
        // Entered at bar 1's open, held to the last close.
        let qty = 10_000.0 / b[0].close;
        let expect = 10_000.0 + qty * (b[59].close - b[1].open);
        assert!(
            (run.final_equity - expect).abs() < 1e-6,
            "{} vs {expect}",
            run.final_equity
        );
    }
}
