use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedVariableFile {
    pub file_name: String,
    pub addon_name: String,
    pub last_modified: String,
    pub size_bytes: u64,
    pub character_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SvTreeNode {
    pub key: String,
    pub value_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<SvTreeNode>>,
}

/// Metadata returned alongside a parsed tree so the frontend can
/// detect external modifications before writing back.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvFileStamp {
    pub size: u64,
    pub modified_epoch_ms: u64,
}

/// The read response bundles the tree with its file stamp.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvReadResponse {
    pub tree: SvTreeNode,
    pub stamp: SvFileStamp,
}

/// Preview of what a save would produce: original content vs new content.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvDiffPreview {
    pub original: String,
    pub serialized: String,
}
