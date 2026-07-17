use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

/// Shared S3 connection settings (server flags / CLI git-config both map here).
#[derive(Debug, Clone, Default)]
pub struct S3Config {
    pub bucket: String,
    pub prefix: String,
    /// MinIO/R2: e.g. `http://127.0.0.1:9000`
    pub endpoint: Option<String>,
    /// MinIO needs path-style addressing
    pub force_path_style: bool,
}

/// Build a client from the standard AWS credential chain (env vars,
/// profiles, IMDS), with region falling back to us-east-1 — S3-compatible
/// stores ignore it, and AWS requires *something* to sign with.
pub async fn make_client(config: &S3Config) -> aws_sdk_s3::Client {
    let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest());
    if let Some(endpoint) = &config.endpoint {
        loader = loader.endpoint_url(endpoint);
    }
    let base = loader.load().await;
    let mut builder =
        aws_sdk_s3::config::Builder::from(&base).force_path_style(config.force_path_style);
    if base.region().is_none() {
        builder = builder.region(aws_sdk_s3::config::Region::new("us-east-1"));
    }
    aws_sdk_s3::Client::from_conf(builder.build())
}

/// S3-compatible chunk store (AWS S3, MinIO, R2). Keys are flat chunk hex
/// under an optional prefix — directory sharding is a filesystem concern.
/// Used by the server's s3 backend and the CLI's serverless mode.
pub struct S3Store {
    client: aws_sdk_s3::Client,
    bucket: String,
    prefix: String,
}

impl S3Store {
    pub async fn connect(config: &S3Config) -> S3Store {
        S3Store {
            client: make_client(config).await,
            bucket: config.bucket.clone(),
            prefix: config.prefix.clone(),
        }
    }

    fn key(&self, hash: &blake3::Hash) -> String {
        format!("{}{}", self.prefix, hash.to_hex())
    }

    pub async fn has(&self, hash: &blake3::Hash) -> Result<bool> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(self.key(hash))
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let service_err = e.into_service_error();
                if service_err.is_not_found() {
                    Ok(false)
                } else {
                    Err(service_err).context("s3 head_object")
                }
            }
        }
    }

    /// Verifies `blake3(data) == hash` before admitting — same guard as
    /// `DiskStore::put`.
    pub async fn put(&self, hash: &blake3::Hash, data: &[u8]) -> Result<()> {
        if blake3::hash(data) != *hash {
            bail!("chunk data does not match hash {}", hash.to_hex());
        }
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(self.key(hash))
            .body(aws_sdk_s3::primitives::ByteStream::from(data.to_vec()))
            .send()
            .await
            .context("s3 put_object")?;
        Ok(())
    }

    pub async fn get(&self, hash: &blake3::Hash) -> Result<Vec<u8>> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.key(hash))
            .send()
            .await
            .with_context(|| format!("chunk {} not in s3 store", hash.to_hex()))?;
        let data = resp.body.collect().await.context("reading s3 body")?.to_vec();
        if blake3::hash(&data) != *hash {
            bail!("chunk {} is corrupt in s3", hash.to_hex());
        }
        Ok(data)
    }

    pub async fn remove(&self, hash: &blake3::Hash) -> Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(self.key(hash))
            .send()
            .await
            .context("s3 delete_object")?;
        Ok(())
    }

    /// All chunks under the prefix with their `LastModified` age — GC's
    /// grace period needs it. One request per 1000 objects.
    pub async fn list(&self) -> Result<Vec<(blake3::Hash, Option<SystemTime>)>> {
        let mut out = Vec::new();
        let mut pages = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&self.prefix)
            .into_paginator()
            .send();
        while let Some(page) = pages.next().await {
            let page = page.context("s3 list_objects_v2")?;
            for obj in page.contents() {
                let Some(key) = obj.key() else { continue };
                let Some(hex) = key.strip_prefix(&self.prefix) else { continue };
                let Ok(hash) = blake3::Hash::from_hex(hex) else {
                    continue; // foreign object under our prefix — not ours to GC
                };
                let modified = obj.last_modified().and_then(|dt| {
                    u64::try_from(dt.secs())
                        .ok()
                        .map(|s| UNIX_EPOCH + Duration::from_secs(s))
                });
                out.push((hash, modified));
            }
        }
        Ok(out)
    }
}
