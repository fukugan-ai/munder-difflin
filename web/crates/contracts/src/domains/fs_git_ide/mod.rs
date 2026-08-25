//! Shared contracts for the browser IDE and its server-owned workspace bridge.

mod fs;
mod git;
mod github;
mod ide;

pub use fs::{
    AbsoluteFileStat, BinaryFile, DirEntry, FileStat, TextFile, WorkspaceCapability, WorkspaceId,
    WorkspaceSummary, WriteFileRequest, WriteFileResult,
};
pub use git::{
    CheckoutRequest, CheckoutResult, GitAheadBehind, GitBranch, GitCommit, GitCompare, GitDiff,
    GitFileAtRevision, GitFileChange, GitOverview, GitStatus, GitStatusEntry, GitWorktree,
    IsolatedWorktree, IsolatedWorktreeState, PrivateWorkspaceCapability, ProvisionWorktreeRequest,
    RemoveIsolatedWorktreeResult,
};
pub use github::{CiRun, GitHubIssue};
pub use ide::{IdeDocument, IdeDocumentKind, IdeSaveState};
