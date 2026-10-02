//! Job, request, and snapshot types for the backtesting system.

use chrono::{DateTime, Utc};
use domain::payloads::bar::Timeframe;
use domain::strategy_def::StrategyDefinition;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Lifecycle status of a backtest job.  Doubles as the "phase" indicator the
/// UI shows while a run is in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestStatus {
    Queued,
    CheckingData,
    CollectingData,
    LoadingData,
    Simulating,
    Completed,
    Failed,
    Cancelled,
}

impl BacktestStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::CheckingData => "checking_data",
            Self::CollectingData => "collecting_data",
            Self::LoadingData => "loading_data",
            Self::Simulating => "simulating",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "checking_data" => Self::CheckingData,
            "collecting_data" => Self::CollectingData,
            "loading_data" => Self::LoadingData,
            "simulating" => Self::Simulating,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

/// A create-backtest request as submitted by the UI.
///
/// Exactly one of `strategy_ref` (UUID in the platform strategy store) or
/// `definition` (inline v1.0 strategy JSON) must resolve to a definition —
/// the API layer performs that resolution before handing the job to the
/// manager.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacktestRequest {
    /// Display name; defaults to "strategy · instrument · timeframe".
    #[serde(default)]
    pub name: Option<String>,
    /// Slug (strategy_id) of a stored strategy definition.
    #[serde(default)]
    pub strategy_ref: Option<String>,
    /// Inline strategy definition (validated before use).
    #[serde(default)]
    pub definition: Option<StrategyDefinition>,
    /// Inline `tbot.strategy/3` graph (Set U U-3.4) — compiled before use, and
    /// run on the one strategy engine.
    #[serde(default)]
    pub graph: Option<domain::strategy_graph::StrategyGraph>,
    /// Instrument symbol, e.g. `"BTC-USDT"` or `"AAPL"`.
    pub instrument_id: String,
    /// Venue name shown on results (e.g. `"kraken"`); defaults per asset class.
    #[serde(default)]
    pub venue_id: Option<String>,
    /// Asset class key, e.g. `"crypto_spot_cex"`, `"equity"`.
    pub asset_class: String,
    /// Bar timeframe key: `"1s" | "1m" | "5m" | "15m" | "1h" | "4h" | "1d"`.
    pub timeframe: String,
    /// Simulation window start (UTC).
    pub start: DateTime<Utc>,
    /// Simulation window end (UTC).
    pub end: DateTime<Utc>,
    /// Starting account balance, decimal string (never a float).
    #[serde(default = "default_initial_balance")]
    pub initial_balance: String,
    /// Account/quote currency for the simulated venue.
    #[serde(default = "default_quote_currency")]
    pub quote_currency: String,
    /// Automatically backfill missing historical data before simulating.
    #[serde(default = "default_true")]
    pub auto_collect: bool,
}

fn default_initial_balance() -> String {
    "100000".to_string()
}

fn default_quote_currency() -> String {
    "USD".to_string()
}

fn default_true() -> bool {
    true
}

/// The strategy a backtest job runs (Set U U-3.4): a v1 definition on the
/// legacy `market_simulator` path, or a `tbot.strategy/3` graph on the one
/// strategy engine. Serialised as the document itself — which one it is, is read
/// from its `format` — so `backtest_jobs.definition` holds exactly what was run.
#[derive(Clone, Debug)]
// A handful of jobs live at once; boxing the v1 definition would ripple through
// every v1 path for no measurable gain.
#[allow(clippy::large_enum_variant)]
pub enum JobStrategy {
    V1(StrategyDefinition),
    V3(Box<domain::strategy_graph::StrategyGraph>),
}

impl JobStrategy {
    /// Parse a stored or submitted document.
    ///
    /// # Errors
    /// A document that is neither format.
    pub fn from_json(v: serde_json::Value) -> Result<Self, serde_json::Error> {
        if v.get("format").and_then(|f| f.as_str()) == Some(domain::strategy_graph::FORMAT) {
            Ok(Self::V3(Box::new(serde_json::from_value(v)?)))
        } else {
            Ok(Self::V1(serde_json::from_value(v)?))
        }
    }

    /// The strategy's name: the v1 slug, or the graph's name.
    #[must_use]
    pub fn slug(&self) -> &str {
        match self {
            Self::V1(d) => &d.strategy_id,
            Self::V3(g) => &g.name,
        }
    }

    /// What `RunConfig.strategy_version` records: the v1 version string, or the
    /// graph's content-addressed revision.
    #[must_use]
    pub fn version(&self) -> String {
        match self {
            Self::V1(d) => d.definition_version.clone(),
            Self::V3(g) => g
                .revision()
                .map(|r| r.as_str().to_string())
                .unwrap_or_default(),
        }
    }
}

impl Serialize for JobStrategy {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::V1(d) => d.serialize(s),
            Self::V3(g) => g.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for JobStrategy {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        Self::from_json(v).map_err(serde::de::Error::custom)
    }
}

/// A fully resolved job specification (request + resolved definition).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedSpec {
    pub name: String,
    pub definition: JobStrategy,
    pub instrument_id: String,
    pub venue_id: String,
    pub asset_class: String,
    pub timeframe: Timeframe,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// Decimal string.
    pub initial_balance: String,
    pub quote_currency: String,
    pub auto_collect: bool,
}

/// A contiguous range of missing historical data.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MissingRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

/// Coverage report for the requested window (including warm-up lead-in).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DataCoverage {
    /// Bars the window would contain with full data.
    pub expected_bars: u64,
    /// Bars currently present in `ClickHouse`.
    pub present_bars: u64,
    /// Bars backfilled by the collection phase of this job.
    pub collected_bars: u64,
    /// Ranges that were missing at check time.
    pub missing_ranges: Vec<MissingRange>,
}

/// Public snapshot of a backtest job, as served to the UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacktestSnapshot {
    pub id: Uuid,
    pub name: String,
    pub strategy_slug: String,
    pub instrument_id: String,
    pub venue_id: String,
    pub asset_class: String,
    pub timeframe: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub initial_balance: String,
    pub quote_currency: String,
    pub auto_collect: bool,
    pub status: BacktestStatus,
    /// Percent complete, 0–100.
    pub progress: f32,
    /// Failure detail when `status == Failed` (phase is the status at failure).
    pub error: Option<String>,
    /// Phase the job was in when it failed (for mid-processing diagnosis).
    pub failed_phase: Option<String>,
    pub coverage: Option<DataCoverage>,
    /// Simulator result document (orders, positions, `PnL` and return stats).
    pub result: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Timeframe helpers shared across the crate.
pub trait TimeframeExt {
    fn from_key(key: &str) -> Option<Timeframe>;
    fn key(&self) -> &'static str;
    fn seconds(&self) -> u64;
}

impl TimeframeExt for Timeframe {
    fn from_key(key: &str) -> Option<Timeframe> {
        match key {
            "1s" => Some(Timeframe::Seconds1),
            "1m" => Some(Timeframe::Minutes1),
            "5m" => Some(Timeframe::Minutes5),
            "15m" => Some(Timeframe::Minutes15),
            "1h" => Some(Timeframe::Hours1),
            "4h" => Some(Timeframe::Hours4),
            "1d" => Some(Timeframe::Daily),
            _ => None,
        }
    }

    fn key(&self) -> &'static str {
        match self {
            Timeframe::Seconds1 => "1s",
            Timeframe::Minutes1 => "1m",
            Timeframe::Minutes5 => "5m",
            Timeframe::Minutes15 => "15m",
            Timeframe::Hours1 => "1h",
            Timeframe::Hours4 => "4h",
            Timeframe::Daily => "1d",
        }
    }

    fn seconds(&self) -> u64 {
        match self {
            Timeframe::Seconds1 => 1,
            Timeframe::Minutes1 => 60,
            Timeframe::Minutes5 => 300,
            Timeframe::Minutes15 => 900,
            Timeframe::Hours1 => 3_600,
            Timeframe::Hours4 => 14_400,
            Timeframe::Daily => 86_400,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeframe_keys_round_trip() {
        for key in ["1s", "1m", "5m", "15m", "1h", "4h", "1d"] {
            let tf = <Timeframe as TimeframeExt>::from_key(key).unwrap();
            assert_eq!(tf.key(), key);
        }
        assert!(<Timeframe as TimeframeExt>::from_key("2m").is_none());
    }

    #[test]
    fn status_terminality() {
        assert!(BacktestStatus::Completed.is_terminal());
        assert!(BacktestStatus::Failed.is_terminal());
        assert!(BacktestStatus::Cancelled.is_terminal());
        assert!(!BacktestStatus::Simulating.is_terminal());
        assert!(!BacktestStatus::CollectingData.is_terminal());
    }

    /// Set U U-3.4: a job's strategy is stored as the document it ran, and read
    /// back as the same format.
    #[test]
    fn a_job_strategy_round_trips_as_the_document_it_ran() {
        let v3 = serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "g",
          "asset_class": "crypto_spot_cex", "timeframe": "1h",
          "nodes": [{"id":"go","op":"ge","args":{"a":"close","b":0}}],
          "decide": {"rules": [{"when":"go","then":{"kind":"hold"}}]}
        });
        let s = JobStrategy::from_json(v3.clone()).expect("a v3 graph");
        assert!(matches!(s, JobStrategy::V3(_)));
        assert_eq!(s.slug(), "g");
        assert_eq!(s.version().len(), 64, "the content-addressed revision");
        let back: JobStrategy = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert!(matches!(back, JobStrategy::V3(_)));
        assert_eq!(back.version(), s.version());

        // A request can carry the graph inline.
        let req: BacktestRequest = serde_json::from_value(serde_json::json!({
            "graph": v3, "instrument_id": "BTC-USD", "asset_class": "crypto_spot_cex",
            "timeframe": "1h", "start": "2025-01-01T00:00:00Z", "end": "2025-02-01T00:00:00Z"
        }))
        .unwrap();
        assert!(req.graph.is_some() && req.definition.is_none());
    }

    #[test]
    fn a_document_that_is_neither_format_is_refused() {
        assert!(JobStrategy::from_json(serde_json::json!({"format": "tbot.strategy/3"})).is_err());
        assert!(JobStrategy::from_json(serde_json::json!({"nonsense": true})).is_err());
    }
}
