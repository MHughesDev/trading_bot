//! Local-filesystem artifact backend. For development; no cloud deps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use xxhash_rust::xxh3::xxh3_64;

use super::{ArtifactRef, ArtifactStore};

pub struct FsArtifactStore {
    root: PathBuf,
}

impl FsArtifactStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self { root: root.as_ref().to_path_buf() }
    }
}

impl ArtifactStore for FsArtifactStore {
    fn put_blocking(&self, key: &str, bytes: &[u8]) -> anyhow::Result<ArtifactRef> {
        let sanitized = key.replace(['/', '\\', ':'], "_");
        let path = self.root.join(&sanitized);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)?;
        let hash = format!("{:016x}", xxh3_64(bytes));
        let uri = format!("file://{}", path.display());
        Ok(ArtifactRef {
            uri,
            content_hash: hash,
            size_bytes: bytes.len() as u64,
        })
    }

    fn get_blocking(&self, uri: &str) -> anyhow::Result<Vec<u8>> {
        let path = uri
            .strip_prefix("file://")
            .ok_or_else(|| anyhow::anyhow!("FsArtifactStore: expected file:// URI, got {uri}"))?;
        Ok(std::fs::read(path)?)
    }

    fn presign_url(&self, uri: &str, _ttl: Duration) -> anyhow::Result<String> {
        Ok(uri.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn round_trips_bytes() {
        let dir = env::temp_dir().join("artifact_store_test");
        let store = FsArtifactStore::new(&dir);
        let data = b"hello artifact store";
        let r = store.put_blocking("test/hello.bin", data).unwrap();
        assert!(!r.content_hash.is_empty());
        assert_eq!(r.size_bytes, data.len() as u64);
        let back = store.get_blocking(&r.uri).unwrap();
        assert_eq!(back, data);
        let r2 = store.put_blocking("test/hello.bin", data).unwrap();
        assert_eq!(r.content_hash, r2.content_hash);
    }
}
