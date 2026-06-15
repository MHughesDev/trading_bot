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
