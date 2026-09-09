use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub clickhouse: ClickhouseConfig,
    #[serde(default)]
    pub nats: NatsConfig,
    #[serde(default)]
    pub redis: RedisConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EmailConfig {
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_user: String,
    pub smtp_password: String,
    pub from_address: String,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_user: String::new(),
            smtp_password: String::new(),
            from_address: "noreply@tradingbot.local".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgres://localhost/trading_bot".into(),
            max_connections: 20,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClickhouseConfig {
    pub url: String,
}

impl Default for ClickhouseConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8123".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NatsConfig {
    pub url: String,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: "nats://localhost:4222".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RedisConfig {
    pub url: String,
}

impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            url: "redis://localhost:6379".into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
    pub host: String,
    pub port: u16,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 8080,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ObservabilityConfig {
    pub log_level: String,
    pub json_logs: bool,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            log_level: "info".into(),
            json_logs: false,
        }
    }
}

/// Internal agent (LLM-driven strategy design + backtest loop).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentConfig {
    /// Concurrent agent runs allowed per platform process.
    pub max_concurrent_runs: usize,
    /// Default LLM↔tool iterations before a run fails on budget.
    pub default_max_iterations: i32,
    /// Default wall-clock budget per run (covers long backtest waits).
    pub default_wallclock_budget_secs: i64,
    /// max_tokens sent on each individual LLM call.
    pub llm_max_tokens_per_call: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_concurrent_runs: 2,
            default_max_iterations: 15,
            default_wallclock_budget_secs: 14_400,
            llm_max_tokens_per_call: 4096,
        }
    }
}
