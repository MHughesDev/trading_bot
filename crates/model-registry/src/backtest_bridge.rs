//! Bridge between eval engine and the backtest crate.
//! Runs a reference strategy with `model_forecast` pointing to a specific version.

use anyhow::Result;
use rust_decimal::Decimal;

pub struct BacktestSummary {
    /// Net P&L in USD — Decimal per ADR-0002 (no f64 on monetary fields).
    pub pnl_usd: Decimal,
    /// Annualised Sharpe ratio (dimensionless ratio, f64 acceptable).
    pub sharpe: f64,
    /// Max drawdown as a fraction 0..1 (dimensionless, f64 acceptable).
    pub max_drawdown_pct: f64,
    pub n_trades: u64,
}

/// Stub: real impl would call `BacktestManager::run` with a synthetic strategy
/// that routes `model_forecast` to the given (`model_id`, version).
#[allow(clippy::unused_async)]
pub async fn run_model_eval_backtest(
    model_id: &str,
    version: i32,
    instrument_id: &str,
    days: u32,
) -> Result<BacktestSummary> {
    // Stub — Phase 4 wires the real BacktestManager once strategy-runtime can
    // resolve model_forecast by (model_id, version).
    let _ = (model_id, version, instrument_id, days);
    Ok(BacktestSummary {
        pnl_usd: Decimal::ZERO,
        sharpe: 0.0,
        max_drawdown_pct: 0.0,
        n_trades: 0,
    })
}
