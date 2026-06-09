//! Backtest tools: `run_backtest` and `get_backtest_result`.
//!
//! Thin wrappers over the Phase 4 backtest infrastructure.

use serde::{Deserialize, Serialize};

use market_simulator_adapter::{placeholder_report, BacktestReport};
use uuid::Uuid;

use crate::McpContext;

/// In-progress or completed backtest job status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BacktestStatus {
    Pending,
    Complete,
    Failed,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunBacktestResult {
    pub backtest_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BacktestResultResponse {
    pub backtest_id: String,
    pub status: BacktestStatus,
    pub report: Option<BacktestReport>,
    pub error: Option<String>,
}

/// `run_backtest` — submit a backtest job for a strategy on an instrument.
///
/// Returns a `backtest_id` that can be polled via `get_backtest_result`.
pub fn run_backtest(ctx: &McpContext, store_id: &str, instrument_id: &str) -> RunBacktestResult {
    let backtest_id = Uuid::new_v4();
    let strategy_id = {
        if let Ok(id) = store_id.parse::<Uuid>() {
            ctx.strategy_store
                .lock()
                .expect("strategy_store lock poisoned")
                .get(&id)
                .map(|d| d.strategy_id.clone())
                .unwrap_or_else(|| store_id.to_owned())
        } else {
            store_id.to_owned()
        }
    };

    let report = placeholder_report(&strategy_id, instrument_id);
    ctx.backtest_results
        .lock()
        .expect("backtest_results lock poisoned")
        .insert(backtest_id, Ok(report));

    RunBacktestResult {
        backtest_id: backtest_id.to_string(),
    }
}

/// `get_backtest_result` — fetch the result of a submitted backtest job.
pub fn get_backtest_result(ctx: &McpContext, backtest_id: &str) -> BacktestResultResponse {
    let id: Uuid = match backtest_id.parse() {
        Ok(v) => v,
        Err(_) => {
            return BacktestResultResponse {
                backtest_id: backtest_id.to_owned(),
                status: BacktestStatus::Failed,
                report: None,
                error: Some(format!("'{backtest_id}' is not a valid UUID")),
            }
        }
    };

    match ctx
        .backtest_results
        .lock()
        .expect("backtest_results lock poisoned")
        .get(&id)
    {
        None => BacktestResultResponse {
            backtest_id: backtest_id.to_owned(),
            status: BacktestStatus::Pending,
            report: None,
            error: None,
        },
        Some(Ok(report)) => BacktestResultResponse {
            backtest_id: backtest_id.to_owned(),
            status: BacktestStatus::Complete,
            report: Some(report.clone()),
            error: None,
        },
        Some(Err(e)) => BacktestResultResponse {
            backtest_id: backtest_id.to_owned(),
            status: BacktestStatus::Failed,
            report: None,
            error: Some(e.clone()),
        },
    }
}
