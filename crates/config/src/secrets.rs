use super::model::Config;

/// Override config values from well-known environment variables.
///
/// Environment variables take precedence over TOML files and are the only
/// supported mechanism for injecting secrets (no secrets in TOML).
pub fn resolve_secrets(config: &mut Config) {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        config.database.url = url;
    }
    if let Ok(url) = std::env::var("CLICKHOUSE_URL") {
        config.clickhouse.url = url;
    }
    if let Ok(url) = std::env::var("NATS_URL") {
        config.nats.url = url;
    }
    if let Ok(url) = std::env::var("REDIS_URL") {
        config.redis.url = url;
    }
}
