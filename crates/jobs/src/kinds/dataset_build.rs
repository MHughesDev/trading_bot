//! `dataset_build` — materialise a declared dataset and run the leakage harness in a job
//! (Set N N-1.5; FEAT-006 section 3).
//!
//! The job is a job-service concern (durable, resumable, idempotent: the dataset is keyed by
//! its spec hash, so a re-run returns the pinned row); the domain work — the point-in-time
//! pull, the features, the labels, the leakage harness, the Parquet write and the
//! registration — is delegated through [`DatasetBuilder`], which `crates/api` implements over
//! the model registry's `DatasetManager`.
//!
//! A dataset that fails the harness is **not registered** and the job ends terminal
//! `leakage_detected`, not retried: a leak does not heal on a second attempt.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{JobError, JobKind, JobOutput, Progress};
use crate::worker::{JobContext, Worker};

/// One dataset to build. `request` is the model registry's `DatasetRequest` as JSON; this
/// crate does not depend on that one, and the builder parses it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetBuildManifest {
    pub tenant: String,
    pub request: Value,
}

impl DatasetBuildManifest {
    /// # Errors
    /// A manifest that does not parse, or has no tenant or request.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let m: Self = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        if m.tenant.trim().is_empty() {
            return Err("tenant is required".into());
        }
        if !m.request.is_object() {
            return Err("request must be a DatasetRequest object".into());
        }
        Ok(m)
    }
}

/// What a build registered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DatasetBuildOutcome {
    pub dataset_id: String,
    pub row_count: i64,
    /// The leakage manifest the dataset was registered with (engine version, data snapshot,
    /// checks).
    pub leakage: Value,
}

/// Why a build did not register a dataset.
#[derive(Debug)]
pub enum BuildFailure {
    /// The harness failed; `failures` names each failed check. Nothing was registered.
    Leakage {
        dataset_id: String,
        failures: Vec<String>,
    },
    /// The request was unusable (does not parse, unknown feature set, ...).
    Invalid(String),
    /// Anything else (store down, write failed): worth retrying.
    Other(anyhow::Error),
}

/// The domain work behind the job.
#[async_trait::async_trait]
pub trait DatasetBuilder: Send + Sync {
    /// Plan and materialise `m`, running the leakage harness before anything is stored.
    ///
    /// # Errors
    /// [`BuildFailure`].
    async fn build(&self, m: &DatasetBuildManifest) -> Result<DatasetBuildOutcome, BuildFailure>;
}

/// The manifest-to-result mapping, free of the worker plumbing so it is testable.
///
/// # Errors
/// A bad manifest or invalid request is terminal `integrity_rejected`; a leak is terminal
/// `leakage_detected`; anything else is a retryable infrastructure failure.
pub async fn run_build(
    builder: &dyn DatasetBuilder,
    manifest: &Value,
) -> Result<JobOutput, JobError> {
    let m = DatasetBuildManifest::from_value(manifest).map_err(|e| {
        JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "invalid_manifest",
            e,
        )
    })?;
    match builder.build(&m).await {
        Ok(out) => Ok(JobOutput {
            result: serde_json::to_value(&out).unwrap_or_default(),
            ..JobOutput::default()
        }),
        Err(BuildFailure::Leakage {
            dataset_id,
            failures,
        }) => Err(JobError::logic(
            ledger::TerminalReason::LeakageDetected,
            "leakage_detected",
            format!(
                "dataset {dataset_id} failed the leakage harness and was not registered: {}",
                failures.join(" | ")
            ),
        )),
        Err(BuildFailure::Invalid(why)) => Err(JobError::logic(
            ledger::TerminalReason::IntegrityRejected,
            "invalid_dataset_request",
            why,
        )),
        Err(BuildFailure::Other(e)) => {
            Err(JobError::infrastructure(format!("dataset_build: {e:#}")))
        }
    }
}

/// The worker.
pub struct DatasetBuildWorker {
    builder: Arc<dyn DatasetBuilder>,
}

impl DatasetBuildWorker {
    #[must_use]
    pub fn new(builder: Arc<dyn DatasetBuilder>) -> Self {
        Self { builder }
    }
}

#[async_trait::async_trait]
impl Worker for DatasetBuildWorker {
    fn kind(&self) -> JobKind {
        JobKind::DatasetBuild
    }

    async fn run(&self, ctx: &JobContext, manifest: &Value) -> Result<JobOutput, JobError> {
        ctx.progress(Progress {
            pct: Some(0.0),
            stage: Some("dataset_build".into()),
            message: Some(
                "pulling bars, building features and labels, checking for leakage".into(),
            ),
        })
        .await;
        run_build(self.builder.as_ref(), manifest).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(fn() -> Result<DatasetBuildOutcome, BuildFailure>);

    #[async_trait::async_trait]
    impl DatasetBuilder for Fake {
        async fn build(
            &self,
            _: &DatasetBuildManifest,
        ) -> Result<DatasetBuildOutcome, BuildFailure> {
            (self.0)()
        }
    }

    fn manifest() -> Value {
        serde_json::json!({"tenant": "t", "request": {"feature_set_ref": "core"}})
    }

    #[test]
    fn the_kind_is_dataset_build_and_a_manifest_needs_a_tenant_and_a_request() {
        assert_eq!(JobKind::DatasetBuild.as_str(), "dataset_build");
        assert!(DatasetBuildManifest::from_value(&manifest()).is_ok());
        assert!(DatasetBuildManifest::from_value(
            &serde_json::json!({"tenant": "", "request": {}})
        )
        .is_err());
        assert!(DatasetBuildManifest::from_value(
            &serde_json::json!({"tenant": "t", "request": 3})
        )
        .is_err());
        assert!(DatasetBuildManifest::from_value(
            &serde_json::json!({"tenant": "t", "request": {}, "extra": 1})
        )
        .is_err());
    }

    #[tokio::test]
    async fn a_clean_build_returns_the_dataset_and_its_leakage_manifest() {
        let b = Fake(|| {
            Ok(DatasetBuildOutcome {
                dataset_id: "sha256:abc".into(),
                row_count: 10,
                leakage: serde_json::json!({"passed": true}),
            })
        });
        let out = run_build(&b, &manifest()).await.unwrap();
        assert_eq!(out.result["dataset_id"], "sha256:abc");
        assert_eq!(out.result["leakage"]["passed"], true);
    }

    #[tokio::test]
    async fn a_leaking_dataset_is_terminal_leakage_detected_and_not_retryable() {
        let b = Fake(|| {
            Err(BuildFailure::Leakage {
                dataset_id: "sha256:abc".into(),
                failures: vec!["label_fold_overlap: rows 4..9".into()],
            })
        });
        let e = run_build(&b, &manifest()).await.unwrap_err();
        assert_eq!(e.terminal, ledger::TerminalReason::LeakageDetected);
        assert!(!e.retryable);
        assert!(e.fix.as_deref().unwrap().contains("label_fold_overlap"));
    }

    #[tokio::test]
    async fn an_invalid_request_is_terminal_and_an_outage_is_retried() {
        let invalid = Fake(|| Err(BuildFailure::Invalid("unknown feature set".into())));
        let e = run_build(&invalid, &manifest()).await.unwrap_err();
        assert_eq!(e.terminal, ledger::TerminalReason::IntegrityRejected);
        assert!(!e.retryable);
        let down = Fake(|| Err(BuildFailure::Other(anyhow::anyhow!("clickhouse down"))));
        assert!(run_build(&down, &manifest()).await.unwrap_err().retryable);
        let bad = run_build(&down, &serde_json::json!({"nope": 1}))
            .await
            .unwrap_err();
        assert_eq!(bad.code, "invalid_manifest");
    }
}
