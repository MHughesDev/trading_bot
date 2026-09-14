//! [`DiagnosticBundle`] — what a research agent reads in order to decide *what
//! to change* (FEAT-003 §7 / §11.1).
//!
//! A summary Sharpe says nothing about the edit to make. Human quants change
//! strategies in response to trade-level P&L shape, when it loses, how far
//! trades go against them, and how long the worst stretch lasted. This module
//! computes that from a [`RunResult`] alone — no simulator access, no LLM —
//! and renders a compact text summary (≤ 1.5 KB) for the model's context.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use chrono::{DateTime, Datelike, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::run::{MetricSet, RunResult, RunStatus, Side, Trade};

/// Text budget for the LLM-facing summary.
const MAX_TEXT_BYTES: usize = 1_500;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TradeStats {
    pub n: usize,
    pub wins: usize,
    pub losses: usize,
    pub win_rate: f64,
    /// Mean P&L of the winning trades, in quote currency.
    pub avg_win: Decimal,
    /// Mean loss magnitude of the losing trades (≥ 0), in quote currency.
    pub avg_loss: Decimal,
    /// R-multiple, unitless.
    pub expectancy: f64,
    pub profit_factor: f64,
    pub pnl_total: Decimal,
    pub costs_total: Decimal,
    pub hold_secs_p50: i64,
    pub mae_p50: f64,
    pub mfe_p50: f64,
    pub longest_losing_streak: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonthSlice {
    /// `YYYY-MM`.
    pub ym: String,
    /// Equity return over the month.
    pub ret: f64,
    /// Worst intra-month drawdown from the month's running peak (≤ 0).
    pub max_dd: f64,
    /// Trades closed in the month.
    pub n_trades: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeSlice {
    pub label: String,
    pub ret: f64,
    pub n_trades: usize,
    pub share_of_time: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorstTrade {
    pub entry_time: DateTime<Utc>,
    pub exit_time: DateTime<Utc>,
    pub side: Side,
    pub pnl: Decimal,
    pub holding_period_secs: i64,
    pub mae: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawdownEpisode {
    pub start: DateTime<Utc>,
    pub trough: DateTime<Utc>,
    /// `None` if the run ended still under water.
    pub recovered: Option<DateTime<Utc>>,
    /// Depth as a fraction (≤ 0).
    pub depth: f64,
    pub duration_secs: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Exposure {
    /// Fraction of the run's span with an open position (holding time / span;
    /// overlapping trades are not double-counted beyond 1.0).
    pub time_in_market: f64,
    /// Notional traded (from the metric set).
    pub turnover: f64,
    /// Costs as a fraction of gross P&L magnitude.
    pub cost_drag: f64,
}

/// Everything the agent needs to reason about a Run, in one response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticBundle {
    pub run_id: String,
    pub status: RunStatus,
    pub metrics: MetricSet,
    pub trades: TradeStats,
    pub by_month: Vec<MonthSlice>,
    /// Empty until a regime engine supplies labelled windows.
    pub by_regime: Vec<RegimeSlice>,
    /// The ten worst trades by P&L.
    pub worst_trades: Vec<WorstTrade>,
    pub longest_drawdown: Option<DrawdownEpisode>,
    pub exposure: Exposure,
    /// Compact, LLM-facing rendering of the above (≤ 1.5 KB).
    pub text: String,
}

fn f(d: rust_decimal::Decimal) -> f64 {
    d.to_f64().unwrap_or(0.0)
}

fn median<T: Copy + PartialOrd>(v: &mut [T]) -> Option<T> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(v[v.len() / 2])
}

fn trade_stats(trades: &[Trade], metrics: &MetricSet) -> TradeStats {
    if trades.is_empty() {
        return TradeStats::default();
    }
    let (mut wins, mut losses) = (0usize, 0usize);
    let (mut gain, mut loss, mut pnl_total, mut costs) = (
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
    );
    let (mut streak, mut longest) = (0usize, 0usize);
    for t in trades {
        let p = t.pnl;
        pnl_total += p;
        costs += t.costs_paid;
        if p > Decimal::ZERO {
            wins += 1;
            gain += p;
            streak = 0;
        } else {
            if p < Decimal::ZERO {
                losses += 1;
                loss -= p;
            }
            streak += 1;
            longest = longest.max(streak);
        }
    }
    let mut holds: Vec<i64> = trades.iter().map(|t| t.holding_period_secs).collect();
    let mut adverse: Vec<f64> = trades.iter().map(|t| t.mae).collect();
    let mut favorable: Vec<f64> = trades.iter().map(|t| t.mfe).collect();
    TradeStats {
        n: trades.len(),
        wins,
        losses,
        win_rate: wins as f64 / trades.len() as f64,
        avg_win: if wins > 0 {
            gain / Decimal::from(wins)
        } else {
            Decimal::ZERO
        },
        avg_loss: if losses > 0 {
            loss / Decimal::from(losses)
        } else {
            Decimal::ZERO
        },
        expectancy: metrics.expectancy,
        profit_factor: metrics.profit_factor,
        pnl_total,
        costs_total: costs,
        hold_secs_p50: median(&mut holds).unwrap_or(0),
        mae_p50: median(&mut adverse).unwrap_or(0.0),
        mfe_p50: median(&mut favorable).unwrap_or(0.0),
        longest_losing_streak: longest,
    }
}

fn ym(ts: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", ts.year(), ts.month())
}

fn by_month(equity: &[(DateTime<Utc>, f64)], trades: &[Trade]) -> Vec<MonthSlice> {
    if equity.is_empty() {
        return Vec::new();
    }
    let mut trades_per_month: BTreeMap<String, usize> = BTreeMap::new();
    for t in trades {
        *trades_per_month.entry(ym(t.exit_time)).or_default() += 1;
    }
    // Month-end equity and intra-month drawdown.
    let mut out: Vec<MonthSlice> = Vec::new();
    let mut prev_close = equity[0].1;
    let mut cur = ym(equity[0].0);
    let (mut peak, mut dd) = (equity[0].1, 0.0f64);
    let mut last = equity[0].1;
    for &(ts, eq) in equity {
        let m = ym(ts);
        if m != cur {
            out.push(MonthSlice {
                ym: cur.clone(),
                ret: if prev_close > 0.0 {
                    last / prev_close - 1.0
                } else {
                    0.0
                },
                max_dd: dd,
                n_trades: trades_per_month.get(&cur).copied().unwrap_or(0),
            });
            prev_close = last;
            cur = m;
            peak = eq;
            dd = 0.0;
        }
        peak = peak.max(eq);
        if peak > 0.0 {
            dd = dd.min(eq / peak - 1.0);
        }
        last = eq;
    }
    out.push(MonthSlice {
        ym: cur.clone(),
        ret: if prev_close > 0.0 {
            last / prev_close - 1.0
        } else {
            0.0
        },
        max_dd: dd,
        n_trades: trades_per_month.get(&cur).copied().unwrap_or(0),
    });
    out
}

fn longest_drawdown(equity: &[(DateTime<Utc>, f64)]) -> Option<DrawdownEpisode> {
    if equity.len() < 2 {
        return None;
    }
    let mut best: Option<DrawdownEpisode> = None;
    let mut peak = equity[0];
    let mut episode: Option<(DateTime<Utc>, DateTime<Utc>, f64)> = None; // start, trough, depth
    for &(ts, eq) in equity {
        if eq >= peak.1 {
            if let Some((start, trough, depth)) = episode.take() {
                let ep = DrawdownEpisode {
                    start,
                    trough,
                    recovered: Some(ts),
                    depth,
                    duration_secs: (ts - start).num_seconds(),
                };
                if best
                    .as_ref()
                    .is_none_or(|b| ep.duration_secs > b.duration_secs)
                {
                    best = Some(ep);
                }
            }
            peak = (ts, eq);
        } else {
            let depth = if peak.1 > 0.0 { eq / peak.1 - 1.0 } else { 0.0 };
            match &mut episode {
                None => episode = Some((peak.0, ts, depth)),
                Some((_, trough, d)) => {
                    if depth < *d {
                        *d = depth;
                        *trough = ts;
                    }
                }
            }
        }
    }
    if let Some((start, trough, depth)) = episode {
        let end = equity[equity.len() - 1].0;
        let ep = DrawdownEpisode {
            start,
            trough,
            recovered: None,
            depth,
            duration_secs: (end - start).num_seconds(),
        };
        if best
            .as_ref()
            .is_none_or(|b| ep.duration_secs > b.duration_secs)
        {
            best = Some(ep);
        }
    }
    best
}

fn exposure(equity: &[(DateTime<Utc>, f64)], trades: &[Trade], metrics: &MetricSet) -> Exposure {
    let span = match (equity.first(), equity.last()) {
        (Some(a), Some(b)) => (b.0 - a.0).num_seconds().max(1) as f64,
        _ => 1.0,
    };
    let held: f64 = trades.iter().map(|t| t.holding_period_secs as f64).sum();
    let gross: f64 = trades
        .iter()
        .map(|t| f(t.pnl).abs() + f(t.costs_paid))
        .sum();
    let costs: f64 = trades.iter().map(|t| f(t.costs_paid)).sum();
    Exposure {
        time_in_market: (held / span).min(1.0),
        turnover: metrics.turnover,
        cost_drag: if gross > 0.0 { costs / gross } else { 0.0 },
    }
}

fn by_regime(
    equity: &[(DateTime<Utc>, f64)],
    trades: &[Trade],
    regimes: &[(DateTime<Utc>, DateTime<Utc>, String)],
) -> Vec<RegimeSlice> {
    if regimes.is_empty() || equity.is_empty() {
        return Vec::new();
    }
    let span = (equity[equity.len() - 1].0 - equity[0].0)
        .num_seconds()
        .max(1) as f64;
    let mut acc: BTreeMap<String, (f64, usize, f64)> = BTreeMap::new(); // ret, trades, secs
    for (start, end, label) in regimes {
        let first = equity.iter().find(|(ts, _)| ts >= start).map(|p| p.1);
        let last = equity.iter().rev().find(|(ts, _)| ts < end).map(|p| p.1);
        let ret = match (first, last) {
            (Some(a), Some(b)) if a > 0.0 => b / a - 1.0,
            _ => 0.0,
        };
        let n = trades
            .iter()
            .filter(|t| t.exit_time >= *start && t.exit_time < *end)
            .count();
        let e = acc.entry(label.clone()).or_default();
        e.0 += ret;
        e.1 += n;
        e.2 += (*end - *start).num_seconds().max(0) as f64;
    }
    acc.into_iter()
        .map(|(label, (ret, n_trades, secs))| RegimeSlice {
            label,
            ret,
            n_trades,
            share_of_time: secs / span,
        })
        .collect()
}

fn render_text(b: &DiagnosticBundle) -> String {
    let m = &b.metrics;
    let t = &b.trades;
    let mut s = String::with_capacity(MAX_TEXT_BYTES);
    let _ = writeln!(
        s,
        "run {} [{:?}] ret {:+.1}% sharpe {:.2} sortino {:.2} calmar {:.2} maxDD {:.1}% PF {:.2} expectancy {:.2}R",
        b.run_id,
        b.status,
        m.total_return * 100.0,
        m.sharpe,
        m.sortino,
        m.calmar,
        m.max_drawdown * 100.0,
        m.profit_factor,
        m.expectancy,
    );
    let _ = writeln!(
        s,
        "trades {} (win {:.0}%, avg win {:.2} / avg loss {:.2}, longest losing streak {}, hold p50 {}h, MAE p50 {:.2}% MFE p50 {:.2}%)",
        t.n,
        t.win_rate * 100.0,
        t.avg_win,
        t.avg_loss,
        t.longest_losing_streak,
        t.hold_secs_p50 / 3600,
        t.mae_p50 * 100.0,
        t.mfe_p50 * 100.0,
    );
    let _ = writeln!(
        s,
        "exposure: in-market {:.0}%, cost drag {:.1}% of gross",
        b.exposure.time_in_market * 100.0,
        b.exposure.cost_drag * 100.0
    );
    if let Some(dd) = &b.longest_drawdown {
        let _ = writeln!(
            s,
            "longest drawdown: {:.1}% from {} for {} days{}",
            dd.depth * 100.0,
            dd.start.format("%Y-%m-%d"),
            dd.duration_secs / 86_400,
            if dd.recovered.is_none() {
                " (unrecovered at end)"
            } else {
                ""
            }
        );
    }
    if !b.by_month.is_empty() {
        let mut worst: Vec<&MonthSlice> = b.by_month.iter().collect();
        worst.sort_by(|a, c| {
            a.ret
                .partial_cmp(&c.ret)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let losing = b.by_month.iter().filter(|x| x.ret < 0.0).count();
        let _ = writeln!(
            s,
            "months: {} total, {} losing; worst {}",
            b.by_month.len(),
            losing,
            worst
                .iter()
                .take(3)
                .map(|x| format!("{} {:+.1}%", x.ym, x.ret * 100.0))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if !b.by_regime.is_empty() {
        s.push_str("by regime: ");
        s.push_str(
            &b.by_regime
                .iter()
                .map(|r| {
                    format!(
                        "{} {:+.1}% ({} trades, {:.0}% of time)",
                        r.label,
                        r.ret * 100.0,
                        r.n_trades,
                        r.share_of_time * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
        );
        s.push('\n');
    }
    if !b.worst_trades.is_empty() {
        s.push_str("worst trades: ");
        s.push_str(
            &b.worst_trades
                .iter()
                .take(5)
                .map(|w| {
                    format!(
                        "{} {:?} {:+.2} ({}h)",
                        w.entry_time.format("%m-%d %H:%M"),
                        w.side,
                        w.pnl,
                        w.holding_period_secs / 3600
                    )
                })
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    if s.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

impl DiagnosticBundle {
    /// Build the bundle from a stored result. `regimes` may be empty.
    #[must_use]
    pub fn from_result(r: &RunResult, regimes: &[(DateTime<Utc>, DateTime<Utc>, String)]) -> Self {
        let trades = trade_stats(&r.trades, &r.metrics);
        let mut worst: Vec<&Trade> = r.trades.iter().collect();
        worst.sort_by_key(|t| t.pnl);
        let worst_trades = worst
            .into_iter()
            .take(10)
            .map(|t| WorstTrade {
                entry_time: t.entry_time,
                exit_time: t.exit_time,
                side: t.side,
                pnl: t.pnl,
                holding_period_secs: t.holding_period_secs,
                mae: t.mae,
            })
            .collect();
        let mut bundle = Self {
            run_id: r.run_id.as_str().to_string(),
            status: r.status,
            metrics: r.metrics,
            trades,
            by_month: by_month(&r.equity_curve, &r.trades),
            by_regime: by_regime(&r.equity_curve, &r.trades, regimes),
            worst_trades,
            longest_drawdown: longest_drawdown(&r.equity_curve),
            exposure: exposure(&r.equity_curve, &r.trades, &r.metrics),
            text: String::new(),
        };
        bundle.text = render_text(&bundle);
        bundle
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::run::executor::map_sim_result;
    use crate::run::{ComputeCost, DataSlice, EvalResolution, RunConfigBuilder};
    use chrono::{Duration, TimeZone};
    use rust_decimal_macros::dec;

    fn fixture() -> RunResult {
        let t0 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let slice = DataSlice::new(
            "BTC-USD",
            t0,
            t0 + Duration::days(120),
            EvalResolution::Day1,
        );
        let cfg = RunConfigBuilder::new("s", "v", slice, "c", "z", "snap").build();
        // Equity: up, then a 3-week drawdown, then recovery and new highs.
        let mut equity = Vec::new();
        let mut e = 100.0;
        for d in 0..120 {
            e *= match d {
                30..=50 => 0.995,
                _ => 1.004,
            };
            equity.push((t0 + Duration::days(d), e));
        }
        let mk = |day: i64, pnl: rust_decimal::Decimal| Trade {
            symbol: "BTC-USD".into(),
            side: Side::Long,
            entry_time: t0 + Duration::days(day),
            exit_time: t0 + Duration::days(day) + Duration::hours(6),
            entry_price: dec!(100),
            exit_price: dec!(101),
            qty: dec!(1),
            mae: -0.01,
            mfe: 0.02,
            holding_period_secs: 6 * 3600,
            costs_paid: dec!(0.1),
            pnl,
        };
        let trades = vec![
            mk(5, dec!(2.0)),
            mk(20, dec!(1.0)),
            mk(35, dec!(-3.0)),
            mk(40, dec!(-1.0)),
            mk(45, dec!(-0.5)),
            mk(70, dec!(4.0)),
        ];
        map_sim_result(&cfg, equity, vec![], trades, ComputeCost::default(), "test")
    }

    #[test]
    fn bundle_finds_the_drawdown_and_worst_trades() {
        let b = DiagnosticBundle::from_result(&fixture(), &[]);
        let dd = b.longest_drawdown.expect("drawdown episode");
        assert!(dd.depth < -0.05, "depth {}", dd.depth);
        assert!(dd.recovered.is_some());
        assert!(dd.duration_secs > 20 * 86_400);
        assert_eq!(b.worst_trades[0].pnl, dec!(-3.0));
        assert_eq!(b.trades.longest_losing_streak, 3);
        assert_eq!(b.by_month.len(), 4);
        assert!(b.by_month.iter().any(|m| m.ret < 0.0));
        assert!(b.text.len() <= 1_500);
        assert!(b.text.contains("longest drawdown"));
    }

    #[test]
    fn regime_slices_when_windows_supplied() {
        let r = fixture();
        let t0 = r.equity_curve[0].0;
        let regimes = vec![
            (t0, t0 + Duration::days(30), "trend".to_string()),
            (
                t0 + Duration::days(30),
                t0 + Duration::days(51),
                "chop".to_string(),
            ),
            (
                t0 + Duration::days(51),
                t0 + Duration::days(120),
                "trend".to_string(),
            ),
        ];
        let b = DiagnosticBundle::from_result(&r, &regimes);
        assert_eq!(b.by_regime.len(), 2);
        let chop = b.by_regime.iter().find(|x| x.label == "chop").unwrap();
        assert!(chop.ret < 0.0);
        assert_eq!(chop.n_trades, 3);
        assert!(b.text.contains("by regime"));
    }

    #[test]
    fn empty_result_is_safe() {
        let t0 = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let slice = DataSlice::new("X", t0, t0 + Duration::days(1), EvalResolution::Day1);
        let cfg = RunConfigBuilder::new("s", "v", slice, "c", "z", "snap").build();
        let r = RunResult::failed(&cfg, ledger::TerminalReason::DataError, "nope", "test");
        let b = DiagnosticBundle::from_result(&r, &[]);
        assert_eq!(b.status, RunStatus::Failed(ledger::TerminalReason::DataError));
        assert!(b.longest_drawdown.is_none());
        assert!(b.by_month.is_empty());
    }
}
