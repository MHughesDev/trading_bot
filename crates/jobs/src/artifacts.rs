//! Content-addressed artifact registry (COMP-005 §8, JB-09).
//!
//! Bytes live in `storage::artifacts::ArtifactStore` (filesystem in dev, S3/MinIO in
//! prod); this module owns the Postgres row that gives them a handle, a manifest, a
//! project scope and a lifetime.
//!
//! The point of handles is that large outputs never travel through an agent's
//! context. A 200 MB parquet extract is `art_9f2c…` plus a manifest saying what is in
//! it; the agent passes the handle to the next job, and reads bytes only if it
//! actually needs them.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::manifest::{artifact_handle, sha256_hex};
use crate::store::JobStoreError;

/// Default lifetime of an unpinned, cheap-to-recreate artifact (COMP-005 §8.3).
pub const DEFAULT_TTL_DAYS: i64 = 30;

/// Artifact types (COMP-005 §8.1).
pub const ARTIFACT_TYPES: &[&str] = &[
    "parquet_extract",
    "dataset",
    "feature_matrix",
    "prediction_series",
    "model_bundle",
    "run_outputs",
    "study_result",
    "path_set",
    "process",
    "chart_spec",
    "report",
    "dossier",
    "skill_bundle",
    "code_snapshot",
    "log",
    "exploration_summary",
    // A snapshot of NOTEBOOK.md / RESEARCH_PLAN.json, pushed by the agent-host at
    // each checkpoint (COMP-006 §3, UI-04).
    //
    // The workspace is a Docker volume the API deliberately cannot reach: giving the
    // API a docker socket to read the agent's files would hand it exactly the
    // capability `only_the_workspace_volume_is_mounted` exists to deny the agent.
    // Snapshotting through the artifact store instead keeps the file history
    // content-addressed and citable, which is what the Notebook pane wants anyway -
    // "what did the plan say when this verdict was reached" is an artifact handle,
    // not a file read.
    "workspace_snapshot",
    // A resumable training checkpoint. Its manifest is checked against the
    // §9 contract before the bytes are stored (`checkpoint::CheckpointManifest`),
    // because a checkpoint that cannot be resumed from is worse than no
    // checkpoint: it looks like insurance.
    "checkpoint",
];

/// Types that expire when nothing cites them. Everything else is kept: a report or a
/// model bundle is expensive or impossible to recreate, whereas a parquet extract is
/// a query away.
const EXPIRING_TYPES: &[&str] = &["parquet_extract", "log"];

#[derive(Debug, Clone)]
pub struct Artifact {
    pub handle: String,
    pub sha256: String,
    pub artifact_type: String,
    pub project_id: Option<Uuid>,
    pub uri: String,
    pub size_bytes: i64,
    pub manifest: Value,
    pub producer_job: Option<String>,
    pub pinned: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub struct ArtifactRegistry {
    pool: PgPool,
    store: std::sync::Arc<dyn storage::artifacts::ArtifactStore>,
}

impl ArtifactRegistry {
    pub fn new(pool: PgPool, store: std::sync::Arc<dyn storage::artifacts::ArtifactStore>) -> Self {
        Self { pool, store }
    }

    /// Stores bytes and registers the artifact.
    ///
    /// Content-addressed, so storing identical bytes twice yields the same handle and
    /// the second call is a no-op. That is what makes a re-run cheap: the same job
    /// producing the same output does not duplicate a gigabyte.
    pub async fn put(
        &self,
        artifact_type: &str,
        project_id: Option<Uuid>,
        bytes: &[u8],
        manifest: Value,
        producer_job: Option<&str>,
    ) -> Result<Artifact, JobStoreError> {
        if !ARTIFACT_TYPES.contains(&artifact_type) {
            return Err(JobStoreError::Invalid(format!(
                "unknown artifact type {artifact_type:?}"
            )));
        }

        // A checkpoint's manifest is the contract; the bytes are only useful
        // through it. Refusing here is the one moment a partial checkpoint can
        // be caught — afterwards the run resumes, the curve looks plausible, and
        // the divergence announces itself to nobody (ADR-P2-07, AT-61).
        if artifact_type == "checkpoint" {
            crate::checkpoint::CheckpointManifest::parse(&manifest).map_err(|e| {
                JobStoreError::Invalid(format!("checkpoint manifest refused: {e}"))
            })?;
        }

        let sha = sha256_hex(bytes);
        let handle = artifact_handle(&sha);

        if let Some(existing) = self.get(&handle).await? {
            return Ok(existing);
        }

        // Key by content hash, sharded on the first byte so no directory grows
        // unbounded.
        let key = format!("sha256/{}/{}", &sha[..2], sha);
        let stored = self
            .store
            .put_blocking(&key, bytes)
            .map_err(|e| JobStoreError::Invalid(format!("artifact store: {e}")))?;

        let expires_at = EXPIRING_TYPES
            .contains(&artifact_type)
            .then(|| Utc::now() + Duration::days(DEFAULT_TTL_DAYS));

        sqlx::query(
            "INSERT INTO artifacts (handle, sha256, type, project_id, uri, size_bytes, xxh3, \
                                    manifest, producer_job, expires_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) ON CONFLICT (handle) DO NOTHING",
        )
        .bind(&handle)
        .bind(&sha)
        .bind(artifact_type)
        .bind(project_id)
        .bind(&stored.uri)
        .bind(stored.size_bytes as i64)
        .bind(&stored.content_hash)
        .bind(&manifest)
        .bind(producer_job)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;

        self.get(&handle)
            .await?
            .ok_or_else(|| JobStoreError::NotFound(handle))
    }

    pub async fn get(&self, handle: &str) -> Result<Option<Artifact>, JobStoreError> {
        let row = sqlx::query("SELECT * FROM artifacts WHERE handle=$1")
            .bind(handle)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| Artifact {
            handle: r.get("handle"),
            sha256: r.get("sha256"),
            artifact_type: r.get("type"),
            project_id: r.get("project_id"),
            uri: r.get("uri"),
            size_bytes: r.get("size_bytes"),
            manifest: r.get("manifest"),
            producer_job: r.get("producer_job"),
            pinned: r.get("pinned"),
            expires_at: r.get("expires_at"),
            created_at: r.get("created_at"),
        }))
    }

    /// Reads an artifact's bytes, enforcing project scope (COMP-005 §8.3).
    ///
    /// A global artifact (`project_id IS NULL`) is readable by anyone; a
    /// project-scoped one only within its project. Cross-project reads are reported
    /// as `not_found` rather than `forbidden`, so a caller cannot use the error to
    /// discover that another project's artifact exists.
    pub async fn read(
        &self,
        handle: &str,
        requesting_project: Option<Uuid>,
    ) -> Result<Vec<u8>, JobStoreError> {
        let artifact = self
            .get(handle)
            .await?
            .ok_or_else(|| JobStoreError::NotFound(handle.to_string()))?;

        if let Some(owner) = artifact.project_id {
            if requesting_project != Some(owner) {
                return Err(JobStoreError::NotFound(handle.to_string()));
            }
        }

        self.store
            .get_blocking(&artifact.uri)
            .map_err(|e| JobStoreError::Invalid(format!("artifact read: {e}")))
    }

    /// Records that something cites this artifact, which pins it (COMP-005 §8.3).
    pub async fn pin(
        &self,
        handle: &str,
        ref_kind: &str,
        ref_id: &str,
    ) -> Result<(), JobStoreError> {
        sqlx::query(
            "INSERT INTO artifact_refs (handle, ref_kind, ref_id) VALUES ($1,$2,$3) \
             ON CONFLICT DO NOTHING",
        )
        .bind(handle)
        .bind(ref_kind)
        .bind(ref_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn unpin(
        &self,
        handle: &str,
        ref_kind: &str,
        ref_id: &str,
    ) -> Result<(), JobStoreError> {
        sqlx::query("DELETE FROM artifact_refs WHERE handle=$1 AND ref_kind=$2 AND ref_id=$3")
            .bind(handle)
            .bind(ref_kind)
            .bind(ref_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes expired, unpinned artifact rows and returns how many went.
    ///
    /// Only the rows: the blobs are content-addressed and may be shared by other
    /// rows, so sweeping bytes needs a separate mark-and-sweep that is not worth
    /// building until storage actually hurts.
    pub async fn expire(&self) -> Result<u64, JobStoreError> {
        let result = sqlx::query(
            "DELETE FROM artifacts WHERE pinned = false AND expires_at IS NOT NULL \
               AND expires_at < now()",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cheap_to_recreate_types_expire() {
        // A report or a model bundle that vanished after 30 days would take a
        // finding's evidence with it.
        for t in EXPIRING_TYPES {
            assert!(ARTIFACT_TYPES.contains(t));
        }
        assert!(!EXPIRING_TYPES.contains(&"report"));
        assert!(!EXPIRING_TYPES.contains(&"dossier"));
        assert!(!EXPIRING_TYPES.contains(&"model_bundle"));
        assert!(!EXPIRING_TYPES.contains(&"prediction_series"));
    }

    #[test]
    fn handles_are_deterministic_in_the_content() {
        let a = artifact_handle(&sha256_hex(b"same bytes"));
        let b = artifact_handle(&sha256_hex(b"same bytes"));
        assert_eq!(a, b);
        assert_ne!(a, artifact_handle(&sha256_hex(b"other bytes")));
    }
}
