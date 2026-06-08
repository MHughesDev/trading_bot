//! Typed Rust structs mirroring `github.com/MHughesDev/market_simulator` contracts.
//!
//! These are data-transfer types only — no logic.  When market_simulator updates
//! its contract, update the column names/types here and in `export.rs`.

use serde::{Deserialize, Serialize};

/// Schema version of the OHLCV Arrow IPC export this repo produces.
pub const OHLCV_SCHEMA_VERSION: &str = "1";

/// Expected column names in the Arrow IPC OHLCV schema (in order).
pub const OHLCV_COLUMNS: &[&str] = &["timestamp_ns", "open", "high", "low", "close", "volume"];

/// A single OHLCV row as expected by market_simulator.
///
/// `timestamp_ns` is nanoseconds since the Unix epoch (UTC).
/// OHLCV fields are plain `f64` per market_simulator's data contract — these are
/// not money types because they cross the external library boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OhlcvRecord {
    pub timestamp_ns: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// A backtest run request submitted to market_simulator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRequest {
    pub strategy_id: String,
    pub definition: serde_json::Value,
    pub instrument_id: String,
    /// Raw Arrow IPC stream bytes.
    #[serde(skip)]
    pub ohlcv_ipc_bytes: Vec<u8>,
    pub start_capital: f64,
    pub data_schema_version: String,
}

/// A single completed trade returned by market_simulator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub entry_time_ns: i64,
    pub exit_time_ns: i64,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: f64,
    pub pnl: f64,
}

/// Aggregated backtest result returned from market_simulator and translated to
/// this repo's domain type by `results.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub strategy_id: String,
    pub instrument_id: String,
    pub total_return_pct: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown_pct: f64,
    pub total_trades: u64,
    pub winning_trades: u64,
    pub trades: Vec<TradeRecord>,
}
