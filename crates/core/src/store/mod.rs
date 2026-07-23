//! Chunk storage: the `ChunkStore` trait, the local sharded `DiskStore`,
//! and the OpenDAL-backed `OpendalStore` (server s3/opendal backends +
//! serverless CLI).

pub mod disk;
pub mod envelope;
mod opendal;

pub use disk::DiskStore;
pub use opendal::{OpendalConfig, OpendalStore};

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
