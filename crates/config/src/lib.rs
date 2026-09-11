pub mod model;
pub mod secrets;

pub use model::Config;

use std::sync::OnceLock;

use config::{Config as LibConfig, ConfigError, Environment, File};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("config error: {0}")]
    Load(#[from] ConfigError),
}

/// Records where `.env` was loaded from, for start-up logging.
static DOTENV_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Loads `.env` into the process environment, once per process.
///
/// Variables already present in the real environment win — `.env` only fills
/// gaps — so an explicit `CRED_KEK=... cargo run` still overrides the file.
/// A missing `.env` is not an error: deployed environments inject real env
/// vars instead.
fn load_dotenv() -> &'static Option<String> {
    DOTENV_PATH.get_or_init(|| match dotenvy::dotenv() {
        Ok(path) => Some(path.display().to_string()),
        Err(_) => None,
    })
}

/// Where `.env` was loaded from, or `None` if there was no `.env` to load.
///
/// Only meaningful after [`load`] has run. Exposed so binaries can log it
/// once tracing is up: a silently ignored `.env` is exactly the failure this
/// module previously had, where `CRED_KEK` sat correctly in the file while
/// the platform reported it unset and disabled credential storage.
pub fn dotenv_path() -> Option<&'static str> {
    DOTENV_PATH.get().and_then(Option::as_deref)
}

/// Load the layered configuration:
/// `.env` → `config/default.toml` → `config/local.toml` → `APP__*` env vars
/// → secrets env vars.
///
/// `.env` is loaded first so that the two env-var layers below actually see
/// what the file declares.
pub fn load() -> Result<Config, ConfigLoadError> {
    load_dotenv();
    let raw = LibConfig::builder()
        .add_source(File::with_name("config/default").required(false))
        .add_source(File::with_name("config/local").required(false))
        .add_source(Environment::with_prefix("APP").separator("__"))
        .build()?;
    let mut cfg: Config = raw.try_deserialize()?;
    secrets::resolve_secrets(&mut cfg);
    Ok(cfg)
}
