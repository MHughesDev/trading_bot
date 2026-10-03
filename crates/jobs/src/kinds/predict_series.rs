//! `predict_series` — fill a model version's prediction series ahead of the
//! backtests that will read it (Set U U-5.5; FEAT-006 §6).
//!
//! The job is a job-service concern (durable, resumable, idempotent: the series
//! is keyed by what was asked, so a re-run rewrites the same rows); the domain
//! work — running the strategy engine's own loop over a window with the
//! point-in-time model provider writing the series — is delegated through
//! [`SeriesRunner`], which the platform implements where the registry, the bar
//! store and the inference sidecar are.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{JobError, JobKind, JobOutput, Progress};
use crate::worker::{JobContext, Worker};

/// One series to fill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PredictSeriesManifest {
    /// `name@version` (or `name@alias`, resolved when the job runs).
    pub model: String,
    pub instrument: String,
    pub timeframe: String,
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
}

impl PredictSeriesManifest {
    /// # Errors
    /// A manifest that does not parse, or an empty window.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let m: Self = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        if m.start >= m.end {
            return Err("start must be before end".into());
        }
        if !m.model.contains('@') {
            return Err(format!("'{}' must name a version: name@version", m.model));
        }
        Ok(m)
    }
}

/// What a series run wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeriesOutcome {
    pub answered: u64,
    pub before_training_end: u64,
    pub other: u64,
}

/// The domain work behind the job.
#[async_trait::async_trait]
pub trait SeriesRunner: Send + Sync {
    /// Fill the series for `m`.
    ///
    /// # Errors
    /// The reason it could not run, for the job record.
    async fn fill(&self, m: &PredictSeriesManifest) -> anyhow::Result<SeriesOutcome>;
}

/// The worker.
pub struct PredictSeriesWorker {
    runner: std::sync::Arc<dyn SeriesRunner>,
}

impl PredictSeriesWorker {
    #[must_use]
    pub fn new(runner: std::sync::Arc<dyn SeriesRunner>) -> Self {
        Self { runner }
    }
}

#[async_trait::async_trait]
impl Worker for PredictSeriesWorker {
    fn kind(&self) -> JobKind {
        JobKind::PredictSeries
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        let m = PredictSeriesManifest::from_value(manifest).map_err(|e| {
            JobError::logic(
                ledger::TerminalReason::IntegrityRejected,
                "invalid_manifest",
                e,
            )
        })?;
        ctx.progress(Progress {
            pct: Some(0.0),
            stage: Some("predict_series".into()),
            message: Some(format!("{} on {} {}", m.model, m.instrument, m.timeframe)),
        })
        .await;
        let out = self
            .runner
            .fill(&m)
            .await
            .map_err(|e| JobError::infrastructure(format!("predict_series: {e:#}")))?;
        Ok(JobOutput {
            result: serde_json::to_value(&out).unwrap_or_default(),
            ..JobOutput::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_names_a_version_and_a_real_window() {
        let ok = serde_json::json!({"model": "vol@3", "instrument": "BTC-USD", "timeframe": "1h",
            "start": "2025-01-01T00:00:00Z", "end": "2025-02-01T00:00:00Z"});
        assert!(PredictSeriesManifest::from_value(&ok).is_ok());
        let mut bad = ok.clone();
        bad["model"] = serde_json::json!("vol");
        assert!(PredictSeriesManifest::from_value(&bad).is_err());
        let mut empty = ok;
        empty["end"] = serde_json::json!("2024-12-01T00:00:00Z");
        assert!(PredictSeriesManifest::from_value(&empty).is_err());
    }
}
