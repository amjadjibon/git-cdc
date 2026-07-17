use serde::{Deserialize, Serialize};

/// LFS-shaped batch API types (DESIGN.md §7, §15.2). `basic` is the only
/// transfer in the MVP; hrefs are server-relative paths the client joins
/// against its configured base URL.
pub const HASH_ALGO: &str = "blake3";
pub const TRANSFER_BASIC: &str = "basic";

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchRequest {
    pub operation: Operation,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transfers: Vec<String>,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<GitRef>,
    pub objects: Vec<ObjectSpec>,
    pub hash_algo: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Upload,
    Download,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GitRef {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectSpec {
    /// `blake3:<hex>`
    pub oid: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BatchResponse {
    pub transfer: String,
    pub objects: Vec<ObjectResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectResult {
    pub oid: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Actions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ObjectError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Actions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<Action>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Action {
    pub href: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObjectError {
    pub code: u16,
    pub message: String,
}

/// Client→server GC request: the complete live set; the server deletes
/// unreferenced chunks past the grace period (PLAN 5.3).
#[derive(Debug, Serialize, Deserialize)]
pub struct GcRequest {
    pub live_oids: Vec<String>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GcResponse {
    pub deleted: Vec<String>,
    pub kept_live: u64,
    pub kept_grace: u64,
}
