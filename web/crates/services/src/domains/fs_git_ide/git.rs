use std::fs;
use std::path::Path;
use std::time::Duration;

use md_web_contracts::domains::fs_git_ide::{
    CheckoutRequest, CheckoutResult, GitAheadBehind, GitBranch, GitCommit, GitCompare, GitDiff,
    GitFileAtRevision, GitFileChange, GitOverview, GitStatus, GitStatusEntry, GitWorktree,
    WorkspaceId, WorkspaceSummary,
};

use super::command::run_command;
use super::fs::secure_existing_path;
use super::{DomainError, WorkspaceRegistry};

pub(super) const GIT_TIMEOUT: Duration = Duration::from_secs(8);
pub(super) const GIT_LONG_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_DIFF_BYTES: u64 = 2 * 1024 * 1024;

/// Local Git reader plus one explicitly-confirmed local checkout operation.
pub struct GitService;

impl GitService {
    pub fn main_repository(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspaceSummary>, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let output = match run_git(
            root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            GIT_TIMEOUT,
        ) {
            Ok(output) => output,
            Err(_) => return Ok(None),
        };
        let common = Path::new(output.stdout.trim());
        let repository = if common.file_name().is_some_and(|name| name == ".git") {
            common.parent().ok_or(DomainError::InvalidResponse)?
        } else {
            common
        };
        let canonical = fs::canonicalize(repository).map_err(|_| DomainError::InvalidResponse)?;
        Ok(registry.summary_for_canonical_root(&canonical))
    }

    pub fn is_repo(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
    ) -> Result<bool, DomainError> {
        let root = registry.resolve(workspace_id)?;
        Ok(
            run_git(root, &["rev-parse", "--is-inside-work-tree"], GIT_TIMEOUT)
                .is_ok_and(|output| output.stdout.trim() == "true"),
        )
    }

    pub fn overview(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        max_commits: u16,
    ) -> Result<GitOverview, DomainError> {
        let root = registry.resolve(workspace_id)?;
        if !run_git(root, &["rev-parse", "--is-inside-work-tree"], GIT_TIMEOUT)
            .is_ok_and(|output| output.stdout.trim() == "true")
        {
            return Ok(GitOverview {
                is_repo: false,
                branch: None,
                status: None,
                commits: Vec::new(),
                local_branches: Vec::new(),
                remote_branches: Vec::new(),
                ahead_behind: None,
            });
        }
        let branch = branch(root)?;
        let status = status(root)?;
        let commits = log(root, max_commits.clamp(1, 500), 0)?;
        let (local_branches, remote_branches) = branches(root)?;
        let ahead_behind = ahead_behind(root)?;
        Ok(GitOverview {
            is_repo: true,
            branch: Some(branch),
            status: Some(status),
            commits,
            local_branches,
            remote_branches,
            ahead_behind: Some(ahead_behind),
        })
    }

    pub fn diff(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        rel_path: &str,
    ) -> Result<GitDiff, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let working_path = match secure_existing_path(root, rel_path) {
            Ok(path) => Some(path),
            Err(DomainError::NotFound) => None,
            Err(error) => return Err(error),
        };
        let head = run_git(root, &["show", &format!("HEAD:{rel_path}")], GIT_TIMEOUT).ok();
        let head_exists = head.is_some();
        let mut head_text = head.map_or_else(String::new, |output| output.stdout);
        let mut working = String::new();
        let mut working_exists = false;
        let mut is_binary = head_text.as_bytes().contains(&0);
        if let Some(path) = working_path {
            let metadata = fs::metadata(&path).map_err(|_| DomainError::Io)?;
            if !metadata.is_file() {
                return Err(DomainError::NotRegularFile);
            }
            if metadata.len() > MAX_DIFF_BYTES {
                return Err(DomainError::FileTooLarge);
            }
            let bytes = fs::read(path).map_err(|_| DomainError::Io)?;
            working_exists = true;
            is_binary |= bytes.contains(&0);
            if !is_binary {
                working = String::from_utf8(bytes).map_err(|_| DomainError::BinaryFile)?;
            }
        }
        if is_binary {
            head_text.clear();
            working.clear();
        }
        Ok(GitDiff {
            rel_path: String::from(rel_path),
            head: head_text,
            working,
            head_exists,
            working_exists,
            is_binary,
        })
    }

    pub fn history(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        count: u16,
        skip: u32,
    ) -> Result<Vec<GitCommit>, DomainError> {
        let root = registry.resolve(workspace_id)?;
        log(root, count.clamp(1, 500), skip)
    }

    pub fn commit_files(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        revision: &str,
    ) -> Result<Vec<GitFileChange>, DomainError> {
        validate_revision(revision)?;
        let root = registry.resolve(workspace_id)?;
        let output = run_git(
            root,
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--root",
                "-z",
                "-M",
                "--name-status",
                revision,
                "--",
            ],
            GIT_LONG_TIMEOUT,
        )?;
        parse_name_status(&output.stdout)
    }

    pub fn file_at_revision(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        revision: &str,
        rel_path: &str,
    ) -> Result<GitFileAtRevision, DomainError> {
        validate_revision(revision)?;
        let root = registry.resolve(workspace_id)?;
        validate_relative_path(rel_path)?;
        let object = format!("{revision}:{rel_path}");
        let Ok(size) = run_git(root, &["cat-file", "-s", &object], GIT_TIMEOUT) else {
            return Ok(GitFileAtRevision {
                exists: false,
                is_binary: false,
                content: String::new(),
            });
        };
        if size.stdout.trim().parse::<u64>().unwrap_or(u64::MAX) > MAX_DIFF_BYTES {
            return Err(DomainError::FileTooLarge);
        }
        let output = run_git(root, &["show", &object], GIT_TIMEOUT)?;
        let is_binary = output.stdout.as_bytes().contains(&0);
        Ok(GitFileAtRevision {
            exists: true,
            is_binary,
            content: if is_binary {
                String::new()
            } else {
                output.stdout
            },
        })
    }

    pub fn compare(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        base: &str,
        head: &str,
        three_dot: bool,
    ) -> Result<GitCompare, DomainError> {
        validate_revision(base)?;
        validate_revision(head)?;
        let root = registry.resolve(workspace_id)?;
        let counts = run_git(
            root,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("{base}...{head}"),
            ],
            GIT_TIMEOUT,
        )?;
        let (behind, ahead) = parse_counts(&counts.stdout)?;
        let merge_base = run_git(root, &["merge-base", base, head], GIT_TIMEOUT)
            .ok()
            .map(|output| output.stdout.trim().to_string())
            .filter(|value| !value.is_empty());
        let range = if three_dot {
            format!("{base}...{head}")
        } else {
            format!("{base}..{head}")
        };
        let output = run_git(
            root,
            &["diff", "-z", "-M", "--name-status", &range, "--"],
            GIT_LONG_TIMEOUT,
        )?;
        Ok(GitCompare {
            ahead,
            behind,
            merge_base,
            files: parse_name_status(&output.stdout)?,
        })
    }

    pub fn worktrees(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<GitWorktree>, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let output = run_git(root, &["worktree", "list", "--porcelain"], GIT_TIMEOUT)?;
        Ok(parse_worktrees(&output.stdout))
    }

    pub fn checkout_local(
        registry: &WorkspaceRegistry,
        request: &CheckoutRequest,
        repo_busy: bool,
    ) -> Result<CheckoutResult, DomainError> {
        if !request.confirmed {
            return Err(DomainError::ConfirmationRequired);
        }
        if repo_busy {
            return Err(DomainError::BusyWorktree);
        }
        validate_revision(&request.reference)?;
        let root = registry.resolve_mutable(&request.workspace_id)?;
        let current = status(root)?;
        if !current.staged.is_empty()
            || !current.unstaged.is_empty()
            || !current.untracked.is_empty()
        {
            return Err(DomainError::DirtyWorktree);
        }
        if request.detach {
            run_git(
                root,
                &["switch", "--detach", &request.reference],
                GIT_LONG_TIMEOUT,
            )?;
        } else {
            run_git(root, &["switch", &request.reference], GIT_LONG_TIMEOUT)?;
        }
        Ok(CheckoutResult {
            detached: request.detach,
        })
    }
}

pub(super) fn run_git(
    cwd: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<super::command::CommandOutput, DomainError> {
    let mut owned = Vec::with_capacity(args.len() + 2);
    owned.push(String::from("-c"));
    owned.push(String::from(if cfg!(windows) {
        "core.hooksPath=NUL"
    } else {
        "core.hooksPath=/dev/null"
    }));
    owned.extend(args.iter().map(|arg| String::from(*arg)));
    run_command("git", cwd, &owned, timeout)
}

fn branch(root: &Path) -> Result<GitBranch, DomainError> {
    let output = run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"], GIT_TIMEOUT)?;
    let branch = output.stdout.trim();
    Ok(GitBranch {
        current: (branch != "HEAD").then(|| String::from(branch)),
        detached: branch == "HEAD",
    })
}

pub(super) fn status(root: &Path) -> Result<GitStatus, DomainError> {
    let output = run_git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        GIT_TIMEOUT,
    )?;
    parse_status(&output.stdout)
}

fn parse_status(output: &str) -> Result<GitStatus, DomainError> {
    let tokens: Vec<&str> = output
        .split('\0')
        .filter(|token| !token.is_empty())
        .collect();
    let mut result = GitStatus::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        let bytes = token.as_bytes();
        if bytes.len() < 3 {
            return Err(DomainError::InvalidResponse);
        }
        let index_status = char::from(bytes[0]);
        let worktree_status = char::from(bytes[1]);
        let path = String::from(&token[3..]);
        if index_status == '?' && worktree_status == '?' {
            result.untracked.push(path);
        } else {
            let entry = GitStatusEntry {
                path,
                index: index_status,
                worktree: worktree_status,
            };
            if index_status != ' ' && index_status != '?' {
                result.staged.push(entry.clone());
            }
            if worktree_status != ' ' && worktree_status != '?' {
                result.unstaged.push(entry);
            }
        }
        if matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C') {
            index += 1;
        }
        index += 1;
    }
    Ok(result)
}

fn log(root: &Path, count: u16, skip: u32) -> Result<Vec<GitCommit>, DomainError> {
    let format = "%H%x1f%P%x1f%s%x1f%an%x1f%at%x1f%D%x1e";
    let count_arg = format!("--max-count={count}");
    let skip_arg = format!("--skip={skip}");
    let mut args = vec!["log", "--all", "--topo-order", count_arg.as_str()];
    if skip > 0 {
        args.push(skip_arg.as_str());
    }
    let pretty = format!("--pretty=format:{format}");
    args.push(pretty.as_str());
    let output = run_git(root, &args, GIT_LONG_TIMEOUT)?;
    parse_log(&output.stdout)
}

fn parse_log(output: &str) -> Result<Vec<GitCommit>, DomainError> {
    let mut commits = Vec::new();
    for record in output
        .split('\u{1e}')
        .filter(|record| !record.trim().is_empty())
    {
        let fields: Vec<&str> = record.trim_start().split('\u{1f}').collect();
        if fields.len() < 6 || fields[0].is_empty() {
            return Err(DomainError::InvalidResponse);
        }
        let sha = String::from(fields[0]);
        commits.push(GitCommit {
            short_sha: sha.chars().take(7).collect(),
            sha,
            parents: fields[1]
                .split(' ')
                .filter(|value| !value.is_empty())
                .map(String::from)
                .collect(),
            subject: String::from(fields[2]),
            author: String::from(fields[3]),
            time: fields[4]
                .parse()
                .map_err(|_| DomainError::InvalidResponse)?,
            refs: fields[5]
                .split(", ")
                .filter(|value| !value.is_empty())
                .map(String::from)
                .collect(),
        });
    }
    Ok(commits)
}

fn branches(root: &Path) -> Result<(Vec<String>, Vec<String>), DomainError> {
    let output = run_git(
        root,
        &["branch", "-a", "--format=%(refname:short)"],
        GIT_TIMEOUT,
    )?;
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(name) = line.strip_prefix("remotes/") {
            remote.push(String::from(name));
        } else {
            local.push(String::from(line));
        }
    }
    Ok((local, remote))
}

fn ahead_behind(root: &Path) -> Result<GitAheadBehind, DomainError> {
    let Ok(upstream) = run_git(
        root,
        &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
        GIT_TIMEOUT,
    ) else {
        return Ok(GitAheadBehind {
            ahead: 0,
            behind: 0,
            upstream: None,
        });
    };
    let upstream = upstream.stdout.trim();
    let output = run_git(
        root,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("HEAD...{upstream}"),
        ],
        GIT_TIMEOUT,
    )?;
    let (ahead, behind) = parse_counts(&output.stdout)?;
    Ok(GitAheadBehind {
        ahead,
        behind,
        upstream: Some(String::from(upstream)),
    })
}

fn parse_counts(output: &str) -> Result<(u64, u64), DomainError> {
    let mut parts = output.split_whitespace();
    let left = parts
        .next()
        .ok_or(DomainError::InvalidResponse)?
        .parse()
        .map_err(|_| DomainError::InvalidResponse)?;
    let right = parts
        .next()
        .ok_or(DomainError::InvalidResponse)?
        .parse()
        .map_err(|_| DomainError::InvalidResponse)?;
    Ok((left, right))
}

fn parse_name_status(output: &str) -> Result<Vec<GitFileChange>, DomainError> {
    let tokens: Vec<&str> = output
        .split('\0')
        .filter(|token| !token.is_empty())
        .collect();
    let mut files = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let status = tokens[index]
            .chars()
            .next()
            .ok_or(DomainError::InvalidResponse)?;
        if matches!(status, 'R' | 'C') {
            let old_path = tokens.get(index + 1).ok_or(DomainError::InvalidResponse)?;
            let path = tokens.get(index + 2).ok_or(DomainError::InvalidResponse)?;
            files.push(GitFileChange {
                path: String::from(*path),
                status,
                old_path: Some(String::from(*old_path)),
            });
            index += 3;
        } else {
            let path = tokens.get(index + 1).ok_or(DomainError::InvalidResponse)?;
            files.push(GitFileChange {
                path: String::from(*path),
                status,
                old_path: None,
            });
            index += 2;
        }
    }
    Ok(files)
}

fn parse_worktrees(output: &str) -> Vec<GitWorktree> {
    output
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut head = String::new();
            let mut branch = None;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    path = Some(String::from(value));
                } else if let Some(value) = line.strip_prefix("HEAD ") {
                    head = String::from(value);
                } else if let Some(value) = line.strip_prefix("branch refs/heads/") {
                    branch = Some(String::from(value));
                }
            }
            path.map(|path| GitWorktree { path, head, branch })
        })
        .collect()
}

pub(super) fn validate_revision(revision: &str) -> Result<(), DomainError> {
    let valid = !revision.is_empty()
        && revision.len() <= 256
        && !revision.starts_with('-')
        && revision.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'.' | b'_' | b'/' | b'^' | b'~' | b'@' | b'{' | b'}' | b'-'
                )
        });
    valid.then_some(()).ok_or(DomainError::InvalidRevision)
}

fn validate_relative_path(rel_path: &str) -> Result<(), DomainError> {
    let path = Path::new(rel_path);
    if rel_path.is_empty()
        || rel_path.contains('\0')
        || path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(DomainError::InvalidPath);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::fs_git_ide::{CheckoutRequest, WorkspaceId};

    use super::{
        GitService, WorkspaceRegistry, parse_counts, parse_log, parse_name_status, parse_status,
        validate_revision,
    };

    #[test]
    fn option_injection_revision_is_rejected() {
        assert!(validate_revision("--upload-pack=evil").is_err());
    }

    #[test]
    fn status_splits_staged_and_untracked() -> Result<(), super::DomainError> {
        let status = parse_status("M  src/main.rs\0?? notes.txt\0")?;
        assert_eq!((status.staged.len(), status.untracked.len()), (1, 1));
        Ok(())
    }

    #[test]
    fn log_parses_one_commit() -> Result<(), super::DomainError> {
        let log = parse_log(
            "abcdef012345\u{1f}\u{1f}subject\u{1f}author\u{1f}42\u{1f}HEAD -> main\u{1e}",
        )?;
        assert_eq!(
            log.first().map(|commit| commit.short_sha.as_str()),
            Some("abcdef0")
        );
        Ok(())
    }

    #[test]
    fn rename_preserves_old_path() -> Result<(), super::DomainError> {
        let files = parse_name_status("R100\0old.txt\0new.txt\0")?;
        assert_eq!(
            files.first().and_then(|file| file.old_path.as_deref()),
            Some("old.txt")
        );
        Ok(())
    }

    #[test]
    fn counts_require_both_sides() {
        assert!(parse_counts("1").is_err());
    }

    #[test]
    fn public_reads_reject_unknown_workspace_before_running_git() {
        let registry = WorkspaceRegistry::default();
        let id = WorkspaceId(String::from("workspace-404"));
        assert!(GitService::is_repo(&registry, &id).is_err());
        assert!(GitService::main_repository(&registry, &id).is_err());
        assert!(GitService::overview(&registry, &id, 20).is_err());
        assert!(GitService::diff(&registry, &id, "src/main.rs").is_err());
        assert!(GitService::history(&registry, &id, 20, 0).is_err());
        assert!(GitService::commit_files(&registry, &id, "HEAD").is_err());
        assert!(GitService::file_at_revision(&registry, &id, "HEAD", "src/main.rs").is_err());
        assert!(GitService::compare(&registry, &id, "HEAD~1", "HEAD", true).is_err());
        assert!(GitService::worktrees(&registry, &id).is_err());
    }

    #[test]
    fn checkout_requires_explicit_confirmation_before_workspace_access() {
        let request = CheckoutRequest {
            workspace_id: WorkspaceId(String::from("workspace-404")),
            reference: String::from("main"),
            detach: false,
            confirmed: false,
        };
        assert!(
            GitService::checkout_local(&WorkspaceRegistry::default(), &request, false).is_err()
        );
    }

    #[test]
    fn main_repository_returns_only_a_registered_root() -> Result<(), Box<dyn std::error::Error>> {
        let root =
            std::env::temp_dir().join(format!("md-main-repository-test-{}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()?;
        assert!(status.success());
        let registry = WorkspaceRegistry::from_paths([root.clone()]);
        let id = registry
            .list()
            .first()
            .ok_or("missing workspace")?
            .id
            .clone();
        let main = GitService::main_repository(&registry, &id)?.ok_or("missing main repo")?;
        assert_eq!(main.id, id);
        std::fs::remove_dir_all(root)?;
        Ok(())
    }
}
