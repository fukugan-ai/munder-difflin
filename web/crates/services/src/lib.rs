#![forbid(unsafe_code)]

mod app_state;
pub mod domains;
mod health;
mod persistence;

pub use app_state::AppState;
pub use domains::DomainRegistry;
pub use domains::config_onboarding::{
    probe_host_tools, resolve_release_repository, web_capabilities,
};
pub use domains::connections::ConnectionsService;
pub use domains::fs_git_ide::{
    DomainError as FsGitIdeError, FsService, GitHubService, GitService, WorkspaceRegistry,
    WorktreeProvisioner,
};
pub use persistence::ServiceError;
