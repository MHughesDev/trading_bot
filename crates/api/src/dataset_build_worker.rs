//! The `dataset_build` job's builder: a declared dataset is planned, built, leakage-checked
//! and registered by the model registry's `DatasetManager` (Set N N-1.5).
//!
//! `JobKind::DatasetBuild` existed with no worker, so a submitted build failed `no_worker`.
//! The job-service half (manifest, terminal mapping, retry rule) is
//! `jobs::kinds::dataset_build`; this is the half that reaches the registry. The leakage
//! harness itself runs inside `DatasetManager::materialize` before anything is stored; a
//! refusal arrives as the typed [`model_registry::datasets::LeakageRefused`] so it can end the
//! job terminal `leakage_detected` instead of being retried.

use jobs::kinds::dataset_build::{
    BuildFailure, DatasetBuildManifest, DatasetBuildOutcome, DatasetBuildWorker, DatasetBuilder,
};
use model_registry::datasets::{DatasetManager, DatasetRequest, LeakageRefused};
use sqlx::PgPool;

pub struct RegistryDatasetBuilder {
    pg: PgPool,
}

impl RegistryDatasetBuilder {
    #[must_use]
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }
}

/// The worker to register on the `Research` class.
#[must_use]
pub fn worker(pg: PgPool) -> DatasetBuildWorker {
    DatasetBuildWorker::new(std::sync::Arc::new(RegistryDatasetBuilder::new(pg)))
}

/// Classify a failure from plan or materialise: a typed leakage refusal and an unusable
/// request are terminal, a database error is retried.
fn classify(e: anyhow::Error) -> BuildFailure {
    if let Some(l) = e.downcast_ref::<LeakageRefused>() {
        return BuildFailure::Leakage {
            dataset_id: l.dataset_id.clone(),
            failures: l.failures.clone(),
        };
    }
    if e.downcast_ref::<sqlx::Error>().is_some() {
        return BuildFailure::Other(e);
    }
    BuildFailure::Invalid(format!("{e:#}"))
}

#[async_trait::async_trait]
impl DatasetBuilder for RegistryDatasetBuilder {
    async fn build(&self, m: &DatasetBuildManifest) -> Result<DatasetBuildOutcome, BuildFailure> {
        let req: DatasetRequest = serde_json::from_value(m.request.clone())
            .map_err(|e| BuildFailure::Invalid(format!("request is not a DatasetRequest: {e}")))?;
        let manager = DatasetManager::new(self.pg.clone());
        let plan = manager.plan(&m.tenant, req).await.map_err(classify)?;
        let rec = manager.materialize(&plan).await.map_err(classify)?;
        let (leakage,): (Option<serde_json::Value>,) = sqlx::query_as(
            "SELECT leakage_report FROM dataset_versions WHERE dataset_version_id = $1",
        )
        .bind(rec.dataset_version_id)
        .fetch_one(&self.pg)
        .await
        .map_err(|e| BuildFailure::Other(e.into()))?;
        Ok(DatasetBuildOutcome {
            dataset_id: rec.dataset_id,
            row_count: rec.row_count,
            leakage: leakage.unwrap_or(serde_json::Value::Null),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_leakage_refusal_is_terminal_a_pg_error_is_retried_anything_else_is_invalid() {
        let leak: anyhow::Error = LeakageRefused {
            dataset_id: "sha256:x".into(),
            failures: vec!["label_fold_overlap: rows 1..2".into()],
        }
        .into();
        assert!(matches!(
            classify(leak),
            BuildFailure::Leakage { ref failures, .. } if failures[0].starts_with("label_fold_overlap")
        ));
        assert!(matches!(
            classify(sqlx::Error::PoolTimedOut.into()),
            BuildFailure::Other(_)
        ));
        assert!(matches!(
            classify(anyhow::anyhow!("unknown feature set: nope")),
            BuildFailure::Invalid(_)
        ));
    }

    #[tokio::test]
    async fn the_worker_serves_the_dataset_build_kind() {
        use jobs::Worker;
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/none")
            .expect("lazy pool");
        assert_eq!(worker(pool).kind(), jobs::JobKind::DatasetBuild);
    }
}
