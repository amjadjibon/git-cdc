use std::time::SystemTime;

use anyhow::Result;
use git_cdc_core::store::{ChunkStore, DiskStore, OpendalStore};

/// Server-side chunk storage. An enum, not a trait object — two variants
/// with one call site each doesn't justify async_trait machinery.
pub enum Backend {
    Disk(DiskStore),
    /// Everything remote: s3, azblob, gcs, sftp, ftp, gdrive, webdav,
    /// onedrive (the s3 flags map onto this via `OpendalConfig::s3`).
    Opendal(OpendalStore),
}

impl Backend {
    pub async fn has(&self, hash: &blake3::Hash) -> Result<bool> {
        match self {
            Backend::Disk(s) => Ok(s.has(hash)),
            Backend::Opendal(s) => s.has(hash).await,
        }
    }

    /// Store the enveloped bytes exactly as uploaded, after verification
    /// (the wire and storage formats are both the envelope — no
    /// re-compression server-side).
    pub async fn put_encoded(&self, hash: &blake3::Hash, encoded: Vec<u8>) -> Result<()> {
        match self {
            Backend::Disk(s) => s.put_encoded(hash, &encoded),
            Backend::Opendal(s) => s.put_encoded(hash, encoded).await,
        }
    }

    /// The enveloped bytes as stored (what downloads serve).
    pub async fn get_encoded(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        match self {
            Backend::Disk(s) => s.get_encoded(hash),
            Backend::Opendal(s) => s.get_encoded(hash).await,
        }
    }

    pub async fn remove(&self, hash: &blake3::Hash) -> Result<()> {
        match self {
            Backend::Disk(s) => s.remove(hash),
            Backend::Opendal(s) => s.remove(hash).await,
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
            Backend::Opendal(s) => s.list().await,
        }
    }
}
