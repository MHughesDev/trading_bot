//! Adapter to `github.com/MHughesDev/market_simulator`.
//!
//! No fill simulation logic and no replay engine in this crate.
//! Translates between this repo's domain types and market_simulator's Arrow IPC
//! contracts (Engine A — equities, crypto spot CEX).

pub mod contract;
pub mod export;
pub mod results;
pub mod run_request;

pub use contract::{BacktestReport, OhlcvRecord, RunRequest, TradeRecord, OHLCV_COLUMNS};
pub use export::{bars_to_ipc_bytes, ohlcv_schema, ExportError, TimedBar};
pub use results::{parse_results, placeholder_report, ResultsError};
pub use run_request::{build_run_request, RunRequestError};
