use std::time::SystemTime;

use anyhow::{Context, Result, bail};

/// OpenDAL connection settings: a service scheme (`azblob`, `gcs`, `sftp`,
/// `ftp`, `gdrive`, `webdav`, `onedrive`, `fs`, ...) plus its key=value
/// options, passed straight through to `Operator::via_iter`.
#[derive(Debug, Clone, Default)]
pub struct OpendalConfig {
    pub scheme: String,
    pub options: Vec<(String, String)>,
    /// Directory chunks live under; normalized to end with `/`.
    pub prefix: String,
}

/// Chunk store over any OpenDAL service. Same API shape as `S3Store`:
/// keys are `{prefix}{hex}`, objects are envelopes, `put*` verifies
/// before admitting (upload-poisoning guard).
pub struct OpendalStore {
    op: opendal::Operator,
    prefix: String,
}

impl OpendalStore {
    pub fn connect(config: &OpendalConfig) -> Result<OpendalStore> {
        opendal::init_default_registry();
        let mut prefix = config.prefix.clone();
        if !prefix.is_empty() && !prefix.ends_with('/') {
            // OpenDAL paths are directory-shaped; a bare "chunks" prefix
            // must be the directory "chunks/" to be listable.
            prefix.push('/');
        }
        let op = opendal::Operator::via_iter(&config.scheme, config.options.clone())
            .with_context(|| format!("opendal: building '{}' operator", config.scheme))?;
        Ok(OpendalStore { op, prefix })
    }

    fn key(&self, hash: &blake3::Hash) -> String {
        format!("{}{}", self.prefix, hash.to_hex())
    }

    pub async fn has(&self, hash: &blake3::Hash) -> Result<bool> {
        self.op
            .exists(&self.key(hash))
            .await
            .context("opendal exists")
    }

    /// Verifies `blake3(data) == hash` before admitting — same guard as
    /// `DiskStore::put`. Objects land as envelopes (compressed when it pays).
    pub async fn put(&self, hash: &blake3::Hash, data: &[u8]) -> Result<()> {
        if blake3::hash(data) != *hash {
            bail!("chunk data does not match hash {}", hash.to_hex());
        }
        self.put_encoded(hash, super::envelope::encode(data)).await
    }

    /// Store an already-enveloped object after verifying it.
    pub async fn put_encoded(&self, hash: &blake3::Hash, encoded: Vec<u8>) -> Result<()> {
        super::envelope::decode(&encoded, hash)?;
        self.op
            .write(&self.key(hash), encoded)
            .await
            .context("opendal write")?;
        Ok(())
    }

    pub async fn get(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        super::envelope::decode(&self.get_encoded(hash).await?, hash)
    }

    /// The enveloped bytes as stored.
    pub async fn get_encoded(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        Ok(self
            .op
            .read(&self.key(hash))
            .await
            .with_context(|| format!("chunk {} not in opendal store", hash.to_hex()))?
            .to_vec())
    }

    pub async fn remove(&self, hash: &blake3::Hash) -> Result<()> {
        self.op
            .delete(&self.key(hash))
            .await
            .context("opendal delete")?;
        Ok(())
    }

    /// All chunks under the prefix with their last-modified age — GC's
    /// grace period needs it. Falls back to a per-entry stat when the
    /// service's listing omits timestamps; `None` if that fails too.
    pub async fn list(&self) -> Result<Vec<(blake3::Hash, Option<SystemTime>)>> {
        let path = if self.prefix.is_empty() {
            "/"
        } else {
            &self.prefix
        };
        let entries = match self.op.list(path).await {
            Ok(entries) => entries,
            // A store that has never seen a put has no prefix directory yet.
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).context("opendal list"),
        };
        let mut out = Vec::new();
        for entry in entries {
            if !entry.metadata().is_file() {
                continue;
            }
            let Ok(hash) = blake3::Hash::from_hex(entry.name()) else {
                continue; // foreign object under our prefix — not ours to GC
            };
            let mut modified = entry.metadata().last_modified();
            if modified.is_none() {
                // ponytail: sequential stat per entry when a service's
                // listing omits timestamps — batch/parallelize if GC over
                // such a service ever gets slow.
                modified = self
                    .op
                    .stat(entry.path())
                    .await
                    .ok()
                    .and_then(|m| m.last_modified());
            }
            out.push((hash, modified.map(SystemTime::from)));
        }
        Ok(out)
    }
}
