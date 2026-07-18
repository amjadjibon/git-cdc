use anyhow::{Context, Result, bail};

use crate::protocol::*;

/// Blocking batch-API client (the CLI is synchronous — clean/smudge filters
/// and one-shot commands; async buys nothing here).
pub struct Client {
    base: String,
    token: String,
    http: reqwest::blocking::Client,
}

impl Client {
    pub fn new(base: &str, token: &str) -> Client {
        Client {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::blocking::Client::new(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    pub fn batch(&self, operation: Operation, objects: Vec<ObjectSpec>) -> Result<BatchResponse> {
        let resp = self
            .http
            .post(self.url("/objects/batch"))
            .bearer_auth(&self.token)
            .json(&BatchRequest {
                operation,
                transfers: vec![TRANSFER_BASIC.into()],
                git_ref: None,
                objects,
                hash_algo: HASH_ALGO.into(),
            })
            .send()
            .context("batch request failed — is the server reachable?")?;
        if !resp.status().is_success() {
            bail!("batch request rejected: {} {}", resp.status(), resp.text()?);
        }
        Ok(resp.json()?)
    }

    pub fn upload(&self, href: &str, data: Vec<u8>) -> Result<()> {
        let resp = self
            .http
            .put(self.url(href))
            .bearer_auth(&self.token)
            .body(data)
            .send()?;
        if !resp.status().is_success() {
            bail!("chunk upload rejected: {} {}", resp.status(), resp.text()?);
        }
        Ok(())
    }

    pub fn download(&self, href: &str) -> Result<Vec<u8>> {
        let resp = self
            .http
            .get(self.url(href))
            .bearer_auth(&self.token)
            .send()?;
        if !resp.status().is_success() {
            bail!("chunk download failed: {} {}", resp.status(), resp.text()?);
        }
        Ok(resp.bytes()?.to_vec())
    }

    pub fn gc(&self, live_oids: Vec<String>, dry_run: bool) -> Result<GcResponse> {
        let resp = self
            .http
            .post(self.url("/gc"))
            .bearer_auth(&self.token)
            .json(&GcRequest { live_oids, dry_run })
            .send()?;
        if !resp.status().is_success() {
            bail!("gc request rejected: {} {}", resp.status(), resp.text()?);
        }
        Ok(resp.json()?)
    }
}
