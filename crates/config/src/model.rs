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
    #[serde(default)]
    pub jobs: JobsConfig,
    /// Instruments whose daily returns define a market scope for the regime
    /// labeller (SPEC §5.4, ADR-P3-02).
    ///
    /// Empty by default and the job does not run. There is no sensible platform
    /// default here: "the market" means a different series for crypto, equities
    /// and futures, and picking one would produce regime labels that look
    /// authoritative and describe something the strategy does not trade.
    #[serde(default)]
    pub regime_scopes: Vec<String>,
}

/// Worker-pool sizing for the durable job service (COMP-005 §7).
///
/// These replace the fixed 3-concurrent backtest semaphore, which lived inside the
/// backtest manager where no operator could see or change it. Defaults are modest
/// because the dev box runs the whole stack; a real deployment raises them.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JobsConfig {
    pub backtest_parallel: usize,
    pub research_parallel: usize,
    pub data_parallel: usize,
    pub eval_parallel: usize,
    pub trainer_parallel: usize,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            backtest_parallel: 4,
            research_parallel: 2,
            data_parallel: 2,
            eval_parallel: 1,
            trainer_parallel: 1,
        }
    }
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
    /// Directory of capability profiles (ADR-0031, harness guide §1.2).
    ///
    /// Loaded at startup. Every constraint in a profile is enforced by harness code;
    /// a missing directory is a hard failure rather than a fallback to defaults,
    /// because "defaults" would mean an unprofiled model running at whatever the
    /// code happens to allow.
    #[serde(default = "default_profiles_dir")]
    pub profiles_dir: String,
    /// The profile a research session runs under unless a project pins another.
    #[serde(default = "default_profile")]
    pub default_profile: String,
}

fn default_profiles_dir() -> String {
    "config/profiles".to_string()
}

fn default_profile() -> String {
    "claude-opus-5".to_string()
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_concurrent_runs: 2,
            default_max_iterations: 15,
            default_wallclock_budget_secs: 14_400,
            llm_max_tokens_per_call: 4096,
            profiles_dir: default_profiles_dir(),
            default_profile: default_profile(),
        }
    }
}
