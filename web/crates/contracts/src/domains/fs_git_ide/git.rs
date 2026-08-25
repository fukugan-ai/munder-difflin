use serde::{Deserialize, Serialize};

use super::WorkspaceId;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitBranch {
    pub current: Option<String>,
    pub detached: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    pub index: char,
    pub worktree: char,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitStatus {
    pub staged: Vec<GitStatusEntry>,
    pub unstaged: Vec<GitStatusEntry>,
    pub untracked: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitCommit {
    pub sha: String,
    pub short_sha: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author: String,
    pub time: i64,
    pub refs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitAheadBehind {
    pub ahead: u64,
    pub behind: u64,
    pub upstream: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitOverview {
    pub is_repo: bool,
    pub branch: Option<GitBranch>,
    pub status: Option<GitStatus>,
    pub commits: Vec<GitCommit>,
    pub local_branches: Vec<String>,
    pub remote_branches: Vec<String>,
    pub ahead_behind: Option<GitAheadBehind>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitDiff {
    pub rel_path: String,
    pub head: String,
    pub working: String,
    pub head_exists: bool,
    pub working_exists: bool,
    pub is_binary: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitFileChange {
    pub path: String,
    pub status: char,
    pub old_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitFileAtRevision {
    pub exists: bool,
    pub is_binary: bool,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitCompare {
    pub ahead: u64,
    pub behind: u64,
    pub merge_base: Option<String>,
    pub files: Vec<GitFileChange>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GitWorktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
}

/// Local-only checkout request. It cannot name a remote GitHub mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckoutRequest {
    pub workspace_id: WorkspaceId,
    pub reference: String,
    pub detach: bool,
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckoutResult {
    pub detached: bool,
}

/// Server-mediated request for an isolated local worktree.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProvisionWorktreeRequest {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub base_reference: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolatedWorktreeState {
    Active,
    Archived,
}

/// Opaque server-issued authority for one app-owned mutable clone/copy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PrivateWorkspaceCapability {
    pub id: String,
    pub workspace_id: WorkspaceId,
    pub source_workspace_id: WorkspaceId,
    pub path: String,
}

/// Provisioning receipt. The nested capability must be persisted with the Agent record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IsolatedWorktree {
    pub capability: PrivateWorkspaceCapability,
    pub branch: String,
    pub state: IsolatedWorktreeState,
}

/// Removal preserves the local branch so agent work remains recoverable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoveIsolatedWorktreeResult {
    pub id: String,
    pub branch: String,
    pub branch_preserved: bool,
}
