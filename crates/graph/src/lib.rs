//! TigerGraph client — capability/compatibility graph (Phase 7).
//!
//! Provides `connect()`, `ping()`, schema initialization, and graph population.
//!
//! Port assignments (standard TigerGraph 3.x deployment):
//! * REST++ data API   — port 9000  (`restpp_port`)
//! * GSQL admin API    — port 14240 (`gsql_port`)

pub mod populate;
pub mod schema;

use thiserror::Error;

/// Default TigerGraph REST++ port (data operations: upsert, delete, query).
pub const DEFAULT_RESTPP_PORT: u16 = 9000;
/// Default TigerGraph GSQL port (schema DDL and introspection).
pub const DEFAULT_GSQL_PORT: u16 = 14240;

/// Errors from graph operations.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("unexpected response: {0}")]
    Response(String),
}

/// Configuration for a TigerGraph instance.
///
/// Two ports are needed because TigerGraph separates its REST++ data API
/// (default 9000) from its GSQL admin API (default 14240).
#[derive(Debug, Clone)]
pub struct TigerGraphConfig {
    pub host: String,
    /// REST++ API port — used for vertex/edge upsert and delete (default 9000).
    pub restpp_port: u16,
    /// GSQL API port — used for schema DDL and introspection (default 14240).
    pub gsql_port: u16,
    pub username: String,
    pub password: String,
    pub graph_name: String,
}

impl TigerGraphConfig {
    pub fn new(
        host: impl Into<String>,
        restpp_port: u16,
        gsql_port: u16,
        username: impl Into<String>,
        password: impl Into<String>,
        graph_name: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            restpp_port,
            gsql_port,
            username: username.into(),
            password: password.into(),
            graph_name: graph_name.into(),
        }
    }

    /// Build from environment variables.
    ///
    /// `TIGERGRAPH_PORT`      → REST++ port (default 9000)
    /// `TIGERGRAPH_GSQL_PORT` → GSQL port (default 14240)
    pub fn from_env() -> Self {
        Self::new(
            std::env::var("TIGERGRAPH_HOST").unwrap_or_else(|_| "localhost".into()),
            std::env::var("TIGERGRAPH_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_RESTPP_PORT),
            std::env::var("TIGERGRAPH_GSQL_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_GSQL_PORT),
            std::env::var("TIGERGRAPH_USER").unwrap_or_else(|_| "tigergraph".into()),
            std::env::var("TIGERGRAPH_PASSWORD").unwrap_or_else(|_| "tigergraph".into()),
            std::env::var("TIGERGRAPH_GRAPH").unwrap_or_else(|_| "trading".into()),
        )
    }
}

/// Connected TigerGraph client.
pub struct TigerGraphClient {
    config: TigerGraphConfig,
    http: reqwest::Client,
}

impl TigerGraphClient {
    /// Establish a connection (creates the HTTP client; no persistent connection).
    pub async fn connect(config: TigerGraphConfig) -> Result<Self, GraphError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| GraphError::Connect(e.to_string()))?;

        Ok(Self { config, http })
    }

    /// Ping the TigerGraph instance.  Returns `Ok(())` on 2xx.
    pub async fn ping(&self) -> Result<(), GraphError> {
        let url = format!(
            "http://{}:{}/api/ping",
            self.config.host, self.config.restpp_port
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| GraphError::Request(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(GraphError::Response(resp.status().to_string()))
        }
    }
}

/// Connect to TigerGraph and return a ready client.
pub async fn connect(config: TigerGraphConfig) -> Result<TigerGraphClient, GraphError> {
    TigerGraphClient::connect(config).await
}
