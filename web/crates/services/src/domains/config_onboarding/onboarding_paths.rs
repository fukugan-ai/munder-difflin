use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use md_web_contracts::domains::config_onboarding::{
    OnboardingPathProbeRequest, ValidatedOnboardingPaths,
};

/// Failure proving a browser-supplied path against server-owned roots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OnboardingPathError {
    HomeUnavailable,
    PathNotAbsolute,
    OutsideAllowedRoots,
    NotDirectory,
    NotGitRepository,
}

impl Display for OnboardingPathError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::HomeUnavailable => "server home is unavailable",
            Self::PathNotAbsolute => "onboarding path must be absolute",
            Self::OutsideAllowedRoots => "onboarding path is outside allowed roots",
            Self::NotDirectory => "onboarding path is not an existing directory",
            Self::NotGitRepository => "registered workspace is not a Git repository",
        })
    }
}

impl std::error::Error for OnboardingPathError {}

/// Expands `~`, canonicalizes existing directories and confines them to server roots.
pub fn validate_onboarding_paths(
    request: &OnboardingPathProbeRequest,
    allowed_roots: &[PathBuf],
    server_home: &Path,
) -> Result<ValidatedOnboardingPaths, OnboardingPathError> {
    let canonical_roots = allowed_roots
        .iter()
        .filter_map(|root| root.canonicalize().ok())
        .filter(|root| root.is_dir())
        .collect::<Vec<_>>();
    if canonical_roots.is_empty() {
        return Err(OnboardingPathError::OutsideAllowedRoots);
    }
    let harness_home = canonical_confined(
        &expand_home(&request.harness_home, server_home)?,
        &canonical_roots,
    )?;
    let mut repositories = BTreeSet::new();
    for repository in &request.registered_repos {
        let canonical =
            canonical_confined(&expand_home(repository, server_home)?, &canonical_roots)?;
        if !canonical.join(".git").exists() {
            return Err(OnboardingPathError::NotGitRepository);
        }
        repositories.insert(canonical.to_string_lossy().into_owned());
    }
    let workspace_cwd = request
        .workspace_cwd
        .as_deref()
        .map(|workspace| {
            canonical_confined(&expand_home(workspace, server_home)?, &canonical_roots)
                .map(|path| path.to_string_lossy().into_owned())
        })
        .transpose()?;
    if workspace_cwd
        .as_ref()
        .is_some_and(|workspace| !repositories.contains(workspace))
    {
        return Err(OnboardingPathError::NotGitRepository);
    }
    Ok(ValidatedOnboardingPaths {
        harness_home: harness_home.to_string_lossy().into_owned(),
        registered_repos: repositories.into_iter().collect(),
        workspace_cwd,
    })
}

fn expand_home(value: &str, server_home: &Path) -> Result<PathBuf, OnboardingPathError> {
    let value = value.trim();
    if value == "~" {
        return Ok(server_home.to_path_buf());
    }
    if let Some(relative) = value.strip_prefix("~/") {
        return Ok(server_home.join(relative));
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(OnboardingPathError::PathNotAbsolute)
    }
}

fn canonical_confined(
    path: &Path,
    canonical_roots: &[PathBuf],
) -> Result<PathBuf, OnboardingPathError> {
    let canonical = path
        .canonicalize()
        .map_err(|_| OnboardingPathError::NotDirectory)?;
    if !canonical.is_dir() {
        return Err(OnboardingPathError::NotDirectory);
    }
    if !canonical_roots
        .iter()
        .any(|root| canonical.starts_with(root))
    {
        return Err(OnboardingPathError::OutsideAllowedRoots);
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use md_web_contracts::domains::config_onboarding::OnboardingPathProbeRequest;

    use super::{OnboardingPathError, validate_onboarding_paths};

    fn fixture(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!(
            "md-config-onboarding-{name}-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;
        Ok(root)
    }

    #[test]
    fn untouched_home_default_expands_on_server() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture("home-default")?;
        let request = OnboardingPathProbeRequest {
            harness_home: String::from("~"),
            registered_repos: Vec::new(),
            workspace_cwd: None,
        };

        let result = validate_onboarding_paths(&request, std::slice::from_ref(&root), &root)?;

        assert_eq!(result.harness_home, root.canonicalize()?.to_string_lossy());
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn relative_home_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture("relative")?;
        let request = OnboardingPathProbeRequest {
            harness_home: String::from("workspace"),
            registered_repos: Vec::new(),
            workspace_cwd: None,
        };

        let result = validate_onboarding_paths(&request, std::slice::from_ref(&root), &root);

        assert_eq!(result, Err(OnboardingPathError::PathNotAbsolute));
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn git_repository_is_canonicalized_and_deduplicated() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture("repository")?;
        let repository = root.join("project");
        fs::create_dir_all(repository.join(".git"))?;
        let request = OnboardingPathProbeRequest {
            harness_home: root.to_string_lossy().into_owned(),
            registered_repos: vec![
                repository.to_string_lossy().into_owned(),
                repository.to_string_lossy().into_owned(),
            ],
            workspace_cwd: Some(repository.to_string_lossy().into_owned()),
        };

        let result = validate_onboarding_paths(&request, std::slice::from_ref(&root), &root)?;

        assert_eq!(result.registered_repos.len(), 1);
        assert_eq!(
            result.workspace_cwd,
            result.registered_repos.first().cloned()
        );
        assert_eq!(
            result.registered_repos.first().map(String::as_str),
            Some(repository.canonicalize()?.to_string_lossy().as_ref())
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn ordinary_directory_is_not_registered_as_repository() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture("not-git")?;
        let directory = root.join("folder");
        fs::create_dir_all(&directory)?;
        let request = OnboardingPathProbeRequest {
            harness_home: root.to_string_lossy().into_owned(),
            registered_repos: vec![directory.to_string_lossy().into_owned()],
            workspace_cwd: Some(directory.to_string_lossy().into_owned()),
        };

        let result = validate_onboarding_paths(&request, std::slice::from_ref(&root), &root);

        assert_eq!(result, Err(OnboardingPathError::NotGitRepository));
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
