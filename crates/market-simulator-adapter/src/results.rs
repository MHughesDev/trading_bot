//! Parse market_simulator result payloads into `BacktestReport`.

use thiserror::Error;

use crate::contract::{BacktestReport, TradeRecord};

/// Errors produced during result parsing.
#[derive(Debug, Error)]
pub enum ResultsError {
    #[error("failed to deserialise results: {0}")]
    Deserialise(#[from] serde_json::Error),
}

/// Parse a JSON result payload returned by market_simulator into a `BacktestReport`.
pub fn parse_results(
    strategy_id: &str,
    instrument_id: &str,
    json: &str,
) -> Result<BacktestReport, ResultsError> {
    #[derive(serde::Deserialize)]
    struct Raw {
        total_return_pct: f64,
        sharpe_ratio: f64,
        max_drawdown_pct: f64,
        total_trades: u64,
        winning_trades: u64,
        trades: Vec<TradeRecord>,
    }

    let raw: Raw = serde_json::from_str(json)?;
    Ok(BacktestReport {
        strategy_id: strategy_id.to_owned(),
        instrument_id: instrument_id.to_owned(),
        total_return_pct: raw.total_return_pct,
        sharpe_ratio: raw.sharpe_ratio,
        max_drawdown_pct: raw.max_drawdown_pct,
        total_trades: raw.total_trades,
        winning_trades: raw.winning_trades,
        trades: raw.trades,
    })
}

/// Build a placeholder `BacktestReport` when no actual run has been performed.
pub fn placeholder_report(strategy_id: &str, instrument_id: &str) -> BacktestReport {
    BacktestReport {
        strategy_id: strategy_id.to_owned(),
        instrument_id: instrument_id.to_owned(),
        total_return_pct: 0.0,
        sharpe_ratio: 0.0,
        max_drawdown_pct: 0.0,
        total_trades: 0,
        winning_trades: 0,
        trades: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_result() {
        let json = r#"{
            "total_return_pct": 12.5,
            "sharpe_ratio": 1.8,
            "max_drawdown_pct": 5.0,
            "total_trades": 42,
            "winning_trades": 28,
            "trades": []
        }"#;
        let report = parse_results("s", "BTC-USDT", json).unwrap();
        assert_eq!(report.strategy_id, "s");
        assert_eq!(report.instrument_id, "BTC-USDT");
        assert_eq!(report.total_trades, 42);
        assert!((report.sharpe_ratio - 1.8).abs() < 1e-6);
    }

    #[test]
    fn placeholder_has_zero_metrics() {
        let r = placeholder_report("s", "BTC-USDT");
        assert_eq!(r.total_trades, 0);
        assert_eq!(r.total_return_pct, 0.0);
    }
}
