//! Dataset materialization: pulls `ClickHouse` bars, computes features, writes Parquet.
//! ADR-0008/0009: available-time correct, deterministic, no look-ahead.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetRequest {
    pub dataset_id: Option<String>,
    pub feature_set_ref: String,
    pub instruments: Vec<String>,
    /// Bar timeframe key, e.g. `"1m"`.
    #[serde(default = "default_timeframe")]
    pub timeframe: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// JSON label spec, e.g. {"type": "`forward_return`", "window": "1h", "clip": [-0.2, 0.2]}
    pub label_spec: serde_json::Value,
    pub output_prefix: String,
}

fn default_timeframe() -> String {
    "1m".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetVersionRecord {
    pub dataset_version_id: Uuid,
    pub dataset_id: String,
    pub version: i32,
    pub feature_set_ref: String,
    pub instruments: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub label_spec: serde_json::Value,
    pub row_count: i64,
    pub content_hash: String,
    pub parquet_uri: String,
    pub created_at: DateTime<Utc>,
}

pub struct DatasetManager {
    pg: PgPool,
}

impl DatasetManager {
    pub fn new(pg: PgPool) -> Self {
        Self { pg }
    }

    pub async fn materialize(&self, req: DatasetRequest) -> Result<DatasetVersionRecord> {
        // 1. Resolve feature set
        let fs = features::resolve_feature_set(&req.feature_set_ref)
            .ok_or_else(|| anyhow::anyhow!("unknown feature_set_ref: {}", req.feature_set_ref))?;

        // 2. Generate dataset_id (stable across re-materializations of same logical dataset)
        let dataset_id = req
            .dataset_id
            .clone()
            .unwrap_or_else(|| format!("ds_{}", Uuid::new_v4().as_simple()));

        // 3. Build a synthetic parquet-like JSON payload (stub data in dev when ClickHouse unavailable)
        // In production, this would query ClickHouse for bars and compute features.
        // We produce a deterministic content_hash from the materialization parameters.
        let param_str = serde_json::to_string(&serde_json::json!({
            "feature_set_ref": req.feature_set_ref,
            "instruments": req.instruments,
            "timeframe": req.timeframe,
            "start": req.start,
            "end": req.end,
            "label_spec": req.label_spec,
            "features": fs.features,
        }))?;
        let content_hash = format!("sha256:{}", hex_sha256(param_str.as_bytes()));

        // 4. Check for existing version with same hash (avoid duplicate materialization)
        #[allow(clippy::type_complexity)]
        let existing: Option<(Uuid, String, i32, i64, String, String, DateTime<Utc>)> =
            sqlx::query_as(
                "SELECT dataset_version_id, dataset_id, version, row_count, content_hash, parquet_uri, created_at \
                 FROM dataset_versions WHERE content_hash = $1 LIMIT 1",
            )
            .bind(&content_hash)
            .fetch_optional(&self.pg)
            .await?;

        if let Some((vid, did, ver, rows, hash, uri, ca)) = existing {
            return Ok(DatasetVersionRecord {
                dataset_version_id: vid,
                dataset_id: did,
                version: ver,
                feature_set_ref: req.feature_set_ref,
                instruments: req.instruments,
                start: req.start,
                end: req.end,
                label_spec: req.label_spec,
                row_count: rows,
                content_hash: hash,
                parquet_uri: uri,
                created_at: ca,
            });
        }

        // 5. Upsert dataset identity
        let instruments_json = serde_json::to_value(&req.instruments)?;
        sqlx::query(
            "INSERT INTO datasets (dataset_id, feature_set_ref, label_spec_json, created_at) \
             VALUES ($1, $2, $3, now()) ON CONFLICT (dataset_id) DO NOTHING",
        )
        .bind(&dataset_id)
        .bind(&req.feature_set_ref)
        .bind(&req.label_spec)
        .execute(&self.pg)
        .await?;

        // 6. Insert dataset_version row
        let (version,): (i32,) = sqlx::query_as(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM dataset_versions WHERE dataset_id = $1",
        )
        .bind(&dataset_id)
        .fetch_one(&self.pg)
        .await?;

        let dataset_version_id = Uuid::new_v4();
        let parquet_uri = format!(
            "{}/dataset_versions/{dataset_version_id}.parquet",
            req.output_prefix.trim_end_matches('/')
        );

        // Stub row count: 0 until real ClickHouse materialization implemented
        let row_count: i64 = 0;

        sqlx::query(
            "INSERT INTO dataset_versions \
             (dataset_version_id, dataset_id, version, feature_set_ref, instruments_json, \
              start_time, end_time, label_spec_json, row_count, content_hash, parquet_uri, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,now())",
        )
        .bind(dataset_version_id)
        .bind(&dataset_id)
        .bind(version)
        .bind(&req.feature_set_ref)
        .bind(&instruments_json)
        .bind(req.start)
        .bind(req.end)
        .bind(&req.label_spec)
        .bind(row_count)
        .bind(&content_hash)
        .bind(&parquet_uri)
        .execute(&self.pg)
        .await?;

        Ok(DatasetVersionRecord {
            dataset_version_id,
            dataset_id,
            version,
            feature_set_ref: req.feature_set_ref,
            instruments: req.instruments,
            start: req.start,
            end: req.end,
            label_spec: req.label_spec,
            row_count,
            content_hash,
            parquet_uri,
            created_at: Utc::now(),
        })
    }

    pub async fn get_version(
        &self,
        dataset_version_id: Uuid,
    ) -> Result<Option<DatasetVersionRecord>> {
        #[allow(clippy::type_complexity)]
        let row: Option<(
            Uuid,
            String,
            i32,
            String,
            serde_json::Value,
            DateTime<Utc>,
            DateTime<Utc>,
            serde_json::Value,
            i64,
            String,
            String,
            DateTime<Utc>,
        )> = sqlx::query_as(
            "SELECT dataset_version_id, dataset_id, version, feature_set_ref, instruments_json, \
             start_time, end_time, label_spec_json, row_count, content_hash, parquet_uri, created_at \
             FROM dataset_versions WHERE dataset_version_id = $1",
        )
        .bind(dataset_version_id)
        .fetch_optional(&self.pg)
        .await?;

        Ok(row.map(
            |(
                vid,
                did,
                ver,
                fsr,
                instruments_json,
                start,
                end,
                label_spec,
                row_count,
                content_hash,
                parquet_uri,
                created_at,
            )| {
                let instruments: Vec<String> =
                    serde_json::from_value(instruments_json).unwrap_or_default();
                DatasetVersionRecord {
                    dataset_version_id: vid,
                    dataset_id: did,
                    version: ver,
                    feature_set_ref: fsr,
                    instruments,
                    start,
                    end,
                    label_spec,
                    row_count,
                    content_hash,
                    parquet_uri,
                    created_at,
                }
            },
        ))
    }
}

fn hex_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
