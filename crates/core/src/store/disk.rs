use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::ChunkStore;

/// Sharded on-disk CAS: `<root>/<hex[0..2]>/<hex[2..4]>/<hex>`.
pub struct DiskStore {
    root: PathBuf,
}

impl DiskStore {
    pub fn new(root: impl Into<PathBuf>) -> DiskStore {
        DiskStore { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path_for(&self, hash: &blake3::Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.root
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(hex.as_str())
    }

    /// All chunk hashes currently in the store (for GC sweeps).
    pub fn list(&self) -> Result<Vec<blake3::Hash>> {
        let mut out = Vec::new();
        if !self.root.exists() {
            return Ok(out);
        }
        for shard1 in fs::read_dir(&self.root)? {
            let shard1 = shard1?.path();
            if !shard1.is_dir() {
                continue;
            }
            for shard2 in fs::read_dir(&shard1)? {
                let shard2 = shard2?.path();
                if !shard2.is_dir() {
                    continue;
                }
                for entry in fs::read_dir(&shard2)? {
                    let name = entry?.file_name();
                    if let Ok(hash) = blake3::Hash::from_hex(name.as_encoded_bytes()) {
                        out.push(hash);
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn remove(&self, hash: &blake3::Hash) -> Result<()> {
        fs::remove_file(self.path_for(hash)).context("removing chunk")
    }
}

impl ChunkStore for DiskStore {
    fn has(&self, hash: &blake3::Hash) -> bool {
        self.path_for(hash).is_file()
    }

    fn put(&self, hash: &blake3::Hash, data: &[u8]) -> Result<()> {
        if blake3::hash(data) != *hash {
            bail!("chunk data does not match hash {}", hash.to_hex());
        }
        self.write_object(hash, &super::envelope::encode(data))
    }

    fn get(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        super::envelope::decode(&self.get_encoded(hash)?, hash)
    }
}

impl DiskStore {
    /// Store an already-enveloped object after verifying it (server upload
    /// path: what arrived on the wire is written verbatim, no re-compress).
    pub fn put_encoded(&self, hash: &blake3::Hash, encoded: &[u8]) -> Result<()> {
        super::envelope::decode(encoded, hash)?;
        self.write_object(hash, encoded)
    }

    /// The enveloped bytes as stored (download path — decode is the
    /// receiver's job).
    pub fn get_encoded(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        fs::read(self.path_for(hash))
            .with_context(|| format!("chunk {} not in store", hash.to_hex()))
    }

    fn write_object(&self, hash: &blake3::Hash, bytes: &[u8]) -> Result<()> {
        let path = self.path_for(hash);
        if path.is_file() {
            return Ok(()); // content-addressed: already have identical bytes
        }
        let dir = path.parent().unwrap();
        fs::create_dir_all(dir).context("creating chunk shard dir")?;
        // Temp file + atomic rename: a killed process never leaves a
        // half-written chunk that has() reports present. Unique temp name so
        // concurrent double-puts of the same chunk don't clobber each other.
        let tmp = dir.join(format!(".tmp-{}-{}", std::process::id(), hash.to_hex()));
        fs::write(&tmp, bytes).context("writing chunk temp file")?;
        fs::rename(&tmp, &path).context("committing chunk")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, DiskStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskStore::new(dir.path().join("objects"));
        (dir, store)
    }

    #[test]
    fn put_get_has_round_trip() {
        let (_dir, s) = store();
        let data = b"some chunk content";
        let hash = blake3::hash(data);

        assert!(!s.has(&hash));
        s.put(&hash, data).unwrap();
        assert!(s.has(&hash));
        assert_eq!(s.get(&hash).unwrap(), data);
        // Idempotent double-put.
        s.put(&hash, data).unwrap();
        assert_eq!(s.list().unwrap(), vec![hash]);
    }

    #[test]
    fn put_rejects_corrupt_data() {
        let (_dir, s) = store();
        let hash = blake3::hash(b"expected content");
        assert!(s.put(&hash, b"different content").is_err());
        assert!(!s.has(&hash));
    }

    #[test]
    fn get_detects_on_disk_corruption() {
        let (_dir, s) = store();
        let data = b"chunk";
        let hash = blake3::hash(data);
        s.put(&hash, data).unwrap();
        fs::write(s.path_for(&hash), b"tampered").unwrap();
        assert!(s.get(&hash).is_err());
    }

    #[test]
    fn list_skips_foreign_and_temp_files() {
        let (_dir, s) = store();
        let data = b"real chunk";
        let hash = blake3::hash(data);
        s.put(&hash, data).unwrap();
        // A crashed put's leftover temp file and a stray file must not be
        // reported as chunks (their names aren't valid blake3 hex).
        let shard = s.path_for(&hash).parent().unwrap().to_path_buf();
        fs::write(shard.join(".tmp-999-deadbeef"), b"partial").unwrap();
        fs::write(shard.join("not-a-hash"), b"junk").unwrap();
        assert_eq!(s.list().unwrap(), vec![hash]);
    }

    #[test]
    fn legacy_bare_chunk_files_still_read() {
        // A store written before the envelope existed holds bare chunk
        // bytes; get() must keep serving them.
        let (_dir, s) = store();
        let data = b"pre-envelope chunk".to_vec();
        let hash = blake3::hash(&data);
        let path = s.path_for(&hash);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &data).unwrap();
        assert!(s.has(&hash));
        assert_eq!(s.get(&hash).unwrap(), data);
        // And the wire accepts legacy raw bodies too (old client, new server).
        let (_dir2, s2) = store();
        s2.put_encoded(&hash, &data).unwrap();
        assert_eq!(s2.get(&hash).unwrap(), data);
    }

    #[test]
    fn get_missing_chunk_errors() {
        let (_dir, s) = store();
        assert!(s.get(&blake3::hash(b"never stored")).is_err());
    }
}
