//! Artifact object-store abstraction — local-FS for dev, S3/MinIO in prod.
//! Backend selected by `ARTIFACT_STORE=fs|s3` (default: fs).

pub mod fs;
pub mod s3;

use std::time::Duration;

/// A stored artifact reference returned by `put`.
#[derive(Debug, Clone)]
pub struct ArtifactRef {
    /// Canonical URI: `file:///…` or `s3://bucket/key`.
    pub uri: String,
    /// Hex-encoded xxh3 content hash for integrity checks.
    pub content_hash: String,
    pub size_bytes: u64,
}

/// Artifact storage backend (local-FS or S3/MinIO).
/// Callers only see this trait; the concrete backend is selected at startup.
pub trait ArtifactStore: Send + Sync {
    fn put_blocking(&self, key: &str, bytes: &[u8]) -> anyhow::Result<ArtifactRef>;
    fn get_blocking(&self, uri: &str) -> anyhow::Result<Vec<u8>>;
    /// Returns a pre-signed URL valid for `ttl` (S3) or a `file://` URI (FS).
    fn presign_url(&self, uri: &str, ttl: Duration) -> anyhow::Result<String>;
}

/// Build the artifact store from the `ARTIFACT_STORE` environment variable.
/// Defaults to `FsArtifactStore` at `./artifacts/`.
pub fn from_env() -> Box<dyn ArtifactStore> {
    match std::env::var("ARTIFACT_STORE").as_deref() {
        Ok("s3") => Box::new(s3::S3ArtifactStore::from_env()),
        _ => Box::new(fs::FsArtifactStore::new("./artifacts")),
    }
}
