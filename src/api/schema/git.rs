use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum GitSection {
    #[default]
    Local,
    Staged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitStatusParams {
    pub target: String,
    #[serde(default)]
    pub section: GitSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitDiffParams {
    pub target: String,
    /// Checkout identity from git.status; reject a target that has changed checkout.
    pub root: String,
    pub path: String,
    #[serde(default)]
    pub section: GitSection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitFileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    /// None for binary or untracked files.
    pub added: Option<u64>,
    pub removed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitStatusResult {
    /// Stable runtime identity for subsequent reads, including after a pane move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_id: Option<String>,
    pub root: String,
    pub branch: String,
    pub section: GitSection,
    pub files: Vec<GitFileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitDiffResult {
    pub text: String,
    pub truncated: bool,
    pub binary: bool,
}
