//! [`MetricSet`] — the standardized metric shape every Run produces (spec §1.1).
//!
//! All fields are `f64` (statistical, "not money" — D-10). The two **honesty
//! hooks** (`trial_count_at_eval`, `is_oos_gap`) are `None` at the Run level and
//! are populated only by Studies (Phase 1) and gates (Phase 4).

use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use super::result::Trade;

/// Which metric a distribution is taken over (used by Studies, Phase 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    Cagr,
    TotalReturn,
    Sharpe,
    Sortino,
    Calmar,
    DetrendedSharpe,
    MaxDrawdown,
    ProfitFactor,
}

/// Inputs to [`MetricSet::compute`]. `equity_returns` are per-period simple
/// returns; `periods_per_year` annualizes (e.g. 252 for daily, 8760 for hourly).
pub struct MetricInputs<'a> {
    pub equity_returns: &'a [f64],
    pub trades: &'a [Trade],
    /// Per-period benchmark returns, aligned with `equity_returns`. Drives
    /// alpha/beta/information_ratio/detrended_sharpe when present.
    pub benchmark_returns: Option<&'a [f64]>,
    /// Per-period net exposure fraction, aligned with `equity_returns`.
    pub net_exposure: Option<&'a [f64]>,
    pub periods_per_year: f64,
}

/// Standardized metric set (spec §1.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSet {
    // return
    pub cagr: f64,
    pub total_return: f64,
    pub ann_vol: f64,
    pub sharpe: f64,
    pub sortino: f64,
    pub calmar: f64,
    pub information_ratio: f64,
    pub alpha: f64,
    pub beta: f64,
    /// Net of market drift (and average-position bias) — Aronson detrending.
    pub detrended_sharpe: f64,
    // risk
    pub max_drawdown: f64,
    pub avg_drawdown: f64,
    pub time_in_drawdown_pct: f64,
    pub cvar_95: f64,
    pub ulcer_index: f64,
    // activity
    pub turnover: f64,
    pub exposure_gross: f64,
    pub exposure_net: f64,
    pub hit_rate: f64,
    pub profit_factor: f64,
    pub n_trades: i64,
    // honesty hooks (populated by Studies; null at Run level)
    pub trial_count_at_eval: Option<i64>,
    pub is_oos_gap: Option<f64>,
}

impl MetricSet {
    /// An all-zero metric set (for failed/rejected runs).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            cagr: 0.0,
            total_return: 0.0,
            ann_vol: 0.0,
            sharpe: 0.0,
            sortino: 0.0,
            calmar: 0.0,
            information_ratio: 0.0,
            alpha: 0.0,
            beta: 0.0,
            detrended_sharpe: 0.0,
            max_drawdown: 0.0,
            avg_drawdown: 0.0,
            time_in_drawdown_pct: 0.0,
            cvar_95: 0.0,
            ulcer_index: 0.0,
            turnover: 0.0,
            exposure_gross: 0.0,
            exposure_net: 0.0,
            hit_rate: 0.0,
            profit_factor: 0.0,
            n_trades: 0,
            trial_count_at_eval: None,
            is_oos_gap: None,
        }
    }

    /// Fetch the scalar value of a given metric kind (for Study distributions).
    #[must_use]
    pub fn value(&self, kind: MetricKind) -> f64 {
        match kind {
            MetricKind::Cagr => self.cagr,
            MetricKind::TotalReturn => self.total_return,
            MetricKind::Sharpe => self.sharpe,
            MetricKind::Sortino => self.sortino,
            MetricKind::Calmar => self.calmar,
            MetricKind::DetrendedSharpe => self.detrended_sharpe,
            MetricKind::MaxDrawdown => self.max_drawdown,
            MetricKind::ProfitFactor => self.profit_factor,
        }
    }

    /// Compute the full metric set from returns + trades (+ optional benchmark).
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn compute(input: &MetricInputs<'_>) -> Self {
        let r = input.equity_returns;
        let n = r.len();
        if n == 0 {
            return Self::empty();
        }
        let ppy = input.periods_per_year.max(1.0);

        // Compound and per-period moments.
        let total_return = r.iter().fold(1.0, |acc, x| acc * (1.0 + x)) - 1.0;
        let mean = r.iter().sum::<f64>() / n as f64;
        let var = if n > 1 {
            r.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1) as f64
        } else {
            0.0
        };
        let std = var.sqrt();
        let ann_vol = std * ppy.sqrt();
        let cagr = if total_return <= -1.0 {
            -1.0
        } else {
            (1.0 + total_return).powf(ppy / n as f64) - 1.0
        };
        let sharpe = if std > 0.0 { mean / std * ppy.sqrt() } else { 0.0 };

        // Sortino: downside deviation about 0.
        let dvar = r.iter().map(|x| x.min(0.0).powi(2)).sum::<f64>() / n as f64;
        let dstd = dvar.sqrt();
        let sortino = if dstd > 0.0 { mean / dstd * ppy.sqrt() } else { 0.0 };

        // Drawdown series from the cumulative equity curve.
        let (max_dd, avg_dd, tid_pct, ulcer) = drawdown_stats(r);
        let calmar = if max_dd.abs() > 0.0 { cagr / max_dd.abs() } else { 0.0 };

        // CVaR 95%: mean of the worst 5% of returns.
        let cvar_95 = cvar(r, 0.95);

        // Benchmark-relative stats.
        let (alpha, beta, information_ratio, detrended_sharpe) = match input.benchmark_returns {
            Some(b) if b.len() == n && n > 1 => {
                let bmean = b.iter().sum::<f64>() / n as f64;
                let bvar = b.iter().map(|x| (x - bmean).powi(2)).sum::<f64>() / (n - 1) as f64;
                let cov = r
                    .iter()
                    .zip(b)
                    .map(|(x, y)| (x - mean) * (y - bmean))
                    .sum::<f64>()
                    / (n - 1) as f64;
                let beta = if bvar > 0.0 { cov / bvar } else { 0.0 };
                let alpha = (mean - beta * bmean) * ppy;
                // Active returns r - b.
                let active: Vec<f64> = r.iter().zip(b).map(|(x, y)| x - y).collect();
                let amean = active.iter().sum::<f64>() / n as f64;
                let astd = (active.iter().map(|x| (x - amean).powi(2)).sum::<f64>()
                    / (n - 1) as f64)
                    .sqrt();
                let ir = if astd > 0.0 { amean / astd * ppy.sqrt() } else { 0.0 };
                // Detrended Sharpe: Sharpe of returns net of market drift
                // (active returns). A pragmatic stand-in for Aronson's full
                // detrending (also net of average-position bias).
                let dsh = if astd > 0.0 { amean / astd * ppy.sqrt() } else { 0.0 };
                (alpha, beta, ir, dsh)
            }
            _ => {
                // No benchmark: detrend only by the strategy's own mean (removes
                // the average-position bias component).
                let demeaned_std = std;
                let dsh = if demeaned_std > 0.0 {
                    // mean of (r - mean) is 0 by construction → detrended Sharpe
                    // collapses to 0 without a market series; report 0.
                    0.0
                } else {
                    0.0
                };
                (0.0, 0.0, 0.0, dsh)
            }
        };

        // Trade-derived activity stats.
        let n_trades = input.trades.len() as i64;
        let (mut wins, mut gains, mut losses, mut notional) = (0i64, 0.0f64, 0.0f64, 0.0f64);
        for t in input.trades {
            let pnl = t.pnl.to_f64().unwrap_or(0.0);
            if pnl > 0.0 {
                wins += 1;
                gains += pnl;
            } else {
                losses += -pnl;
            }
            let entry = t.entry_price.to_f64().unwrap_or(0.0);
            let qty = t.qty.to_f64().unwrap_or(0.0);
            notional += (entry * qty).abs();
        }
        let hit_rate = if n_trades > 0 { wins as f64 / n_trades as f64 } else { 0.0 };
        let profit_factor = if losses > 0.0 {
            gains / losses
        } else if gains > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        // Exposure from the net-exposure series when available.
        let (exposure_gross, exposure_net) = match input.net_exposure {
            Some(e) if !e.is_empty() => {
                let gross = e.iter().map(|x| x.abs()).sum::<f64>() / e.len() as f64;
                let net = e.iter().sum::<f64>() / e.len() as f64;
                (gross, net)
            }
            _ => (0.0, 0.0),
        };

        Self {
            cagr,
            total_return,
            ann_vol,
            sharpe,
            sortino,
            calmar,
            information_ratio,
            alpha,
            beta,
            detrended_sharpe,
            max_drawdown: max_dd,
            avg_drawdown: avg_dd,
            time_in_drawdown_pct: tid_pct,
            cvar_95,
            ulcer_index: ulcer,
            turnover: notional,
            exposure_gross,
            exposure_net,
            hit_rate,
            profit_factor,
            n_trades,
            trial_count_at_eval: None,
            is_oos_gap: None,
        }
    }
}

/// Returns `(max_drawdown, avg_drawdown, time_in_drawdown_pct, ulcer_index)`.
/// Drawdowns are negative fractions; `max_drawdown` is the most negative.
fn drawdown_stats(returns: &[f64]) -> (f64, f64, f64, f64) {
    let mut equity = 1.0;
    let mut peak = 1.0;
    let mut max_dd = 0.0f64;
    let mut dd_sum = 0.0f64;
    let mut dd_count = 0u64;
    let mut sq_sum = 0.0f64;
    let n = returns.len();
    for r in returns {
        equity *= 1.0 + r;
        if equity > peak {
            peak = equity;
        }
        let dd = if peak > 0.0 { equity / peak - 1.0 } else { 0.0 };
        if dd < max_dd {
            max_dd = dd;
        }
        if dd < 0.0 {
            dd_sum += dd;
            dd_count += 1;
        }
        sq_sum += dd * dd;
    }
    let avg_dd = if dd_count > 0 { dd_sum / dd_count as f64 } else { 0.0 };
    let tid_pct = if n > 0 { dd_count as f64 / n as f64 * 100.0 } else { 0.0 };
    let ulcer = if n > 0 { (sq_sum / n as f64).sqrt() } else { 0.0 };
    (max_dd, avg_dd, tid_pct, ulcer)
}

/// Conditional value at risk: mean of the worst `(1 - level)` tail of returns.
fn cvar(returns: &[f64], level: f64) -> f64 {
    if returns.is_empty() {
        return 0.0;
    }
    let mut sorted = returns.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let tail = ((1.0 - level) * sorted.len() as f64).ceil().max(1.0) as usize;
    let slice = &sorted[..tail.min(sorted.len())];
    slice.iter().sum::<f64>() / slice.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_yield_empty_metrics() {
        let m = MetricSet::compute(&MetricInputs {
            equity_returns: &[],
            trades: &[],
            benchmark_returns: None,
            net_exposure: None,
            periods_per_year: 252.0,
        });
        assert_eq!(m.total_return, 0.0);
        assert_eq!(m.n_trades, 0);
        assert!(m.trial_count_at_eval.is_none());
    }

    #[test]
    fn known_fixture_total_return_and_sharpe() {
        // Four periods of +1%, all positive → positive Sharpe, no drawdown.
        let r = [0.01, 0.01, 0.01, 0.01];
        let m = MetricSet::compute(&MetricInputs {
            equity_returns: &r,
            trades: &[],
            benchmark_returns: None,
            net_exposure: None,
            periods_per_year: 252.0,
        });
        let expected_total = 1.01f64.powi(4) - 1.0;
        assert!((m.total_return - expected_total).abs() < 1e-12);
        // Constant positive returns → zero variance → Sharpe defined as 0.
        assert_eq!(m.sharpe, 0.0);
        assert_eq!(m.max_drawdown, 0.0);
    }

    #[test]
    fn drawdown_is_negative_when_equity_falls() {
        let r = [0.10, -0.20, 0.05];
        let m = MetricSet::compute(&MetricInputs {
            equity_returns: &r,
            trades: &[],
            benchmark_returns: None,
            net_exposure: None,
            periods_per_year: 252.0,
        });
        assert!(m.max_drawdown < 0.0);
        assert!(m.time_in_drawdown_pct > 0.0);
        assert!(m.ulcer_index > 0.0);
    }

    #[test]
    fn sharpe_positive_for_noisy_positive_mean() {
        let r = [0.02, -0.01, 0.03, 0.00, 0.015];
        let m = MetricSet::compute(&MetricInputs {
            equity_returns: &r,
            trades: &[],
            benchmark_returns: None,
            net_exposure: None,
            periods_per_year: 252.0,
        });
        assert!(m.sharpe > 0.0);
        assert!(m.ann_vol > 0.0);
    }

    #[test]
    fn benchmark_gives_beta_near_one_for_identical_series() {
        let r = [0.02, -0.01, 0.03, 0.00, 0.015];
        let m = MetricSet::compute(&MetricInputs {
            equity_returns: &r,
            trades: &[],
            benchmark_returns: Some(&r),
            net_exposure: None,
            periods_per_year: 252.0,
        });
        assert!((m.beta - 1.0).abs() < 1e-9);
        // Active returns are all zero → IR and detrended Sharpe are 0.
        assert_eq!(m.information_ratio, 0.0);
        assert_eq!(m.detrended_sharpe, 0.0);
    }

    #[test]
    fn exposure_from_series() {
        let r = [0.0, 0.0];
        let e = [1.0, -1.0];
        let m = MetricSet::compute(&MetricInputs {
            equity_returns: &r,
            trades: &[],
            benchmark_returns: None,
            net_exposure: Some(&e),
            periods_per_year: 252.0,
        });
        assert!((m.exposure_gross - 1.0).abs() < 1e-12);
        assert!((m.exposure_net - 0.0).abs() < 1e-12);
    }
}
