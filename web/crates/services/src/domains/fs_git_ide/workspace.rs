use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use md_web_contracts::domains::fs_git_ide::{WorkspaceId, WorkspaceSummary};

use super::DomainError;

#[derive(Debug)]
struct WorkspaceRecord {
    summary: WorkspaceSummary,
    canonical_root: PathBuf,
}

/// Process-owned capability table. Browser input can select an ID, never a root path.
#[derive(Debug, Default)]
pub struct WorkspaceRegistry {
    records: Vec<WorkspaceRecord>,
}

impl WorkspaceRegistry {
    /// Builds a deterministic registry from existing, non-symlink directories.
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut canonical = BTreeSet::new();
        for path in paths {
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            if let Ok(root) = std::fs::canonicalize(path) {
                canonical.insert(root);
            }
        }

        let records = canonical
            .into_iter()
            .enumerate()
            .map(|(index, canonical_root)| {
                let name = canonical_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("workspace");
                WorkspaceRecord {
                    summary: WorkspaceSummary {
                        id: WorkspaceId(format!("workspace-{}", index + 1)),
                        name: String::from(name),
                        display_path: canonical_root.to_string_lossy().into_owned(),
                    },
                    canonical_root,
                }
            })
            .collect();
        Self { records }
    }

    /// Loads roots from the platform path-list environment used by the web host.
    pub fn from_environment() -> Self {
        let value = std::env::var_os("MD_REGISTERED_REPOS").unwrap_or_default();
        Self::from_paths(std::env::split_paths(&value))
    }

    /// Returns presentation-only workspace records.
    pub fn list(&self) -> Vec<WorkspaceSummary> {
        self.records
            .iter()
            .map(|record| record.summary.clone())
            .collect()
    }

    pub(crate) fn resolve(&self, id: &WorkspaceId) -> Result<&Path, DomainError> {
        self.records
            .iter()
            .find(|record| record.summary.id == *id)
            .map(|record| record.canonical_root.as_path())
            .ok_or(DomainError::InvalidWorkspace)
    }

    pub(crate) fn summary_for_canonical_root(&self, root: &Path) -> Option<WorkspaceSummary> {
        self.records
            .iter()
            .find(|record| record.canonical_root == root)
            .map(|record| record.summary.clone())
    }

    pub(crate) fn authorize_absolute(&self, path: &Path) -> Result<PathBuf, DomainError> {
        if !path.is_absolute() {
            return Err(DomainError::InvalidPath);
        }
        let canonical = std::fs::canonicalize(path).map_err(|_| DomainError::NotFound)?;
        let authorized = self
            .records
            .iter()
            .any(|record| canonical.starts_with(&record.canonical_root));
        authorized
            .then_some(canonical)
            .ok_or(DomainError::InvalidPath)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use md_web_contracts::domains::fs_git_ide::WorkspaceId;

    use super::WorkspaceRegistry;

    fn task_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("md-fs-git-ide-{name}-{}", std::process::id()))
    }

    #[test]
    fn unavailable_roots_are_not_capabilities() -> Result<(), Box<dyn std::error::Error>> {
        let root = task_dir("missing-root");
        let registry = WorkspaceRegistry::from_paths([root]);
        assert!(registry.list().is_empty());
        Ok(())
    }

    #[test]
    fn registered_directory_resolves_by_id() -> Result<(), Box<dyn std::error::Error>> {
        let root = task_dir("registered-root");
        fs::create_dir_all(&root)?;
        let registry = WorkspaceRegistry::from_paths([root.clone()]);
        let resolved = registry.resolve(&WorkspaceId(String::from("workspace-1")))?;
        assert_eq!(resolved, fs::canonicalize(&root)?);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn unknown_id_is_rejected() {
        let registry = WorkspaceRegistry::default();
        assert!(
            registry
                .resolve(&WorkspaceId(String::from("workspace-1")))
                .is_err()
        );
    }

    #[test]
    fn environment_registry_is_always_queryable() {
        let _ = WorkspaceRegistry::from_environment().list();
    }
}
