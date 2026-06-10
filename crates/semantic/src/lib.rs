//! Milvus vector-store client — embedding pipeline and metadata-filtered search
//! (Phase 7).  Configured for `text-embedding-3-small` (1536 dimensions).
//!
//! Milvus REST v2 API runs on port 19530 (same port as gRPC in Milvus 2.3+).
//! Port 9091 is the legacy monitoring/metrics port and does NOT serve the REST
//! v2 API at `/v2/vectordb/...` (H-11).

pub mod collection;
pub mod embed;

use thiserror::Error;

/// Embedding model this client is configured for.
pub const EMBEDDING_MODEL: &str = "text-embedding-3-small";
/// Embedding dimension for `text-embedding-3-small`.
pub const EMBEDDING_DIMS: u32 = 1536;
/// Collection name for social/news text embeddings.
pub const SOCIAL_COLLECTION: &str = "trading_social_posts";

/// Default Milvus HTTP REST v2 port (Milvus 2.3+).
pub const DEFAULT_HTTP_PORT: u16 = 19530;

/// Errors from semantic/vector operations.
#[derive(Debug, Error)]
pub enum SemanticError {
    #[error("connection failed: {0}")]
    Connect(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("unexpected response: {0}")]
    Response(String),
    #[error("collection error: {0}")]
    Collection(String),
}

/// Configuration for a Milvus instance.
#[derive(Debug, Clone)]
pub struct MilvusConfig {
    pub host: String,
    pub http_port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl MilvusConfig {
    pub fn new(host: impl Into<String>, http_port: u16) -> Self {
        Self {
            host: host.into(),
            http_port,
            username: None,
            password: None,
        }
    }

    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self.password = Some(password.into());
        self
    }

    /// Build from environment variables.
    pub fn from_env() -> Self {
        let host = std::env::var("MILVUS_HOST").unwrap_or_else(|_| "localhost".into());
        let port = std::env::var("MILVUS_HTTP_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_HTTP_PORT);
        let mut cfg = Self::new(host, port);
        if let (Ok(user), Ok(pass)) = (
            std::env::var("MILVUS_USERNAME"),
            std::env::var("MILVUS_PASSWORD"),
        ) {
            cfg = cfg.with_auth(user, pass);
        }
        cfg
    }
}

/// Collection specification for social-post embeddings.
#[derive(Debug, Clone)]
pub struct CollectionSpec {
    pub name: String,
    pub dims: u32,
    pub embedding_model: String,
}

impl CollectionSpec {
    pub fn social_posts() -> Self {
        Self {
            name: SOCIAL_COLLECTION.to_owned(),
            dims: EMBEDDING_DIMS,
            embedding_model: EMBEDDING_MODEL.to_owned(),
        }
    }
}

/// Connected Milvus client.
pub struct MilvusClient {
    config: MilvusConfig,
    http: reqwest::Client,
    /// Configured collection spec.
    pub collection_spec: CollectionSpec,
}

impl MilvusClient {
    /// Connect to Milvus and return a ready client.
    pub async fn connect(
        config: MilvusConfig,
        collection_spec: CollectionSpec,
    ) -> Result<Self, SemanticError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| SemanticError::Connect(e.to_string()))?;

        Ok(Self {
            config,
            http,
            collection_spec,
        })
    }

    /// Return a request builder pre-configured with auth headers (M-4).
    ///
    /// Attaches `Authorization: Bearer user:password` when credentials are set,
    /// so all collection and embed paths use authenticated requests.
    pub(crate) fn authed_post(&self, url: &str) -> reqwest::RequestBuilder {
        let req = self.http.post(url);
        self.attach_auth(req)
    }

    pub(crate) fn authed_get(&self, url: &str) -> reqwest::RequestBuilder {
        let req = self.http.get(url);
        self.attach_auth(req)
    }

    fn attach_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let (Some(user), Some(pass)) = (&self.config.username, &self.config.password) {
            req.header(
                "Authorization",
                format!("Bearer {user}:{pass}"),
            )
        } else {
            req
        }
    }

    /// Ping the Milvus HTTP endpoint.  Returns `Ok(())` on 2xx.
    pub async fn ping(&self) -> Result<(), SemanticError> {
        let url = format!(
            "http://{}:{}/healthz",
            self.config.host, self.config.http_port
        );
        let resp = self
            .authed_get(&url)
            .send()
            .await
            .map_err(|e| SemanticError::Request(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(SemanticError::Response(resp.status().to_string()))
        }
    }
}

/// Connect to Milvus with the social-posts collection spec.
pub async fn connect(config: MilvusConfig) -> Result<MilvusClient, SemanticError> {
    MilvusClient::connect(config, CollectionSpec::social_posts()).await
}
