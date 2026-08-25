use std::path::Path;
use std::time::Duration;

use md_web_contracts::domains::fs_git_ide::{CiRun, GitHubIssue, WorkspaceId};

use super::command::run_command;
use super::{DomainError, WorkspaceRegistry};

const ALLOWED_REPO: &str = "fukugan-ai/munder-difflin";
const ORIGINAL_REPO: &str = "chaitanyagiri/munder-difflin";
const GH_TIMEOUT: Duration = Duration::from_secs(15);

/// Read-only GitHub ingestion pinned to the user's fork.
pub struct GitHubService;

impl GitHubService {
    pub fn issues(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<GitHubIssue>, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let repo = resolve_repo(root)?;
        let output = run_command("gh", root, &issue_args(&repo), GH_TIMEOUT)?;
        parse_issues(&output.stdout)
    }

    pub fn ci_runs(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<CiRun>, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let repo = resolve_repo(root)?;
        let output = run_command("gh", root, &ci_args(&repo), GH_TIMEOUT)?;
        parse_ci_runs(&output.stdout)
    }
}

/// Builds the only allowed issue command. It deliberately excludes issue bodies.
pub fn issue_args(repo: &str) -> Vec<String> {
    [
        "issue",
        "list",
        "--repo",
        repo,
        "--json",
        "number,title,assignees,labels,url,state",
        "--limit",
        "30",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Builds the only allowed Actions command. Listing does not dispatch or rerun CI.
pub fn ci_args(repo: &str) -> Vec<String> {
    [
        "run",
        "list",
        "--repo",
        repo,
        "--limit",
        "5",
        "--json",
        "name,status,conclusion,url,databaseId",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// Parses an origin URL and returns the canonical fork only.
pub fn parse_allowed_repo(remote: &str) -> Option<String> {
    let trimmed = remote.trim();
    let repo = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
        .or_else(|| trimmed.strip_prefix("git@github.com:"))?
        .strip_suffix(".git")
        .unwrap_or_else(|| {
            trimmed
                .strip_prefix("https://github.com/")
                .or_else(|| trimmed.strip_prefix("ssh://git@github.com/"))
                .or_else(|| trimmed.strip_prefix("git@github.com:"))
                .unwrap_or("")
        });
    let mut parts = repo.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some()
        || owner.is_empty()
        || name.is_empty()
        || !owner.bytes().all(valid_owner_byte)
        || !name.bytes().all(valid_repo_byte)
    {
        return None;
    }
    let normalized = format!("{owner}/{name}").to_ascii_lowercase();
    if normalized == ORIGINAL_REPO || normalized != ALLOWED_REPO {
        return None;
    }
    Some(String::from(ALLOWED_REPO))
}

fn valid_owner_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

fn valid_repo_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

fn resolve_repo(root: &Path) -> Result<String, DomainError> {
    let hooks = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let args = vec![
        String::from("-c"),
        format!("core.hooksPath={hooks}"),
        String::from("remote"),
        String::from("get-url"),
        String::from("origin"),
    ];
    let output = run_command("git", root, &args, Duration::from_secs(8))?;
    parse_allowed_repo(&output.stdout).ok_or(DomainError::RepositoryNotAllowed)
}

fn parse_issues(json: &str) -> Result<Vec<GitHubIssue>, DomainError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| DomainError::InvalidResponse)?;
    let rows = value.as_array().ok_or(DomainError::InvalidResponse)?;
    rows.iter()
        .map(|row| {
            Ok(GitHubIssue {
                number: row
                    .get("number")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                title: json_string(row, "title"),
                url: json_string(row, "url"),
                labels: json_names(row, "labels", "name"),
                assignees: json_names(row, "assignees", "login"),
            })
        })
        .collect()
}

fn parse_ci_runs(json: &str) -> Result<Vec<CiRun>, DomainError> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|_| DomainError::InvalidResponse)?;
    let rows = value.as_array().ok_or(DomainError::InvalidResponse)?;
    Ok(rows
        .iter()
        .map(|row| CiRun {
            name: json_string(row, "name"),
            status: json_string(row, "status"),
            conclusion: row
                .get("conclusion")
                .and_then(serde_json::Value::as_str)
                .map(String::from),
            url: json_string(row, "url"),
        })
        .collect())
}

fn json_string(row: &serde_json::Value, field: &str) -> String {
    row.get(field)
        .and_then(serde_json::Value::as_str)
        .map_or_else(String::new, String::from)
}

fn json_names(row: &serde_json::Value, field: &str, name_field: &str) -> Vec<String> {
    row.get(field)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.get(name_field).and_then(serde_json::Value::as_str))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::fs_git_ide::{WorkspaceId, WorkspaceSummary};

    use super::{
        GitHubService, ci_args, issue_args, parse_allowed_repo, parse_ci_runs, parse_issues,
    };
    use crate::domains::fs_git_ide::WorkspaceRegistry;

    #[test]
    fn fork_remote_is_allowed() {
        assert_eq!(
            parse_allowed_repo("git@github.com:fukugan-ai/munder-difflin.git").as_deref(),
            Some("fukugan-ai/munder-difflin")
        );
    }

    #[test]
    fn original_remote_is_refused() {
        assert!(
            parse_allowed_repo("https://github.com/chaitanyagiri/munder-difflin.git").is_none()
        );
    }

    #[test]
    fn issue_command_is_read_only_and_has_no_body() {
        let args = issue_args("fukugan-ai/munder-difflin").join(" ");
        assert!(args.starts_with("issue list --repo fukugan-ai/munder-difflin"));
        assert!(!args.contains("body"));
    }

    #[test]
    fn ci_command_cannot_dispatch_or_rerun() {
        let args = ci_args("fukugan-ai/munder-difflin").join(" ");
        assert!(args.starts_with("run list --repo fukugan-ai/munder-difflin"));
        assert!(
            !args.contains("rerun") && !args.contains("cancel") && !args.contains("workflow run")
        );
    }

    #[test]
    fn issue_response_is_normalized() -> Result<(), super::DomainError> {
        let issues = parse_issues(
            r#"[{"number":7,"title":"題名","url":"https://example.invalid/7","labels":[{"name":"bug"}],"assignees":[{"login":"aria"}]}]"#,
        )?;
        assert_eq!(
            issues.first().map(|issue| issue.labels.as_slice()),
            Some([String::from("bug")].as_slice())
        );
        Ok(())
    }

    #[test]
    fn ci_null_conclusion_is_preserved() -> Result<(), super::DomainError> {
        let runs = parse_ci_runs(
            r#"[{"name":"check","status":"in_progress","conclusion":null,"url":"https://example.invalid/run"}]"#,
        )?;
        assert!(runs.first().is_some_and(|run| run.conclusion.is_none()));
        Ok(())
    }

    #[test]
    fn github_calls_reject_unknown_workspace_before_network() {
        let registry = WorkspaceRegistry::default();
        let id = WorkspaceId(String::from("missing"));
        assert!(GitHubService::issues(&registry, &id).is_err());
        assert!(GitHubService::ci_runs(&registry, &id).is_err());
    }

    #[test]
    fn contract_type_remains_constructible() {
        let summary = WorkspaceSummary {
            id: WorkspaceId(String::from("workspace-1")),
            name: String::from("repo"),
            display_path: String::from("/repo"),
        };
        assert_eq!(summary.name, "repo");
    }
}
