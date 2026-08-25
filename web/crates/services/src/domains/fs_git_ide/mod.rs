//! Server-only filesystem, local Git, and read-only GitHub services.

mod command;
mod error;
mod fs;
mod git;
mod github;
mod workspace;
mod worktree;

pub use error::DomainError;
pub use fs::FsService;
pub use git::GitService;
pub use github::GitHubService;
pub use workspace::WorkspaceRegistry;
pub use worktree::WorktreeProvisioner;
