pub mod model;
pub mod secrets;

pub use model::Config;

use config::{Config as LibConfig, ConfigError, Environment, File};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("config error: {0}")]
    Load(#[from] ConfigError),
}

/// Load the layered configuration:
/// `config/default.toml` → `config/local.toml` → `APP__*` env vars → secrets env vars.
pub fn load() -> Result<Config, ConfigLoadError> {
    let raw = LibConfig::builder()
        .add_source(File::with_name("config/default").required(false))
        .add_source(File::with_name("config/local").required(false))
        .add_source(Environment::with_prefix("APP").separator("__"))
        .build()?;
    let mut cfg: Config = raw.try_deserialize()?;
    secrets::resolve_secrets(&mut cfg);
    Ok(cfg)
}
