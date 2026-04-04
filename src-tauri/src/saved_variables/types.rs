use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SvValueType {
    Table,
    String,
    Number,
    Boolean,
    Nil,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SvChangeType {
    Modified,
    Added,
    Removed,
}

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
    pub value_type: SvValueType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<SvTreeNode>>,
    /// Pre-escaped Lua string content for values that contain non-UTF8 bytes.
    /// When present, the serializer outputs this directly instead of re-escaping
    /// the `value` field, ensuring lossless round-tripping of binary addon data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_lua_value: Option<String>,
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

/// A single value change detected between the original and edited trees.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvChange {
    /// Key path from root, e.g. ["Default", "@Account", "settingName"]
    pub path: Vec<String>,
    pub change_type: SvChangeType,
    /// Human-readable old value (None for additions)
    pub old_value: Option<String>,
    /// Human-readable new value (None for removals)
    pub new_value: Option<String>,
}

/// Preview of changes: a list of individual value diffs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvDiffPreview {
    pub changes: Vec<SvChange>,
}
