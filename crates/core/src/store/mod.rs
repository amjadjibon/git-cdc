//! Chunk storage: the `ChunkStore` trait, the local sharded `DiskStore`,
//! and the S3-compatible `S3Store` (server s3 backend + serverless CLI).

pub mod disk;
pub mod envelope;
pub mod opendal;
pub mod s3;

pub use disk::DiskStore;
pub use opendal::{OpendalConfig, OpendalStore};
pub use s3::{S3Config, S3Store, make_client};

use anyhow::Result;

/// Content-addressable chunk storage. Deliberately just has/put/get —
/// refcounting was dropped per PLAN-REVIEW; GC is mark-and-sweep.
pub trait ChunkStore {
    fn has(&self, hash: &blake3::Hash) -> bool;
    /// Verifies `blake3(data) == hash` before admitting — this is also the
    /// server's upload-poisoning guard.
    fn put(&self, hash: &blake3::Hash, data: &[u8]) -> Result<()>;
    fn get(&self, hash: &blake3::Hash) -> Result<Vec<u8>>;
}
