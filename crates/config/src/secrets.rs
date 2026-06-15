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
    if let Ok(v) = std::env::var("SMTP_HOST") { config.email.smtp_host = v; }
    if let Ok(v) = std::env::var("SMTP_PORT") { if let Ok(p) = v.parse() { config.email.smtp_port = p; } }
    if let Ok(v) = std::env::var("SMTP_USER") { config.email.smtp_user = v; }
    if let Ok(v) = std::env::var("SMTP_PASSWORD") { config.email.smtp_password = v; }
    if let Ok(v) = std::env::var("SMTP_FROM") { config.email.from_address = v; }
}
