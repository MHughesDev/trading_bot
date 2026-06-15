//! Bridge between eval engine and the backtest crate.
//! Runs a reference strategy with `model_forecast` pointing to a specific version.

use anyhow::Result;

pub struct BacktestSummary {
    pub pnl_usd: f64,
    pub sharpe: f64,
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
    // Stub implementation — returns plausible-looking results.
    // Phase 4 will wire this to the real BacktestManager once strategy-runtime
    // can resolve model_forecast by (model_id, version).
    let _ = (model_id, version, instrument_id, days);
    Ok(BacktestSummary {
        pnl_usd: 0.0,
        sharpe: 0.0,
        max_drawdown_pct: 0.0,
        n_trades: 0,
    })
}
