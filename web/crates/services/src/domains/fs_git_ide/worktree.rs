use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::fs_git_ide::{
    IsolatedWorktree, IsolatedWorktreeState, PrivateWorkspaceCapability, ProvisionWorktreeRequest,
    RemoveIsolatedWorktreeResult, WorkspaceId,
};

use super::git::{run_git, status, validate_revision};
use super::{DomainError, PrivateWorkspaceRoot, WorkspaceRegistry};

const MAX_COPY_FILES: u64 = 100_000;
const MAX_COPY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct ProvisionedRecord {
    receipt: IsolatedWorktree,
    is_git: bool,
    initial_fingerprint: Option<u64>,
}

/// Issues app-owned private clones/copies. It never creates a Git worktree or updates source Git
/// metadata; registered source repositories remain read-only capabilities.
pub struct WorktreeProvisioner {
    authority: PrivateWorkspaceRoot,
    records: Mutex<BTreeMap<String, ProvisionedRecord>>,
}

impl WorktreeProvisioner {
    pub fn new(root: PathBuf) -> Result<Self, DomainError> {
        Ok(Self {
            authority: PrivateWorkspaceRoot::new(root)?,
            records: Mutex::new(BTreeMap::new()),
        })
    }

    pub fn private_root(&self) -> PrivateWorkspaceRoot {
        self.authority.clone()
    }

    pub fn create_isolated_worktree(
        &self,
        registry: &WorkspaceRegistry,
        request: &ProvisionWorktreeRequest,
    ) -> Result<IsolatedWorktree, DomainError> {
        let slug = validate_name(&request.name)?;
        validate_revision(&request.base_reference)?;
        let source = registry.resolve_source(&request.workspace_id)?;
        if self.authority.path().starts_with(source) || source.starts_with(self.authority.path()) {
            return Err(DomainError::InvalidPath);
        }

        let id = next_id();
        let destination = self.authority.path().join(&id);
        if destination.exists() {
            return Err(DomainError::InvalidPath);
        }
        let branch = format!("munder/{slug}-{id}");
        let is_git = is_repository(source);
        let initial_fingerprint = if is_git {
            clone_repository(source, &destination, &request.base_reference, &branch)?;
            None
        } else {
            copy_bounded(source, &destination)?;
            Some(tree_fingerprint(&destination)?)
        };

        let canonical = self
            .authority
            .authorize(destination.clone())
            .ok_or_else(|| {
                cleanup_private(&destination);
                DomainError::InvalidPath
            })?;
        let receipt = IsolatedWorktree {
            capability: PrivateWorkspaceCapability {
                id: id.clone(),
                workspace_id: WorkspaceId(format!("private-{id}")),
                source_workspace_id: request.workspace_id.clone(),
                path: canonical.to_string_lossy().into_owned(),
            },
            branch: if is_git { branch } else { String::new() },
            state: IsolatedWorktreeState::Active,
        };
        self.records.lock().map_err(|_| DomainError::Io)?.insert(
            id,
            ProvisionedRecord {
                receipt: receipt.clone(),
                is_git,
                initial_fingerprint,
            },
        );
        Ok(receipt)
    }

    pub fn archive(&self, id: &str) -> Result<IsolatedWorktree, DomainError> {
        let mut records = self.records.lock().map_err(|_| DomainError::Io)?;
        let record = records.get_mut(id).ok_or(DomainError::UnknownWorktree)?;
        record.receipt.state = IsolatedWorktreeState::Archived;
        Ok(record.receipt.clone())
    }

    /// Removes only an unchanged private clone/copy. No source Git command or metadata mutation is
    /// performed, and archived/dirty private workspaces fail closed.
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
        if record.receipt.state == IsolatedWorktreeState::Archived {
            return Err(DomainError::ArchivedWorktree);
        }
        registry.resolve_source(&record.receipt.capability.source_workspace_id)?;
        let private = self
            .authority
            .authorize(PathBuf::from(&record.receipt.capability.path))
            .ok_or(DomainError::InvalidPath)?;
        if record.is_git {
            ensure_clean(&private)?;
        } else if record.initial_fingerprint != Some(tree_fingerprint(&private)?) {
            return Err(DomainError::DirtyWorktree);
        }
        fs::remove_dir_all(&private).map_err(|_| DomainError::Io)?;
        records.remove(id);
        Ok(RemoveIsolatedWorktreeResult {
            id: String::from(id),
            branch: record.receipt.branch,
            branch_preserved: false,
        })
    }

    pub fn get(&self, id: &str) -> Result<IsolatedWorktree, DomainError> {
        self.records
            .lock()
            .map_err(|_| DomainError::Io)?
            .get(id)
            .map(|record| record.receipt.clone())
            .ok_or(DomainError::UnknownWorktree)
    }
}

fn is_repository(root: &Path) -> bool {
    run_git(
        root,
        &["rev-parse", "--show-toplevel"],
        super::git::GIT_TIMEOUT,
    )
    .ok()
    .and_then(|output| fs::canonicalize(output.stdout.trim()).ok())
    .is_some_and(|top| top == root)
}

fn clone_repository(
    source: &Path,
    destination: &Path,
    base_reference: &str,
    branch: &str,
) -> Result<(), DomainError> {
    let base_object = format!("{base_reference}^{{commit}}");
    let base_sha = run_git(
        source,
        &["rev-parse", "--verify", &base_object],
        super::git::GIT_TIMEOUT,
    )?
    .stdout
    .trim()
    .to_owned();
    if base_sha.is_empty() {
        return Err(DomainError::InvalidRevision);
    }
    let source_arg = source.to_string_lossy().into_owned();
    let destination_arg = destination.to_string_lossy().into_owned();
    if let Err(error) = run_git(
        source.parent().unwrap_or(source),
        &[
            "clone",
            "--no-hardlinks",
            "--no-checkout",
            &source_arg,
            &destination_arg,
        ],
        super::git::GIT_LONG_TIMEOUT,
    ) {
        cleanup_private(destination);
        return Err(error);
    }
    if let Err(error) = run_git(
        destination,
        &["switch", "-c", branch, &base_sha],
        super::git::GIT_LONG_TIMEOUT,
    ) {
        cleanup_private(destination);
        return Err(error);
    }
    if let Err(error) = run_git(
        destination,
        &["remote", "remove", "origin"],
        super::git::GIT_TIMEOUT,
    ) {
        cleanup_private(destination);
        return Err(error);
    }
    if let Err(error) = reject_symlinks(destination) {
        cleanup_private(destination);
        return Err(error);
    }
    Ok(())
}

fn reject_symlinks(root: &Path) -> Result<(), DomainError> {
    for entry in fs::read_dir(root).map_err(|_| DomainError::Io)? {
        let entry = entry.map_err(|_| DomainError::Io)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| DomainError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(DomainError::InvalidPath);
        }
        if metadata.is_dir() {
            reject_symlinks(&entry.path())?;
        } else if !metadata.is_file() {
            return Err(DomainError::InvalidPath);
        }
    }
    Ok(())
}

fn copy_bounded(source: &Path, destination: &Path) -> Result<(), DomainError> {
    fs::create_dir(destination).map_err(|_| DomainError::Io)?;
    let mut budget = CopyBudget::default();
    let result = copy_directory(source, destination, &mut budget);
    if result.is_err() {
        cleanup_private(destination);
    }
    result
}

#[derive(Default)]
struct CopyBudget {
    files: u64,
    bytes: u64,
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    budget: &mut CopyBudget,
) -> Result<(), DomainError> {
    for entry in fs::read_dir(source).map_err(|_| DomainError::Io)? {
        let entry = entry.map_err(|_| DomainError::Io)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| DomainError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(DomainError::InvalidPath);
        }
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|_| DomainError::Io)?;
            copy_directory(&entry.path(), &target, budget)?;
        } else if metadata.is_file() {
            budget.files = budget.files.saturating_add(1);
            budget.bytes = budget.bytes.saturating_add(metadata.len());
            if budget.files > MAX_COPY_FILES || budget.bytes > MAX_COPY_BYTES {
                return Err(DomainError::FileTooLarge);
            }
            fs::copy(entry.path(), target).map_err(|_| DomainError::Io)?;
        } else {
            return Err(DomainError::InvalidPath);
        }
    }
    Ok(())
}

fn tree_fingerprint(root: &Path) -> Result<u64, DomainError> {
    let mut paths = Vec::new();
    collect_files(root, root, &mut paths)?;
    paths.sort();
    let mut hasher = DefaultHasher::new();
    let mut total = 0_u64;
    for relative in paths {
        relative.hash(&mut hasher);
        let path = root.join(&relative);
        let metadata = fs::metadata(&path).map_err(|_| DomainError::Io)?;
        total = total.saturating_add(metadata.len());
        if total > MAX_COPY_BYTES {
            return Err(DomainError::FileTooLarge);
        }
        let mut file = fs::File::open(path).map_err(|_| DomainError::Io)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|_| DomainError::Io)?;
            if read == 0 {
                break;
            }
            buffer[..read].hash(&mut hasher);
        }
    }
    Ok(hasher.finish())
}

fn collect_files(root: &Path, current: &Path, paths: &mut Vec<PathBuf>) -> Result<(), DomainError> {
    for entry in fs::read_dir(current).map_err(|_| DomainError::Io)? {
        let entry = entry.map_err(|_| DomainError::Io)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| DomainError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(DomainError::InvalidPath);
        }
        if metadata.is_dir() {
            collect_files(root, &entry.path(), paths)?;
        } else if metadata.is_file() {
            paths.push(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(|_| DomainError::InvalidPath)?
                    .to_path_buf(),
            );
        } else {
            return Err(DomainError::InvalidPath);
        }
    }
    Ok(())
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
    let sequence = NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed);
    format!("wt-{nanos:x}-{sequence:x}")
}

fn cleanup_private(path: &Path) {
    if path.exists() {
        let _ = fs::remove_dir_all(path);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use md_web_contracts::domains::fs_git_ide::{
        CheckoutRequest, IsolatedWorktreeState, ProvisionWorktreeRequest, WorkspaceId,
        WriteFileRequest,
    };

    use super::WorktreeProvisioner;
    use crate::domains::fs_git_ide::{FsService, GitService, WorkspaceRegistry};

    fn fixture(name: &str) -> Result<(PathBuf, WorkspaceRegistry), Box<dyn std::error::Error>> {
        let base = std::env::temp_dir().join(format!(
            "md-private-workspace-{name}-{}",
            std::process::id()
        ));
        if base.exists() {
            fs::remove_dir_all(&base)?;
        }
        let source = base.join("source");
        fs::create_dir_all(&source)?;
        git(&source, &["init", "-q"])?;
        git(&source, &["config", "user.email", "test@example.invalid"])?;
        git(&source, &["config", "user.name", "Test"])?;
        fs::write(source.join("README.md"), "one")?;
        git(&source, &["add", "README.md"])?;
        git(&source, &["commit", "-q", "-m", "one"])?;
        fs::write(source.join("README.md"), "two")?;
        git(&source, &["commit", "-q", "-am", "two"])?;
        Ok((base, WorkspaceRegistry::from_paths([source])))
    }

    fn request(name: &str) -> ProvisionWorktreeRequest {
        ProvisionWorktreeRequest {
            workspace_id: WorkspaceId(String::from("source-1")),
            name: String::from(name),
            base_reference: String::from("HEAD"),
        }
    }

    fn git(cwd: &Path, args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
        Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()?
            .success()
            .then_some(())
            .ok_or_else(|| "git failed".into())
    }

    fn git_output(cwd: &Path, args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
        let output = Command::new("git").args(args).current_dir(cwd).output()?;
        if !output.status.success() {
            return Err("git failed".into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    #[test]
    fn save_and_checkout_never_mutate_registered_source() -> Result<(), Box<dyn std::error::Error>>
    {
        let (base, source_registry) = fixture("source-invariant")?;
        let source = base.join("source");
        let before_file = fs::read(source.join("README.md"))?;
        let before_branch = git_output(&source, &["branch", "--show-current"])?;
        let before_head = git_output(&source, &["rev-parse", "HEAD"])?;
        let before_status = git_output(&source, &["status", "--porcelain=v1"])?;
        let provisioner = WorktreeProvisioner::new(base.join("private"))?;
        let private = provisioner.create_isolated_worktree(&source_registry, &request("agent"))?;
        assert!(git_output(Path::new(&private.capability.path), &["remote"])?.is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_ne!(
                fs::metadata(source.join("README.md"))?.ino(),
                fs::metadata(Path::new(&private.capability.path).join("README.md"))?.ino()
            );
        }
        let registry = source_registry
            .with_private_workspaces(&provisioner.private_root(), [private.capability.clone()]);

        assert!(
            FsService::write_text(
                &registry,
                &WriteFileRequest {
                    workspace_id: WorkspaceId(String::from("source-1")),
                    rel_path: String::from("README.md"),
                    content: String::from("forbidden"),
                }
            )
            .is_err()
        );
        assert!(
            GitService::checkout_local(
                &registry,
                &CheckoutRequest {
                    workspace_id: WorkspaceId(String::from("source-1")),
                    reference: String::from("HEAD^"),
                    detach: true,
                    confirmed: true,
                },
                false,
            )
            .is_err()
        );
        FsService::write_text(
            &registry,
            &WriteFileRequest {
                workspace_id: private.capability.workspace_id.clone(),
                rel_path: String::from("README.md"),
                content: String::from("private"),
            },
        )?;
        git(
            Path::new(&private.capability.path),
            &["checkout", "--", "README.md"],
        )?;
        GitService::checkout_local(
            &registry,
            &CheckoutRequest {
                workspace_id: private.capability.workspace_id,
                reference: String::from("HEAD^"),
                detach: true,
                confirmed: true,
            },
            false,
        )?;

        assert_eq!(fs::read(source.join("README.md"))?, before_file);
        assert_eq!(
            git_output(&source, &["branch", "--show-current"])?,
            before_branch
        );
        assert_eq!(git_output(&source, &["rev-parse", "HEAD"])?, before_head);
        assert_eq!(
            git_output(&source, &["status", "--porcelain=v1"])?,
            before_status
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn clone_is_private_and_archive_preserves_it() -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("archive")?;
        let provisioner = WorktreeProvisioner::new(base.join("private"))?;
        let record = provisioner.create_isolated_worktree(&registry, &request("agent"))?;
        assert!(
            Path::new(&record.capability.path).starts_with(fs::canonicalize(base.join("private"))?)
        );
        assert_eq!(
            provisioner.archive(&record.capability.id)?.state,
            IsolatedWorktreeState::Archived
        );
        assert!(
            provisioner
                .remove_isolated_worktree(&registry, &record.capability.id)
                .is_err()
        );
        assert!(Path::new(&record.capability.path).exists());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn non_git_source_is_bounded_copied_without_aliasing() -> Result<(), Box<dyn std::error::Error>>
    {
        let base = std::env::temp_dir().join(format!("md-private-copy-{}", std::process::id()));
        if base.exists() {
            fs::remove_dir_all(&base)?;
        }
        let source = base.join("source");
        fs::create_dir_all(&source)?;
        fs::write(source.join("note.txt"), "source")?;
        let registry = WorkspaceRegistry::from_paths([source.clone()]);
        let provisioner = WorktreeProvisioner::new(base.join("private"))?;
        let private = provisioner.create_isolated_worktree(&registry, &request("copy"))?;
        fs::write(
            Path::new(&private.capability.path).join("note.txt"),
            "private",
        )?;
        assert_eq!(fs::read_to_string(source.join("note.txt"))?, "source");
        assert!(
            provisioner
                .remove_isolated_worktree(&registry, &private.capability.id)
                .is_err()
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn path_like_name_is_rejected_before_copy() -> Result<(), Box<dyn std::error::Error>> {
        let (base, registry) = fixture("invalid-name")?;
        let provisioner = WorktreeProvisioner::new(base.join("private"))?;
        assert!(
            provisioner
                .create_isolated_worktree(&registry, &request("../../escape"))
                .is_err()
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }
}
