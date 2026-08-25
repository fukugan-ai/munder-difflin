use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::fs_git_ide::{
    IsolatedWorktree, IsolatedWorktreeState, ProvisionWorktreeRequest, RemoveIsolatedWorktreeResult,
};

use super::git::{run_git, status, validate_revision};
use super::{DomainError, WorkspaceRegistry};

static NEXT_WORKTREE_ID: AtomicU64 = AtomicU64::new(1);

/// Process-owned issuer for isolated worktree capabilities.
///
/// The browser can provide a workspace ID and a display name, but never a filesystem root
/// or destination path. The configured root and every issued path remain server-owned.
pub struct WorktreeProvisioner {
    canonical_root: PathBuf,
    records: Mutex<BTreeMap<String, IsolatedWorktree>>,
}

impl WorktreeProvisioner {
    /// Creates an issuer rooted at one absolute, non-symlink server directory.
    pub fn new(worktree_root: PathBuf) -> Result<Self, DomainError> {
        if !worktree_root.is_absolute() {
            return Err(DomainError::InvalidPath);
        }
        fs::create_dir_all(&worktree_root).map_err(|_| DomainError::Io)?;
        let metadata = fs::symlink_metadata(&worktree_root).map_err(|_| DomainError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(DomainError::InvalidPath);
        }
        let canonical_root = fs::canonicalize(worktree_root).map_err(|_| DomainError::Io)?;
        Ok(Self {
            canonical_root,
            records: Mutex::new(BTreeMap::new()),
        })
    }

    /// Creates a clean local branch and worktree without contacting a remote.
    pub fn create_isolated_worktree(
        &self,
        registry: &WorkspaceRegistry,
        request: &ProvisionWorktreeRequest,
    ) -> Result<IsolatedWorktree, DomainError> {
        let slug = validate_name(&request.name)?;
        validate_revision(&request.base_reference)?;
        let repository = registry.resolve(&request.workspace_id)?;
        if self.canonical_root.starts_with(repository)
            || repository.starts_with(&self.canonical_root)
        {
            return Err(DomainError::InvalidPath);
        }
        ensure_repository(repository)?;
        ensure_clean(repository)?;
        let mut records = self.records.lock().map_err(|_| DomainError::Io)?;

        let base_object = format!("{}^{{commit}}", request.base_reference);
        let base_sha = run_git(
            repository,
            &["rev-parse", "--verify", base_object.as_str()],
            super::git::GIT_TIMEOUT,
        )?
        .stdout
        .trim()
        .to_owned();
        if base_sha.is_empty() {
            return Err(DomainError::InvalidRevision);
        }

        let id = next_id();
        let destination = self.canonical_root.join(&id);
        if destination.exists() {
            return Err(DomainError::InvalidPath);
        }
        let branch = format!("munder/{slug}-{id}");
        let destination_arg = destination.to_string_lossy().into_owned();
        let result = run_git(
            repository,
            &[
                "worktree",
                "add",
                "-b",
                branch.as_str(),
                destination_arg.as_str(),
                request.base_reference.as_str(),
            ],
            super::git::GIT_LONG_TIMEOUT,
        );
        if let Err(error) = result {
            rollback_partial(repository, &destination, &branch, &base_sha);
            return Err(error);
        }

        let canonical_destination = match fs::canonicalize(&destination) {
            Ok(path) if path.starts_with(&self.canonical_root) => path,
            _ => {
                rollback_partial(repository, &destination, &branch, &base_sha);
                return Err(DomainError::InvalidPath);
            }
        };
        let record = IsolatedWorktree {
            id: id.clone(),
            workspace_id: request.workspace_id.clone(),
            path: canonical_destination.to_string_lossy().into_owned(),
            branch,
            state: IsolatedWorktreeState::Active,
        };
        records.insert(id, record.clone());
        Ok(record)
    }

    /// Marks a worktree for preservation without executing Git or deleting anything.
    pub fn archive(&self, id: &str) -> Result<IsolatedWorktree, DomainError> {
        let mut records = self.records.lock().map_err(|_| DomainError::Io)?;
        let record = records.get_mut(id).ok_or(DomainError::UnknownWorktree)?;
        record.state = IsolatedWorktreeState::Archived;
        Ok(record.clone())
    }

    /// Removes only a clean, active worktree. The branch is intentionally preserved.
    pub fn remove_isolated_worktree(
        &self,
        registry: &WorkspaceRegistry,
        id: &str,
    ) -> Result<RemoveIsolatedWorktreeResult, DomainError> {
        let mut records = self.records.lock().map_err(|_| DomainError::Io)?;
        let record = records
            .get(id)
            .cloned()
            .ok_or(DomainError::UnknownWorktree)?;
        if record.state == IsolatedWorktreeState::Archived {
            return Err(DomainError::ArchivedWorktree);
        }
        let repository = registry.resolve(&record.workspace_id)?;
        ensure_repository(repository)?;
        let worktree = fs::canonicalize(&record.path).map_err(|_| DomainError::NotFound)?;
        if !worktree.starts_with(&self.canonical_root) || worktree == self.canonical_root {
            return Err(DomainError::InvalidPath);
        }
        ensure_clean(&worktree)?;
        let worktree_arg = worktree.to_string_lossy().into_owned();
        run_git(
            repository,
            &["worktree", "remove", worktree_arg.as_str()],
            super::git::GIT_LONG_TIMEOUT,
        )?;
        records.remove(id);
        Ok(RemoveIsolatedWorktreeResult {
            id: String::from(id),
            branch: record.branch,
            branch_preserved: true,
        })
    }

    /// Resolves an opaque ID for trusted server integration code.
    pub fn get(&self, id: &str) -> Result<IsolatedWorktree, DomainError> {
        self.records
            .lock()
            .map_err(|_| DomainError::Io)?
            .get(id)
            .cloned()
            .ok_or(DomainError::UnknownWorktree)
    }
}

fn ensure_repository(root: &Path) -> Result<(), DomainError> {
    let valid = run_git(
        root,
        &["rev-parse", "--is-inside-work-tree"],
        super::git::GIT_TIMEOUT,
    )
    .is_ok_and(|output| output.stdout.trim() == "true");
    valid.then_some(()).ok_or(DomainError::NotGitRepository)
}

fn ensure_clean(root: &Path) -> Result<(), DomainError> {
    let current = status(root)?;
    if current.staged.is_empty() && current.unstaged.is_empty() && current.untracked.is_empty() {
        Ok(())
    } else {
        Err(DomainError::DirtyWorktree)
    }
}

fn validate_name(name: &str) -> Result<String, DomainError> {
    let trimmed = name.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= 48
        && !trimmed.starts_with('-')
        && trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    valid
        .then(|| trimmed.to_ascii_lowercase())
        .ok_or(DomainError::InvalidWorktreeName)
}

fn next_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = NEXT_WORKTREE_ID.fetch_add(1, Ordering::Relaxed);
    format!("wt-{nanos:x}-{sequence:x}")
}

fn rollback_partial(repository: &Path, destination: &Path, branch: &str, base_sha: &str) {
    let destination_arg = destination.to_string_lossy().into_owned();
    let _ = run_git(
        repository,
        &["worktree", "remove", destination_arg.as_str()],
        super::git::GIT_LONG_TIMEOUT,
    );
    let reference = format!("refs/heads/{branch}");
    let _ = run_git(
        repository,
        &["update-ref", "-d", reference.as_str(), base_sha],
        super::git::GIT_TIMEOUT,
    );
    let _ = fs::remove_dir(destination);
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use md_web_contracts::domains::fs_git_ide::{
        IsolatedWorktreeState, ProvisionWorktreeRequest, WorkspaceId,
    };

    use super::{WorktreeProvisioner, rollback_partial};
    use crate::domains::fs_git_ide::WorkspaceRegistry;

    fn fixture(name: &str) -> Result<(PathBuf, WorkspaceRegistry), Box<dyn std::error::Error>> {
        let base = std::env::temp_dir().join(format!(
            "md-worktree-provisioner-{name}-{}",
            std::process::id()
        ));
        if base.exists() {
            fs::remove_dir_all(&base)?;
        }
        let repository = base.join("repository");
        fs::create_dir_all(&repository)?;
        git(&repository, &["init", "-q"])?;
        git(
            &repository,
            &["config", "user.email", "test@example.invalid"],
        )?;
        git(&repository, &["config", "user.name", "Test"])?;
        fs::write(repository.join("README.md"), "fixture")?;
        git(&repository, &["add", "README.md"])?;
        git(&repository, &["commit", "-q", "-m", "fixture"])?;
        let registry = WorkspaceRegistry::from_paths([repository]);
        Ok((base, registry))
    }

    fn request(name: &str) -> ProvisionWorktreeRequest {
        ProvisionWorktreeRequest {
            workspace_id: WorkspaceId(String::from("workspace-1")),
            name: String::from(name),
            base_reference: String::from("HEAD"),
        }
    }

    fn git(cwd: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git").args(args).current_dir(cwd).status()?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "git failed".into())
    }

    #[test]
    fn creates_under_server_root_and_preserves_branch_on_remove()
    -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("create-remove")?;
        let root = base.join("worktrees");
        let provisioner = WorktreeProvisioner::new(root.clone())?;
        let record = provisioner.create_isolated_worktree(&registry, &request("Agent_1"))?;
        assert!(Path::new(&record.path).starts_with(fs::canonicalize(&root)?));
        assert_eq!(record.state, IsolatedWorktreeState::Active);
        let removed = provisioner.remove_isolated_worktree(&registry, &record.id)?;
        assert!(removed.branch_preserved);
        assert!(!Path::new(&record.path).exists());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn archive_preserves_and_blocks_removal() -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("archive")?;
        let provisioner = WorktreeProvisioner::new(base.join("worktrees"))?;
        let record = provisioner.create_isolated_worktree(&registry, &request("agent"))?;
        assert_eq!(
            provisioner.archive(&record.id)?.state,
            IsolatedWorktreeState::Archived
        );
        assert!(
            provisioner
                .remove_isolated_worktree(&registry, &record.id)
                .is_err()
        );
        assert!(Path::new(&record.path).exists());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn rejects_browser_style_path_names_before_git() -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("invalid-name")?;
        let provisioner = WorktreeProvisioner::new(base.join("worktrees"))?;
        assert!(
            provisioner
                .create_isolated_worktree(&registry, &request("../../escape"))
                .is_err()
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn dirty_repository_is_not_provisioned() -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("dirty")?;
        fs::write(base.join("repository").join("dirty.txt"), "dirty")?;
        let provisioner = WorktreeProvisioner::new(base.join("worktrees"))?;
        assert!(
            provisioner
                .create_isolated_worktree(&registry, &request("agent"))
                .is_err()
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn dirty_isolated_worktree_is_preserved() -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("dirty-isolated")?;
        let provisioner = WorktreeProvisioner::new(base.join("worktrees"))?;
        let record = provisioner.create_isolated_worktree(&registry, &request("agent"))?;
        fs::write(Path::new(&record.path).join("agent.txt"), "work")?;
        assert!(
            provisioner
                .remove_isolated_worktree(&registry, &record.id)
                .is_err()
        );
        assert!(Path::new(&record.path).exists());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn partial_rollback_removes_only_the_exact_new_branch_and_empty_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let (base, _registry) = fixture("rollback")?;
        let repository = base.join("repository");
        let destination = base.join("worktrees").join("partial");
        fs::create_dir_all(&destination)?;
        let branch = "munder/rollback-test";
        git(&repository, &["branch", branch, "HEAD"])?;
        let base_sha = String::from_utf8(
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(&repository)
                .output()?
                .stdout,
        )?;

        rollback_partial(&repository, &destination, branch, base_sha.trim());

        let branch_exists = Command::new("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                "refs/heads/munder/rollback-test",
            ])
            .current_dir(&repository)
            .status()?
            .success();
        assert!(!branch_exists);
        assert!(!destination.exists());
        fs::remove_dir_all(base)?;
        Ok(())
    }
}
