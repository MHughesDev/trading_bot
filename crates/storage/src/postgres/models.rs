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
         LEFT JOIN model_artifacts a ON a.artifact_id = v.artifact_id \
         ORDER BY v.model_id, v.version",
    )
    .fetch_all(pool)
    .await?)
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
