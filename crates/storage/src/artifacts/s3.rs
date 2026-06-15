//! S3/MinIO artifact backend stub.
//! Compiles in all configurations; panics at runtime if called without
//! real S3 credentials (`ARTIFACT_STORE=s3` in production).

use std::time::Duration;

use super::{ArtifactRef, ArtifactStore};

pub struct S3ArtifactStore {
    bucket: String,
    prefix: String,
}

impl S3ArtifactStore {
    pub fn from_env() -> Self {
        Self {
            bucket: std::env::var("ARTIFACT_S3_BUCKET").unwrap_or_else(|_| "artifacts".into()),
            prefix: std::env::var("ARTIFACT_S3_PREFIX").unwrap_or_else(|_| "models/".into()),
        }
    }
}

impl ArtifactStore for S3ArtifactStore {
    fn put_blocking(&self, key: &str, _bytes: &[u8]) -> anyhow::Result<ArtifactRef> {
        anyhow::bail!(
            "S3ArtifactStore is not yet fully implemented. \
             Bucket={}, prefix={}, key={key}",
            self.bucket,
            self.prefix
        )
    }

    fn get_blocking(&self, uri: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("S3ArtifactStore is not yet fully implemented. URI={uri}")
    }

    fn presign_url(&self, uri: &str, _ttl: Duration) -> anyhow::Result<String> {
        anyhow::bail!("S3ArtifactStore is not yet fully implemented. URI={uri}")
    }
}
