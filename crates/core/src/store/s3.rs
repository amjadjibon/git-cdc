//! S3 backend settings. Since the OpenDAL migration this is just a flag/
//! git-config surface that maps onto [`OpendalStore`](super::OpendalStore)
//! with the `s3` scheme — kept so existing `--s3-*` flags and `cdc.s3.*`
//! git config keep working unchanged.

use anyhow::Result;

use super::opendal::{OpendalConfig, OpendalStore};

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

impl S3Config {
    /// Credentials/region come from the standard AWS env vars and config
    /// files (OpenDAL loads them); region falls back to us-east-1 —
    /// S3-compatible stores ignore it, and AWS requires *something* to sign
    /// with. Addressing style matches the old aws-sdk behavior:
    /// virtual-host unless `force_path_style`.
    pub fn to_opendal(&self) -> OpendalConfig {
        let mut options = vec![("bucket".to_string(), self.bucket.clone())];
        if let Some(endpoint) = &self.endpoint {
            options.push(("endpoint".into(), endpoint.clone()));
        }
        if !self.force_path_style {
            options.push(("enable_virtual_host_style".into(), "true".into()));
        }
        if std::env::var("AWS_REGION").is_err() && std::env::var("AWS_DEFAULT_REGION").is_err() {
            options.push(("region".into(), "us-east-1".into()));
        }
        OpendalConfig {
            scheme: "s3".into(),
            options,
            prefix: self.prefix.clone(),
        }
    }

    /// The S3-compatible chunk store (AWS S3, MinIO, R2) for these settings.
    pub fn connect(&self) -> Result<OpendalStore> {
        OpendalStore::connect(&self.to_opendal())
    }
}
