//! `layer1_run` manifest (Set M M-1.5; FEAT-004 §2.2).
//!
//! The manifest names a research-strategy source file, its parameters and the window. It has
//! **no P&L field** and `deny_unknown_fields`, so a submission that carries one is refused
//! at parse (ST-02), not read and ignored.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// How many truncation cuts the Gate-0 test uses unless asked otherwise.
pub const DEFAULT_CUTS: usize = 8;
pub const MAX_SOURCE_BYTES: usize = 128 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer1Manifest {
    /// The ledger experiment this run is a trial of.
    pub experiment_id: String,
    /// Python defining one `@research_strategy` function named `positions`.
    pub source: String,
    pub instrument: String,
    pub timeframe: String,
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    /// Point-in-time read horizon: bars first known at or before this instant.
    pub as_of: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default = "default_cuts")]
    pub truncation_cuts: usize,
    #[serde(default = "default_equity")]
    pub initial_equity: f64,
    pub fee_bps: f64,
    pub half_spread_bps: f64,
    /// A dated cost model (`backtest::cost::CostModel` as JSON). When present it replaces
    /// `fee_bps` and `half_spread_bps` and adds square-root impact (Set M M-2.1).
    #[serde(default)]
    pub cost_model: Option<Value>,
    /// Sample the declared params instead of running one point (Set M M-1.6). The caller
    /// supplies a count, never the points.
    #[serde(default)]
    pub sweep: Option<SweepSpec>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepSpec {
    pub max_runs: u32,
}

fn default_cuts() -> usize {
    DEFAULT_CUTS
}
fn default_equity() -> f64 {
    10_000.0
}

impl Layer1Manifest {
    /// # Errors
    /// A manifest that does not parse (an unknown field such as `pnl` included), is empty,
    /// has an empty window, or states costs that are negative or non-finite.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let m: Self = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        if m.source.trim().is_empty() || m.source.len() > MAX_SOURCE_BYTES {
            return Err(format!("source must be 1..={MAX_SOURCE_BYTES} bytes"));
        }
        if m.start >= m.end {
            return Err("start must be before end".into());
        }
        if m.experiment_id.trim().is_empty() {
            return Err(
                "experiment_id is required: a Layer 1 run is a trial of an experiment".into(),
            );
        }
        if !(1..=64).contains(&m.truncation_cuts) {
            return Err("truncation_cuts must be 1..=64: zero would skip the Gate-0 test".into());
        }
        let bad = |x: f64| !x.is_finite() || x < 0.0;
        if bad(m.fee_bps)
            || bad(m.half_spread_bps)
            || !(m.initial_equity.is_finite() && m.initial_equity > 0.0)
        {
            return Err("fee_bps and half_spread_bps must be finite and non-negative, initial_equity positive".into());
        }
        if let Some(sw) = m.sweep {
            if !(1..=200).contains(&sw.max_runs) {
                return Err("sweep.max_runs must be 1..=200".into());
            }
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Value {
        serde_json::json!({"experiment_id":"e1","source":"def positions(d,p): pass","instrument":"BTC-USD",
            "timeframe":"1h","start":"2025-01-01T00:00:00Z","end":"2025-02-01T00:00:00Z",
            "as_of":"2025-03-01T00:00:00Z","fee_bps":10.0,"half_spread_bps":2.0})
    }

    #[test]
    fn a_manifest_parses_and_a_pnl_field_is_refused_at_the_type() {
        assert!(Layer1Manifest::from_value(&ok()).is_ok());
        let mut v = ok();
        v["pnl"] = serde_json::json!(1.0);
        assert!(Layer1Manifest::from_value(&v).unwrap_err().contains("pnl"));
        let mut v = ok();
        v["equity"] = serde_json::json!([1.0]);
        assert!(Layer1Manifest::from_value(&v).is_err());
    }

    #[test]
    fn zero_cuts_would_skip_the_gate_zero_test_and_is_refused() {
        let mut v = ok();
        v["truncation_cuts"] = serde_json::json!(0);
        assert!(Layer1Manifest::from_value(&v)
            .unwrap_err()
            .contains("truncation_cuts"));
    }

    #[test]
    fn an_empty_window_a_blank_experiment_and_negative_costs_are_refused() {
        let mut v = ok();
        v["end"] = serde_json::json!("2024-01-01T00:00:00Z");
        assert!(Layer1Manifest::from_value(&v).is_err());
        let mut v = ok();
        v["experiment_id"] = serde_json::json!(" ");
        assert!(Layer1Manifest::from_value(&v).is_err());
        let mut v = ok();
        v["fee_bps"] = serde_json::json!(-1.0);
        assert!(Layer1Manifest::from_value(&v).is_err());
    }

    #[test]
    fn the_kind_is_research_class_and_not_counted_at_submission() {
        use crate::types::{JobKind, WorkerClass};
        assert_eq!(JobKind::parse("layer1_run"), Some(JobKind::Layer1Run));
        assert_eq!(JobKind::Layer1Run.worker_class(), WorkerClass::Research);
        assert!(
            !JobKind::Layer1Run.counts_trial(),
            "the trial is registered inside the run, after the code ran"
        );
    }
}
