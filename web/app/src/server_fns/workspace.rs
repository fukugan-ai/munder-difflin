use dioxus::prelude::*;
use md_web_contracts::domains::fs_git_ide::{
    AbsoluteFileStat, BinaryFile, CheckoutRequest, CheckoutResult, CiRun, DirEntry, GitCommit,
    GitCompare, GitDiff, GitFileAtRevision, GitFileChange, GitHubIssue, GitOverview, GitWorktree,
    TextFile, WorkspaceId, WorkspaceSummary, WriteFileRequest, WriteFileResult,
};

#[cfg(feature = "server")]
async fn registry() -> Result<md_web_services::WorkspaceRegistry, ServerFnError> {
    use std::path::PathBuf;

    let override_paths = std::env::var_os("MD_REGISTERED_REPOS")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .filter(|paths| !paths.is_empty());
    let repository = super::persistence_repository()
        .await
        .map_err(|_| safe_error())?;
    let config = md_web_services::domains::config_onboarding::load_config(&repository)
        .await
        .map_err(|_| safe_error())?;
    let paths = override_paths
        .unwrap_or_else(|| config.registered_repos.iter().map(PathBuf::from).collect());
    let sources = md_web_services::WorkspaceRegistry::from_source_paths(paths);
    let Some(harness_home) = std::env::var_os("MD_HARNESS_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| config.harness_home.as_ref().map(PathBuf::from))
        .filter(|path| path.is_absolute())
    else {
        return Ok(sources);
    };
    let authority = md_web_services::PrivateWorkspaceRoot::new(harness_home.join("worktrees"))
        .map_err(service_error)?;
    let private_capabilities = super::pty::workspace_private_capabilities().await?;
    Ok(sources.with_private_workspaces(&authority, private_capabilities))
}

#[cfg_attr(not(feature = "server"), allow(dead_code))]
fn safe_error() -> ServerFnError {
    ServerFnError::new("workspace操作に失敗しました")
}

#[cfg(feature = "server")]
fn service_error(_: md_web_services::FsGitIdeError) -> ServerFnError {
    safe_error()
}

#[get("/api/fs-git-ide/workspaces")]
pub(crate) async fn workspaces() -> Result<Vec<WorkspaceSummary>, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(registry().await?.list());
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

// Directory paths may legitimately be empty at the workspace root. Keep them in the request body
// so URL path normalization cannot erase the root representation before the server function runs.
#[post("/api/fs-git-ide/list")]
pub(crate) async fn list_dir(
    workspace_id: WorkspaceId,
    rel_path: String,
) -> Result<Vec<DirEntry>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::FsService::list_dir(&registry().await?, &workspace_id, &rel_path)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/text/:workspace_id/:rel_path")]
pub(crate) async fn read_text(
    workspace_id: WorkspaceId,
    rel_path: String,
) -> Result<TextFile, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::FsService::read_text(&registry().await?, &workspace_id, &rel_path)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/binary/:workspace_id/:rel_path")]
pub(crate) async fn read_binary(
    workspace_id: WorkspaceId,
    rel_path: String,
) -> Result<BinaryFile, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::FsService::read_binary(&registry().await?, &workspace_id, &rel_path)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[post("/api/fs-git-ide/text")]
pub(crate) async fn write_text(
    request: WriteFileRequest,
) -> Result<WriteFileResult, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::FsService::write_text(&registry().await?, &request)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[post("/api/fs-git-ide/stat-absolute")]
pub(crate) async fn stat_absolute(path: String) -> Result<AbsoluteFileStat, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::FsService::stat_absolute(&registry().await?, &path)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/:workspace_id")]
pub(crate) async fn git_overview(workspace_id: WorkspaceId) -> Result<GitOverview, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::overview(&registry().await?, &workspace_id, 100)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/main/:workspace_id")]
pub(crate) async fn git_main_repository(
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceSummary>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::main_repository(&registry().await?, &workspace_id)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/diff/:workspace_id/:rel_path")]
pub(crate) async fn git_diff(
    workspace_id: WorkspaceId,
    rel_path: String,
) -> Result<GitDiff, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::diff(&registry().await?, &workspace_id, &rel_path)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/history/:workspace_id/:count/:skip")]
pub(crate) async fn git_history(
    workspace_id: WorkspaceId,
    count: u16,
    skip: u32,
) -> Result<Vec<GitCommit>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::history(&registry().await?, &workspace_id, count, skip)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/commit-files/:workspace_id/:revision")]
pub(crate) async fn git_commit_files(
    workspace_id: WorkspaceId,
    revision: String,
) -> Result<Vec<GitFileChange>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::commit_files(&registry().await?, &workspace_id, &revision)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/show/:workspace_id/:revision/:rel_path")]
pub(crate) async fn git_show_file(
    workspace_id: WorkspaceId,
    revision: String,
    rel_path: String,
) -> Result<GitFileAtRevision, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::file_at_revision(
        &registry().await?,
        &workspace_id,
        &revision,
        &rel_path,
    )
    .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/compare/:workspace_id/:base/:head/:three_dot")]
pub(crate) async fn git_compare_refs(
    workspace_id: WorkspaceId,
    base: String,
    head: String,
    three_dot: bool,
) -> Result<GitCompare, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::compare(
        &registry().await?,
        &workspace_id,
        &base,
        &head,
        three_dot,
    )
    .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/git/worktrees/:workspace_id")]
pub(crate) async fn git_worktrees(
    workspace_id: WorkspaceId,
) -> Result<Vec<GitWorktree>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitService::worktrees(&registry().await?, &workspace_id)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[post("/api/fs-git-ide/git/checkout")]
pub(crate) async fn git_checkout(
    request: CheckoutRequest,
) -> Result<CheckoutResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let workspaces = registry().await?;
        let selected = workspaces
            .list()
            .into_iter()
            .find(|workspace| workspace.id == request.workspace_id)
            .ok_or_else(safe_error)?;
        let (active, _) = super::pty::list_agents().await.map_err(|_| safe_error())?;
        let root = std::path::Path::new(&selected.display_path);
        let busy = active.iter().any(|agent| {
            std::path::Path::new(&agent.cwd)
                .canonicalize()
                .is_ok_and(|cwd| cwd.starts_with(root))
        });
        md_web_services::GitService::checkout_local(&workspaces, &request, busy)
            .map_err(service_error)
    }
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/github/issues/:workspace_id")]
pub(crate) async fn github_issues(
    workspace_id: WorkspaceId,
) -> Result<Vec<GitHubIssue>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitHubService::issues(&registry().await?, &workspace_id)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

#[get("/api/fs-git-ide/github/ci/:workspace_id")]
pub(crate) async fn github_ci_runs(workspace_id: WorkspaceId) -> Result<Vec<CiRun>, ServerFnError> {
    #[cfg(feature = "server")]
    return md_web_services::GitHubService::ci_runs(&registry().await?, &workspace_id)
        .map_err(service_error);
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}
