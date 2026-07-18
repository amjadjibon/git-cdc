use std::time::SystemTime;

use anyhow::Result;
use git_cdc_core::store::s3::S3Store;
use git_cdc_core::store::{ChunkStore, DiskStore};

/// Server-side chunk storage. Two variants with one call site each — an
/// enum, not a trait object (ponytail: async_trait machinery buys nothing
/// at this scale; add it if a third backend ever appears).
pub enum Backend {
    Disk(DiskStore),
    S3(S3Store),
}

impl Backend {
    pub async fn has(&self, hash: &blake3::Hash) -> Result<bool> {
        match self {
            Backend::Disk(s) => Ok(s.has(hash)),
            Backend::S3(s) => s.has(hash).await,
        }
    }

    /// Verifies `blake3(data) == hash` before admitting (both variants).
    pub async fn put(&self, hash: &blake3::Hash, data: &[u8]) -> Result<()> {
        match self {
            Backend::Disk(s) => s.put(hash, data),
            Backend::S3(s) => s.put(hash, data).await,
        }
    }

    pub async fn get(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        match self {
            Backend::Disk(s) => s.get(hash),
            Backend::S3(s) => s.get(hash).await,
        }
    }

    pub async fn remove(&self, hash: &blake3::Hash) -> Result<()> {
        match self {
            Backend::Disk(s) => s.remove(hash),
            Backend::S3(s) => s.remove(hash).await,
        }
    }

    /// All chunks with their last-modified time (disk mtime / S3
    /// LastModified) — GC's grace period needs the age.
    pub async fn list(&self) -> Result<Vec<(blake3::Hash, Option<SystemTime>)>> {
        match self {
            Backend::Disk(s) => Ok(s
                .list()?
                .into_iter()
                .map(|h| {
                    let mtime = std::fs::metadata(s.path_for(&h))
                        .and_then(|m| m.modified())
                        .ok();
                    (h, mtime)
                })
                .collect()),
            Backend::S3(s) => s.list().await,
        }
    }
}
