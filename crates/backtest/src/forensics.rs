//! Evidence-gathering for the falsification tools (ADR-P7-05).
//!
//! ## The hole this closes
//!
//! The external-review sprint gave the agent `report_kill` with a closed
//! vocabulary of defect classes — `data_artifact`, `lookahead`,
//! `below_cost_floor` — and then gave it **no way to look at a trade list or a
//! bar**. It could name a defect and could not gather the evidence the report
//! requires, which made the whole mechanism decorative for four of its seven
//! classes.
//!
//! Everything here answers one of those classes with a number:
//!
//! | Class | Answered by |
//! |---|---|
//! | `data_artifact` | [`concentration`] + [`suspicious_bars`] |
//! | `lookahead` | [`shift_sensitivity`] |
//! | `below_cost_floor` | [`cost_floor`] |
//! | `invalid_metric` | [`metric_validity`] |
//!
//! ## Why these are measurements and not verdicts
//!
//! Each function returns what it found, never "this is a kill". A concentration
//! of 94% in six bars is *a fact*; whether it is a data artifact depends on
//! whether those bars are real, which is why [`concentration`] and
//! [`suspicious_bars`] are separate and a reporter is expected to join them.
//! Collapsing that into one "is_artifact" boolean would let the agent file a
//! kill it had not actually checked.

use serde::{Deserialize, Serialize};

/// One realised round trip, reduced to what a forensic check needs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TradeSlice {
    pub entry_index: usize,
    pub exit_index: usize,
    pub pnl: f64,
}

// ── P&L concentration ────────────────────────────────────────────────────────

/// How much of the result rests on how little of the sample.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Concentration {
    /// Trades ordered by contribution, largest first: `(index_into_trades, pnl)`.
    pub top: Vec<(usize, f64)>,
    /// Fraction of total **gross gain** the top `n` trades account for.
    pub top_n_share: f64,
    /// How many trades it takes to reach `share_target` of the gross gain.
    pub trades_for_target: usize,
    pub total_trades: usize,
    pub net_pnl: f64,
    /// Sum of winning trades. The denominator, deliberately: dividing by *net*
    /// P&L lets a strategy whose net is near zero report a concentration of
    /// several thousand percent, which is arithmetic rather than evidence.
    pub gross_gain: f64,
}

impl Concentration {
    /// Whether the result would survive deleting the top `n` trades.
    ///
    /// The question that matters. A strategy whose edge disappears without its
    /// six best bars did not have an edge; it had six bars.
    #[must_use]
    pub fn net_without_top(&self, n: usize) -> f64 {
        self.net_pnl - self.top.iter().take(n).map(|(_, p)| *p).sum::<f64>()
    }
}

/// Measure how concentrated a trade list's P&L is.
///
/// `share_target` is the fraction of gross gain to count trades toward — 0.9 is
/// the useful default. Returns `None` for an empty list or one with no winners,
/// because "how concentrated are the gains" has no answer when there are none.
#[must_use]
pub fn concentration(
    trades: &[TradeSlice],
    share_target: f64,
    top_n: usize,
) -> Option<Concentration> {
    if trades.is_empty() {
        return None;
    }
    let gross_gain: f64 = trades.iter().map(|t| t.pnl.max(0.0)).sum();
    if gross_gain <= 0.0 {
        return None;
    }

    let mut ranked: Vec<(usize, f64)> =
        trades.iter().enumerate().map(|(i, t)| (i, t.pnl)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut running = 0.0;
    let mut trades_for_target = 0usize;
    for (_, pnl) in &ranked {
        if running / gross_gain >= share_target {
            break;
        }
        if *pnl > 0.0 {
            running += pnl;
            trades_for_target += 1;
        } else {
            break;
        }
    }

    let top: Vec<(usize, f64)> = ranked.into_iter().take(top_n.max(1)).collect();
    let top_n_share = top.iter().map(|(_, p)| p.max(0.0)).sum::<f64>() / gross_gain;

    Some(Concentration {
        top,
        top_n_share,
        trades_for_target,
        total_trades: trades.len(),
        net_pnl: trades.iter().map(|t| t.pnl).sum(),
        gross_gain,
    })
}

// ── suspicious bars ──────────────────────────────────────────────────────────

/// One bar, as a data-quality check sees it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarSlice {
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// What is wrong with a bar.
///
/// A closed vocabulary, like the kill classes: each names something checkable
/// rather than something that merely looks odd.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarDefect {
    /// `high < low`, or a price outside its own range. The bar is not a bar.
    Impossible,
    /// A wick well beyond the body **on no volume**. The classic bad print: a
    /// price nothing traded at, which a backtest will happily fill against.
    WickWithoutVolume,
    /// Identical OHLC to the previous bar with volume. A stale feed republished.
    StaleRepeat,
    /// A move many times the local volatility that fully reverses next bar.
    SpikeAndRevert,
    /// Zero volume in a session that should have some.
    NoVolume,
    /// A non-positive price.
    NonPositivePrice,
}

/// A bar flagged by a check.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suspicion {
    pub index: usize,
    pub defect: BarDefect,
    /// How extreme, in units natural to the defect — wick multiples, sigma, and
    /// so on. Kept so a reporter can say *how* bad rather than only *that*.
    pub magnitude: f64,
}

/// Scan bars for defects.
///
/// `wick_multiple` is how far past the body a wick must reach before a
/// zero-volume bar is flagged (3.0 is sensible); `spike_sigma` how many local
/// standard deviations a reverting move must be (8.0 is sensible — at that size
/// it is a print, not a market).
#[must_use]
pub fn suspicious_bars(bars: &[BarSlice], wick_multiple: f64, spike_sigma: f64) -> Vec<Suspicion> {
    let mut out = Vec::new();

    for (i, b) in bars.iter().enumerate() {
        if b.open <= 0.0 || b.high <= 0.0 || b.low <= 0.0 || b.close <= 0.0 {
            out.push(Suspicion {
                index: i,
                defect: BarDefect::NonPositivePrice,
                magnitude: 0.0,
            });
            continue;
        }
        if b.high < b.low
            || b.open > b.high
            || b.open < b.low
            || b.close > b.high
            || b.close < b.low
        {
            out.push(Suspicion {
                index: i,
                defect: BarDefect::Impossible,
                magnitude: 0.0,
            });
            continue;
        }

        let body = (b.close - b.open).abs();
        let upper = b.high - b.open.max(b.close);
        let lower = b.open.min(b.close) - b.low;
        let wick = upper.max(lower);

        // The classic bad print: a price nothing traded at.
        if b.volume <= 0.0 && body > 0.0 && wick > body * wick_multiple {
            out.push(Suspicion {
                index: i,
                defect: BarDefect::WickWithoutVolume,
                magnitude: wick / body,
            });
        } else if b.volume <= 0.0 {
            out.push(Suspicion {
                index: i,
                defect: BarDefect::NoVolume,
                magnitude: 0.0,
            });
        }

        if i > 0 {
            let p = &bars[i - 1];
            if b.volume > 0.0
                && (b.open - p.open).abs() < f64::EPSILON
                && (b.high - p.high).abs() < f64::EPSILON
                && (b.low - p.low).abs() < f64::EPSILON
                && (b.close - p.close).abs() < f64::EPSILON
            {
                out.push(Suspicion {
                    index: i,
                    defect: BarDefect::StaleRepeat,
                    magnitude: 0.0,
                });
            }
        }
    }

    // Spike-and-revert needs a local volatility estimate, so it is a second
    // pass over a window rather than a per-bar test.
    const WINDOW: usize = 30;
    if bars.len() > WINDOW + 2 {
        let returns: Vec<f64> = bars
            .windows(2)
            .map(|w| {
                if w[0].close > 0.0 {
                    (w[1].close - w[0].close) / w[0].close
                } else {
                    0.0
                }
            })
            .collect();

        for i in WINDOW..returns.len().saturating_sub(1) {
            let window = &returns[i - WINDOW..i];
            let mean = window.iter().sum::<f64>() / window.len() as f64;
            let var = window.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / window.len() as f64;
            let sd = var.sqrt();
            if sd <= 0.0 {
                continue;
            }
            let (here, next) = (returns[i], returns[i + 1]);
            // Large, and undone immediately: a print rather than a move.
            if here.abs() > spike_sigma * sd
                && here.signum() != next.signum()
                && (here + next).abs() < here.abs() * 0.25
            {
                out.push(Suspicion {
                    index: i + 1,
                    defect: BarDefect::SpikeAndRevert,
                    magnitude: here.abs() / sd,
                });
            }
        }
    }

    out
}

// ── look-ahead ───────────────────────────────────────────────────────────────

/// What happened when the signal was shifted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShiftSensitivity {
    pub baseline: f64,
    /// `(shift_in_bars, metric)`. Negative shifts look *forward*, which is the
    /// direction that exposes leakage.
    pub shifted: Vec<(i32, f64)>,
    /// Whether the metric inverts when the signal is delayed by one bar.
    pub inverts_on_delay: bool,
    /// Whether looking one bar *forward* improves it.
    pub improves_on_lookahead: bool,
}

impl ShiftSensitivity {
    /// The reading a reporter can act on.
    ///
    /// Inversion on a one-bar delay is the canonical look-ahead signature: a
    /// strategy trading on the bar it computes from loses exactly what it made
    /// once it can no longer see that bar.
    #[must_use]
    pub fn verdict(&self) -> &'static str {
        match (self.inverts_on_delay, self.improves_on_lookahead) {
            (true, _) => {
                "inverts when delayed one bar — the canonical look-ahead signature. The strategy \
                 is almost certainly trading on the bar it computes from."
            }
            (false, true) => {
                "improves when allowed to look one bar forward, which is expected and is not \
                 evidence of leakage on its own. The delay test is the one that matters."
            }
            (false, false) => {
                "degrades gracefully under shifting, which is what a real edge does. This is not \
                 proof of absence: a leak through a feature rather than through timing would not \
                 show here."
            }
        }
    }
}

/// Measure a metric's sensitivity to shifting the signal.
///
/// `evaluate(shift)` re-runs whatever produced the metric with the signal moved
/// by `shift` bars. The caller owns that; this owns the interpretation.
pub fn shift_sensitivity(mut evaluate: impl FnMut(i32) -> f64, shifts: &[i32]) -> ShiftSensitivity {
    let baseline = evaluate(0);
    let shifted: Vec<(i32, f64)> = shifts
        .iter()
        .filter(|s| **s != 0)
        .map(|s| (*s, evaluate(*s)))
        .collect();

    let delayed = shifted.iter().find(|(s, _)| *s == 1).map(|(_, m)| *m);
    let ahead = shifted.iter().find(|(s, _)| *s == -1).map(|(_, m)| *m);

    ShiftSensitivity {
        baseline,
        inverts_on_delay: delayed.is_some_and(|m| baseline > 0.0 && m < 0.0),
        improves_on_lookahead: ahead.is_some_and(|m| m > baseline),
        shifted,
    }
}

// ── the cost floor ───────────────────────────────────────────────────────────

/// Where an edge sits relative to the cost of taking it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostFloor {
    /// Mean gross P&L per trade, in bps of notional.
    pub gross_bps_per_trade: f64,
    /// The round trip actually charged.
    pub round_trip_bps: f64,
    /// Gross over cost. Below 1 the edge never existed.
    pub ratio: f64,
    pub trades: usize,
}

impl CostFloor {
    /// Whether this clears the floor at all.
    #[must_use]
    pub fn clears(&self) -> bool {
        self.ratio > 1.0
    }
}

/// Compare a strategy's gross edge to its round trip.
///
/// Uses **gross** rather than net on purpose: the question is whether the raw
/// signal is bigger than the cost, and netting costs out first answers a
/// different question with the same words.
#[must_use]
pub fn cost_floor(gross_pnl_bps: &[f64], round_trip_bps: f64) -> Option<CostFloor> {
    if gross_pnl_bps.is_empty() || round_trip_bps <= 0.0 {
        return None;
    }
    let gross = gross_pnl_bps.iter().sum::<f64>() / gross_pnl_bps.len() as f64;
    Some(CostFloor {
        gross_bps_per_trade: gross,
        round_trip_bps,
        ratio: gross / round_trip_bps,
        trades: gross_pnl_bps.len(),
    })
}

// ── metric validity ──────────────────────────────────────────────────────────

/// Why a metric should not be believed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricProblem {
    /// Too few observations for the statistic to mean anything.
    TooFewObservations { n: usize, minimum: usize },
    /// Every return identical — the denominator is zero and the ratio is
    /// infinite or undefined.
    ZeroVariance,
    /// Non-finite values in the series.
    NonFinite { count: usize },
    /// Almost every bar is flat, so the "observations" are mostly the strategy
    /// not trading. A Sharpe over three hundred bars of which four are trades is
    /// a Sharpe over four numbers.
    MostlyFlat { non_zero: usize, total: usize },
}

/// Check whether a return series can support a ratio statistic.
///
/// Returns every problem found rather than the first: a series can be both too
/// short and mostly flat, and fixing one would not fix the other.
#[must_use]
pub fn metric_validity(returns: &[f64], minimum_observations: usize) -> Vec<MetricProblem> {
    let mut problems = Vec::new();

    let non_finite = returns.iter().filter(|r| !r.is_finite()).count();
    if non_finite > 0 {
        problems.push(MetricProblem::NonFinite { count: non_finite });
    }

    let finite: Vec<f64> = returns.iter().copied().filter(|r| r.is_finite()).collect();
    if finite.len() < minimum_observations {
        problems.push(MetricProblem::TooFewObservations {
            n: finite.len(),
            minimum: minimum_observations,
        });
    }

    let non_zero = finite.iter().filter(|r| r.abs() > f64::EPSILON).count();
    if !finite.is_empty() && non_zero * 10 < finite.len() {
        problems.push(MetricProblem::MostlyFlat {
            non_zero,
            total: finite.len(),
        });
    }

    // Identity rather than a computed variance. Summing a hundred copies of
    // 0.01 does not give exactly 1.0, so the mean carries a rounding error and
    // the variance of a genuinely constant series comes out as a tiny positive
    // number — which is precisely the case this check exists to catch.
    if finite.len() > 1 && finite.iter().all(|r| (r - finite[0]).abs() <= f64::EPSILON) {
        problems.push(MetricProblem::ZeroVariance);
    }

    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(pnl: f64) -> TradeSlice {
        TradeSlice {
            entry_index: 0,
            exit_index: 1,
            pnl,
        }
    }

    fn bar(open: f64, high: f64, low: f64, close: f64, volume: f64) -> BarSlice {
        BarSlice {
            open,
            high,
            low,
            close,
            volume,
        }
    }

    // ── concentration ────────────────────────────────────────────────────────

    /// The headline case: a strategy whose edge is six bars.
    #[test]
    fn a_result_resting_on_a_handful_of_trades_is_visible() {
        let mut trades: Vec<TradeSlice> = (0..194).map(|_| trade(-1.0)).collect();
        for _ in 0..6 {
            trades.push(trade(200.0));
        }
        let c = concentration(&trades, 0.9, 6).unwrap();

        assert_eq!(c.total_trades, 200);
        assert!(
            (c.top_n_share - 1.0).abs() < 1e-9,
            "all the gain is in six trades"
        );
        assert_eq!(c.trades_for_target, 6);
        assert!(c.net_pnl > 0.0);
        assert!(
            c.net_without_top(6) < 0.0,
            "without them the strategy loses money"
        );
    }

    /// Dividing by *net* P&L lets a strategy whose net is near zero report a
    /// concentration of several thousand percent, which is arithmetic rather
    /// than evidence.
    #[test]
    fn concentration_is_measured_against_gross_gain_not_net() {
        let trades = vec![trade(100.0), trade(-99.0), trade(1.0)];
        let c = concentration(&trades, 0.9, 1).unwrap();
        assert!((c.gross_gain - 101.0).abs() < 1e-9);
        assert!(
            c.top_n_share <= 1.0,
            "a share above 1 would be an artefact of the denominator"
        );
    }

    #[test]
    fn a_diffuse_result_needs_many_trades_to_reach_the_target() {
        let trades: Vec<TradeSlice> = (0..100).map(|_| trade(1.0)).collect();
        let c = concentration(&trades, 0.9, 6).unwrap();
        assert_eq!(c.trades_for_target, 90);
        assert!((c.top_n_share - 0.06).abs() < 1e-9);
    }

    #[test]
    fn a_list_with_no_winners_has_no_concentration_to_report() {
        assert!(concentration(&[], 0.9, 6).is_none());
        assert!(concentration(&[trade(-1.0), trade(-2.0)], 0.9, 6).is_none());
    }

    // ── suspicious bars ──────────────────────────────────────────────────────

    /// The classic bad print: a price nothing traded at, which a backtest will
    /// happily fill against.
    #[test]
    fn a_long_wick_on_no_volume_is_flagged() {
        let bars = vec![
            bar(100.0, 100.5, 99.5, 100.1, 1000.0),
            bar(100.0, 140.0, 99.9, 100.1, 0.0),
            bar(100.1, 100.6, 99.8, 100.2, 1000.0),
        ];
        let found = suspicious_bars(&bars, 3.0, 8.0);
        let wick = found
            .iter()
            .find(|s| s.defect == BarDefect::WickWithoutVolume);
        assert!(wick.is_some(), "{found:?}");
        assert_eq!(wick.unwrap().index, 1);
        assert!(
            wick.unwrap().magnitude > 100.0,
            "the wick is enormous relative to the body"
        );
    }

    #[test]
    fn an_impossible_bar_is_flagged_before_anything_else_is_measured() {
        let bars = vec![bar(100.0, 99.0, 101.0, 100.0, 5.0)];
        let found = suspicious_bars(&bars, 3.0, 8.0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].defect, BarDefect::Impossible);
    }

    #[test]
    fn a_close_outside_the_bars_range_is_impossible() {
        let bars = vec![bar(100.0, 101.0, 99.0, 105.0, 5.0)];
        assert_eq!(
            suspicious_bars(&bars, 3.0, 8.0)[0].defect,
            BarDefect::Impossible
        );
    }

    #[test]
    fn a_republished_bar_is_flagged_as_stale() {
        let b = bar(100.0, 101.0, 99.0, 100.5, 500.0);
        let found = suspicious_bars(&[b, b], 3.0, 8.0);
        assert!(
            found
                .iter()
                .any(|s| s.defect == BarDefect::StaleRepeat && s.index == 1),
            "{found:?}"
        );
    }

    #[test]
    fn a_zero_volume_bar_with_no_wick_is_flagged_as_no_volume_not_as_a_wick() {
        let bars = vec![bar(100.0, 100.2, 99.8, 100.1, 0.0)];
        let found = suspicious_bars(&bars, 3.0, 8.0);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].defect, BarDefect::NoVolume);
    }

    /// A move many sigma wide that is fully undone next bar is a print, not a
    /// market.
    #[test]
    fn a_spike_that_fully_reverts_is_flagged() {
        let mut bars: Vec<BarSlice> = (0..60)
            .map(|i| {
                let p = 100.0 + f64::from(i % 3) * 0.01;
                bar(p, p + 0.02, p - 0.02, p, 1000.0)
            })
            .collect();
        // A 10% jump and an immediate return.
        bars[40] = bar(110.0, 110.1, 109.9, 110.0, 1000.0);
        let found = suspicious_bars(&bars, 3.0, 8.0);
        assert!(
            found.iter().any(|s| s.defect == BarDefect::SpikeAndRevert),
            "a 10% move that fully reverts should be flagged: {found:?}"
        );
    }

    #[test]
    fn clean_bars_are_not_flagged() {
        let bars: Vec<BarSlice> = (0..100)
            .map(|i| {
                let p = 100.0 + f64::from(i) * 0.1;
                bar(p, p + 0.3, p - 0.3, p + 0.1, 1000.0 + f64::from(i))
            })
            .collect();
        assert!(suspicious_bars(&bars, 3.0, 8.0).is_empty());
    }

    // ── look-ahead ───────────────────────────────────────────────────────────

    /// The canonical signature: a strategy trading on the bar it computes from
    /// loses exactly what it made once it can no longer see that bar.
    #[test]
    fn a_metric_that_inverts_on_a_one_bar_delay_reads_as_lookahead() {
        let s = shift_sensitivity(
            |shift| match shift {
                0 => 2.9,
                1 => -2.7,
                -1 => 4.0,
                _ => 0.0,
            },
            &[-1, 1, 2],
        );
        assert!(s.inverts_on_delay);
        assert!(s.verdict().contains("canonical look-ahead signature"));
    }

    /// Improving when allowed to see the future is expected and is not, on its
    /// own, evidence of anything.
    #[test]
    fn improving_on_lookahead_alone_is_not_reported_as_leakage() {
        let s = shift_sensitivity(
            |shift| match shift {
                0 => 1.0,
                -1 => 3.0,
                1 => 0.6,
                _ => 0.0,
            },
            &[-1, 1],
        );
        assert!(!s.inverts_on_delay);
        assert!(s.improves_on_lookahead);
        assert!(s.verdict().contains("not evidence of leakage on its own"));
    }

    /// The verdict must not overclaim: a leak through a feature rather than
    /// through timing would not show in a shift test at all.
    #[test]
    fn a_clean_shift_profile_says_what_it_does_not_prove() {
        let s = shift_sensitivity(
            |shift| match shift {
                0 => 1.2,
                1 => 0.9,
                -1 => 1.1,
                _ => 0.5,
            },
            &[-1, 1, 5],
        );
        assert!(!s.inverts_on_delay);
        assert!(s.verdict().contains("not proof of absence"));
    }

    #[test]
    fn the_baseline_is_measured_once_and_zero_shift_is_not_repeated() {
        let mut calls = Vec::new();
        let s = shift_sensitivity(
            |shift| {
                calls.push(shift);
                1.0
            },
            &[0, 1, -1],
        );
        assert_eq!(
            calls,
            vec![0, 1, -1],
            "the explicit 0 in the list is skipped"
        );
        assert_eq!(s.shifted.len(), 2);
    }

    // ── the cost floor ───────────────────────────────────────────────────────

    /// The review's example: a break-even cost of 1.8 bp against a 20 bp floor.
    #[test]
    fn an_edge_smaller_than_the_round_trip_does_not_clear_the_floor() {
        let f = cost_floor(&[1.8; 500], 20.0).unwrap();
        assert!(!f.clears());
        assert!((f.ratio - 0.09).abs() < 1e-9);
    }

    #[test]
    fn an_edge_several_times_the_round_trip_clears_it() {
        let f = cost_floor(&[60.0; 100], 20.0).unwrap();
        assert!(f.clears());
        assert!((f.ratio - 3.0).abs() < 1e-9);
    }

    /// Netting costs out first answers a different question with the same words.
    #[test]
    fn the_floor_is_measured_on_gross_and_a_free_venue_has_no_floor() {
        assert!(cost_floor(&[1.0], 0.0).is_none());
        assert!(cost_floor(&[], 20.0).is_none());
    }

    // ── metric validity ──────────────────────────────────────────────────────

    /// A Sharpe over three hundred bars of which four are trades is a Sharpe
    /// over four numbers.
    #[test]
    fn a_mostly_flat_series_is_flagged_however_long_it_is() {
        let mut returns = vec![0.0; 300];
        returns[10] = 0.01;
        returns[20] = -0.02;
        returns[30] = 0.03;
        returns[40] = 0.01;

        let problems = metric_validity(&returns, 30);
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, MetricProblem::MostlyFlat { non_zero: 4, .. })),
            "{problems:?}"
        );
    }

    /// A series can be both too short and mostly flat, and fixing one would not
    /// fix the other.
    #[test]
    fn every_problem_is_reported_not_only_the_first() {
        // Long enough that "mostly flat" genuinely applies at the 10% threshold,
        // short of the minimum asked for, and carrying a NaN.
        let mut returns = vec![0.0; 50];
        returns[7] = 0.01;
        returns[19] = -0.02;
        returns.push(f64::NAN);
        let problems = metric_validity(&returns, 100);
        assert!(problems.len() >= 3, "{problems:?}");
        assert!(problems
            .iter()
            .any(|p| matches!(p, MetricProblem::NonFinite { .. })));
        assert!(problems
            .iter()
            .any(|p| matches!(p, MetricProblem::TooFewObservations { .. })));
        assert!(problems
            .iter()
            .any(|p| matches!(p, MetricProblem::MostlyFlat { .. })));
    }

    #[test]
    fn a_constant_series_has_zero_variance() {
        let problems = metric_validity(&[0.01; 100], 30);
        assert!(problems.contains(&MetricProblem::ZeroVariance));
    }

    #[test]
    fn a_healthy_series_has_no_problems() {
        let returns: Vec<f64> = (0..300)
            .map(|i| ((f64::from(i) * 0.7).sin()) * 0.01)
            .collect();
        assert!(metric_validity(&returns, 30).is_empty());
    }
}
