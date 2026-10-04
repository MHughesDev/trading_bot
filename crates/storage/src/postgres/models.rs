//! Model registry Postgres helpers.
//! The model-registry crate owns business logic; this module exposes
//! any shared sqlx utilities needed by other storage consumers.

// Re-exported for callers that need to check model existence without
// pulling in the full model-registry crate.
pub async fn model_exists(pool: &super::PgPool, model_id: &str) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM ai_models WHERE model_id = $1)")
        .bind(model_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(false)
}

// ── contracts on model versions (Set U U-5.1, migration 0146) ────────────────

/// One model version as the strategy compiler and model providers see it.
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct VersionContractRow {
    pub model_id: String,
    pub version: i32,
    pub io_contract: Option<serde_json::Value>,
    pub contract_source: Option<String>,
    pub trained_through: Option<chrono::DateTime<chrono::Utc>>,
    pub model_kind: String,
    pub artifact_uri: Option<String>,
    pub artifact_sha256: Option<String>,
    /// An LLM adapter's settings (`definition_json.adapter`), when the model
    /// is one.
    pub adapter: Option<serde_json::Value>,
    /// A transform's code (`definition_json.transform`), when the model is one.
    pub transform: Option<serde_json::Value>,
}

/// Every model version, with its contract (if any) and artifact.
///
/// The artifact is the version's `model` artifact — the one `artifact_id`
/// names when it is set, else the most recent. Training writes
/// `model_artifacts` by `(model_id, version)` and has never set
/// `model_versions.artifact_id`, so joining on that column (as this did until
/// 2026-10-04) left every trained version with no artifact: no backtest and no
/// paper deployment could ever be answered by a trained model.
///
/// # Errors
/// A database error.
pub async fn version_contracts(
    pool: &super::PgPool,
) -> Result<Vec<VersionContractRow>, super::PgError> {
    Ok(sqlx::query_as(
        "SELECT v.model_id, v.version, v.io_contract, v.contract_source, v.trained_through, \
                m.model_kind, a.storage_uri AS artifact_uri, a.sha256 AS artifact_sha256, \
                m.definition_json -> 'adapter' AS adapter, \
                m.definition_json -> 'transform' AS transform \
         FROM model_versions v \
         JOIN ai_models m ON m.model_id = v.model_id \
         LEFT JOIN LATERAL ( \
             SELECT storage_uri, sha256 FROM model_artifacts a \
              WHERE a.model_id = v.model_id AND a.version = v.version \
                AND a.artifact_type = 'model' \
              ORDER BY (a.artifact_id = v.artifact_id) DESC NULLS LAST, \
                       a.created_at DESC, a.artifact_id \
              LIMIT 1 \
         ) a ON true \
         ORDER BY v.model_id, v.version",
    )
    .fetch_all(pool)
    .await?)
}

/// One walk-forward fold's model (Set U U-5.8, migration 0189).
#[derive(Clone, Debug, sqlx::FromRow)]
pub struct FoldArtifactRow {
    pub model_id: String,
    pub version: i32,
    pub fold_index: i32,
    pub trained_through: chrono::DateTime<chrono::Utc>,
    pub storage_uri: String,
    pub sha256: String,
}

/// Every version's fold models, earliest training end first.
///
/// # Errors
/// A database error.
pub async fn fold_artifacts(pool: &super::PgPool) -> Result<Vec<FoldArtifactRow>, super::PgError> {
    Ok(sqlx::query_as(
        "SELECT model_id, version, fold_index, trained_through, storage_uri, sha256 \
         FROM model_artifacts WHERE artifact_type = 'model_fold' \
         ORDER BY model_id, version, trained_through, fold_index",
    )
    .fetch_all(pool)
    .await?)
}

/// Record one fold's model: trained only on what came before its test window,
/// so a backtest may ask it about the decisions inside that window. Written
/// once per (model, version, fold); a repeat is a no-op.
///
/// # Errors
/// A database error.
#[allow(clippy::too_many_arguments)]
pub async fn record_fold_artifact(
    pool: &super::PgPool,
    model_id: &str,
    version: i32,
    fold_index: i32,
    trained_through: chrono::DateTime<chrono::Utc>,
    storage_uri: &str,
    size_bytes: i64,
    sha256: &str,
) -> Result<(), super::PgError> {
    sqlx::query(
        "INSERT INTO model_artifacts \
           (model_id, version, storage_uri, artifact_type, size_bytes, sha256, fold_index, trained_through) \
         VALUES ($1, $2, $3, 'model_fold', $4, $5, $6, $7) \
         ON CONFLICT (model_id, version, fold_index) WHERE artifact_type = 'model_fold' DO NOTHING",
    )
    .bind(model_id)
    .bind(version)
    .bind(storage_uri)
    .bind(size_bytes)
    .bind(sha256)
    .bind(fold_index)
    .bind(trained_through)
    .execute(pool)
    .await?;
    Ok(())
}

/// Every alias and the version it points at now. The compiler resolves an
/// alias to a version once; a running strategy keeps that version.
///
/// # Errors
/// A database error.
pub async fn model_aliases(
    pool: &super::PgPool,
) -> Result<Vec<(String, String, i32)>, super::PgError> {
    Ok(
        sqlx::query_as("SELECT model_id, alias, version FROM model_aliases")
            .fetch_all(pool)
            .await?,
    )
}

/// Versions that have no contract yet, with what a contract is derived from:
/// the model's definition and the version's training metrics (which carry the
/// bundle's `feature_order` when the trainer reported it).
///
/// # Errors
/// A database error.
pub async fn versions_without_contract(
    pool: &super::PgPool,
) -> Result<Vec<(String, i32, serde_json::Value, Option<serde_json::Value>)>, super::PgError> {
    Ok(sqlx::query_as(
        "SELECT v.model_id, v.version, m.definition_json, v.metrics_json \
         FROM model_versions v JOIN ai_models m ON m.model_id = v.model_id \
         WHERE v.io_contract IS NULL ORDER BY v.model_id, v.version",
    )
    .fetch_all(pool)
    .await?)
}

/// Record a version's contract. A derived contract never replaces a declared
/// one; a declared one replaces either.
///
/// # Errors
/// A database error.
pub async fn set_version_contract(
    pool: &super::PgPool,
    model_id: &str,
    version: i32,
    contract: &serde_json::Value,
    declared: bool,
) -> Result<bool, super::PgError> {
    let sql = if declared {
        "UPDATE model_versions SET io_contract = $3, contract_source = 'declared' \
         WHERE model_id = $1 AND version = $2"
    } else {
        "UPDATE model_versions SET io_contract = $3, contract_source = 'derived' \
         WHERE model_id = $1 AND version = $2 AND io_contract IS NULL"
    };
    let done = sqlx::query(sql)
        .bind(model_id)
        .bind(version)
        .bind(contract)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() == 1)
}

/// Record how far a version's training data reached. Set once, when the
/// training run reports it; never inferred afterwards.
///
/// # Errors
/// A database error.
pub async fn set_trained_through(
    pool: &super::PgPool,
    model_id: &str,
    version: i32,
    trained_through: chrono::DateTime<chrono::Utc>,
) -> Result<(), super::PgError> {
    sqlx::query(
        "UPDATE model_versions SET trained_through = $3 \
         WHERE model_id = $1 AND version = $2 AND trained_through IS NULL",
    )
    .bind(model_id)
    .bind(version)
    .bind(trained_through)
    .execute(pool)
    .await?;
    Ok(())
}
