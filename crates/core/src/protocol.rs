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

#[cfg(test)]
mod tests {
    use super::*;

    // Locks the wire format: field names and omissions are protocol, not
    // implementation — a rename here breaks every existing client/server pair.
    #[test]
    fn batch_request_wire_format_is_lfs_shaped() {
        let req = BatchRequest {
            operation: Operation::Upload,
            transfers: vec![TRANSFER_BASIC.into()],
            git_ref: Some(GitRef { name: "refs/heads/main".into() }),
            objects: vec![ObjectSpec { oid: "blake3:ab".into(), size: 7 }],
            hash_algo: HASH_ALGO.into(),
        };
        let json: serde_json::Value = serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        assert_eq!(json["operation"], "upload");
        assert_eq!(json["ref"]["name"], "refs/heads/main");
        assert_eq!(json["hash_algo"], "blake3");
        assert_eq!(json["objects"][0]["oid"], "blake3:ab");
    }

    #[test]
    fn optional_fields_are_omitted_not_null() {
        let req = BatchRequest {
            operation: Operation::Download,
            transfers: vec![],
            git_ref: None,
            objects: vec![],
            hash_algo: HASH_ALGO.into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("ref"), "absent ref must be omitted: {json}");
        assert!(!json.contains("transfers"), "empty transfers must be omitted: {json}");

        let ok = ObjectResult { oid: "blake3:ab".into(), size: 7, actions: None, error: None };
        let json = serde_json::to_string(&ok).unwrap();
        assert!(!json.contains("actions") && !json.contains("error"), "{json}");
    }

    #[test]
    fn missing_optional_fields_deserialize() {
        let req: BatchRequest = serde_json::from_str(
            r#"{"operation":"download","objects":[],"hash_algo":"blake3"}"#,
        )
        .unwrap();
        assert!(req.git_ref.is_none() && req.transfers.is_empty());
        let gc: GcRequest = serde_json::from_str(r#"{"live_oids":[]}"#).unwrap();
        assert!(!gc.dry_run, "dry_run defaults to false");
    }
}
