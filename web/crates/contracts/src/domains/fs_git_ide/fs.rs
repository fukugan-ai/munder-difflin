use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

/// Opaque handle resolved to a canonical local root only by the server.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct WorkspaceId(pub String);

impl Display for WorkspaceId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Server-issued authority associated with a workspace identity.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceCapability {
    SourceReadOnly,
    PrivateMutable,
}

/// Workspace metadata safe to show in the local browser UI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub name: String,
    pub display_path: String,
    pub capability: WorkspaceCapability,
}

/// One lazily-loaded directory entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_ms: i64,
}

/// UTF-8 file payload accepted by the Monaco editor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextFile {
    pub rel_path: String,
    pub content: String,
    pub size: u64,
}

/// Size-bounded binary file payload used for local image previews.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryFile {
    pub rel_path: String,
    pub bytes: Vec<u8>,
    pub mime: String,
    pub size: u64,
}

/// Metadata-only result for a path inside a managed workspace.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileStat {
    pub exists: bool,
    pub is_file: bool,
    pub rel_path: String,
}

/// Metadata-only result for an absolute path authorized by a registered root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AbsoluteFileStat {
    pub exists: bool,
    pub is_file: bool,
    pub path: String,
}

/// Explicit write command. Browser callers never provide a server path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteFileRequest {
    pub workspace_id: WorkspaceId,
    pub rel_path: String,
    pub content: String,
}

/// Successful local write receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteFileResult {
    pub rel_path: String,
    pub bytes_written: u64,
}

#[cfg(test)]
mod tests {
    use super::WorkspaceId;

    #[test]
    fn workspace_id_display_is_exact() {
        assert_eq!(
            WorkspaceId(String::from("workspace-1")).to_string(),
            "workspace-1"
        );
    }
}
