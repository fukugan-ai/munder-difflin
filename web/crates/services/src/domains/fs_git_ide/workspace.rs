use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use md_web_contracts::domains::fs_git_ide::{
    PrivateWorkspaceCapability, WorkspaceCapability, WorkspaceId, WorkspaceSummary,
};

use super::DomainError;

#[derive(Clone, Debug)]
pub struct PrivateWorkspaceRoot {
    canonical_root: PathBuf,
}

impl PrivateWorkspaceRoot {
    /// Creates a server-owned authority root. A path beneath a source never grants this authority.
    pub fn new(root: PathBuf) -> Result<Self, DomainError> {
        if !root.is_absolute() {
            return Err(DomainError::InvalidPath);
        }
        std::fs::create_dir_all(&root).map_err(|_| DomainError::Io)?;
        let metadata = std::fs::symlink_metadata(&root).map_err(|_| DomainError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(DomainError::InvalidPath);
        }
        Ok(Self {
            canonical_root: std::fs::canonicalize(root).map_err(|_| DomainError::Io)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.canonical_root
    }

    pub(crate) fn authorize(&self, path: PathBuf) -> Option<PathBuf> {
        let canonical = std::fs::canonicalize(path).ok()?;
        (canonical != self.canonical_root && canonical.starts_with(&self.canonical_root))
            .then_some(canonical)
    }
}

#[derive(Debug)]
enum WorkspaceRoot {
    SourceReadOnly(PathBuf),
    PrivateMutable(PathBuf),
}

impl WorkspaceRoot {
    fn path(&self) -> &Path {
        match self {
            Self::SourceReadOnly(path) | Self::PrivateMutable(path) => path,
        }
    }

    fn capability(&self) -> WorkspaceCapability {
        match self {
            Self::SourceReadOnly(_) => WorkspaceCapability::SourceReadOnly,
            Self::PrivateMutable(_) => WorkspaceCapability::PrivateMutable,
        }
    }
}

#[derive(Debug)]
struct WorkspaceRecord {
    summary: WorkspaceSummary,
    root: WorkspaceRoot,
}

/// Process-owned capability table. Registered paths are read-only sources. Mutable roots can only
/// enter through an explicit [`PrivateWorkspaceRoot`] authority.
#[derive(Debug, Default)]
pub struct WorkspaceRegistry {
    records: Vec<WorkspaceRecord>,
}

impl WorkspaceRegistry {
    /// Compatibility constructor. Every input is deliberately a source-read-only capability.
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::from_source_paths(paths)
    }

    pub fn from_source_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut canonical = BTreeSet::new();
        for path in paths {
            if let Some(root) = canonical_directory(path) {
                canonical.insert(root);
            }
        }
        let records = canonical
            .into_iter()
            .enumerate()
            .map(|(index, root)| record(root, format!("source-{}", index + 1), false))
            .collect();
        Self { records }
    }

    /// Admits only complete provisioner-issued identities proven by the app-owned root authority.
    pub fn with_private_workspaces(
        mut self,
        authority: &PrivateWorkspaceRoot,
        capabilities: impl IntoIterator<Item = PrivateWorkspaceCapability>,
    ) -> Self {
        for capability in capabilities {
            let source_is_registered = self
                .record(&capability.source_workspace_id)
                .is_ok_and(|record| matches!(record.root, WorkspaceRoot::SourceReadOnly(_)));
            let Some(root) = authority.authorize(PathBuf::from(&capability.path)) else {
                continue;
            };
            let identity_matches = capability.workspace_id.0
                == format!("private-{}", capability.id)
                && root.file_name().and_then(|name| name.to_str()) == Some(capability.id.as_str());
            if !source_is_registered
                || !identity_matches
                || self
                    .records
                    .iter()
                    .any(|record| record.summary.id == capability.workspace_id)
            {
                continue;
            }
            self.records
                .push(record(root, capability.workspace_id.0, true));
        }
        self
    }

    pub fn from_environment() -> Self {
        let value = std::env::var_os("MD_REGISTERED_REPOS").unwrap_or_default();
        Self::from_source_paths(std::env::split_paths(&value))
    }

    pub fn list(&self) -> Vec<WorkspaceSummary> {
        self.records
            .iter()
            .map(|record| record.summary.clone())
            .collect()
    }

    pub(crate) fn resolve(&self, id: &WorkspaceId) -> Result<&Path, DomainError> {
        Ok(self.record(id)?.root.path())
    }

    pub(crate) fn resolve_source(&self, id: &WorkspaceId) -> Result<&Path, DomainError> {
        match &self.record(id)?.root {
            WorkspaceRoot::SourceReadOnly(path) => Ok(path),
            WorkspaceRoot::PrivateMutable(_) => Err(DomainError::InvalidWorkspace),
        }
    }

    pub(crate) fn resolve_mutable(&self, id: &WorkspaceId) -> Result<&Path, DomainError> {
        match &self.record(id)?.root {
            WorkspaceRoot::PrivateMutable(path) => Ok(path),
            WorkspaceRoot::SourceReadOnly(_) => Err(DomainError::ReadOnlyWorkspace),
        }
    }

    fn record(&self, id: &WorkspaceId) -> Result<&WorkspaceRecord, DomainError> {
        self.records
            .iter()
            .find(|record| record.summary.id == *id)
            .ok_or(DomainError::InvalidWorkspace)
    }

    pub(crate) fn summary_for_canonical_root(&self, root: &Path) -> Option<WorkspaceSummary> {
        self.records
            .iter()
            .find(|record| record.root.path() == root)
            .map(|record| record.summary.clone())
    }

    pub(crate) fn authorize_absolute(&self, path: &Path) -> Result<PathBuf, DomainError> {
        if !path.is_absolute() {
            return Err(DomainError::InvalidPath);
        }
        let canonical = std::fs::canonicalize(path).map_err(|_| DomainError::NotFound)?;
        self.records
            .iter()
            .any(|record| canonical.starts_with(record.root.path()))
            .then_some(canonical)
            .ok_or(DomainError::InvalidPath)
    }
}

fn canonical_directory(path: PathBuf) -> Option<PathBuf> {
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    std::fs::canonicalize(path).ok()
}

fn record(root: PathBuf, id: String, mutable: bool) -> WorkspaceRecord {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    let workspace_root = if mutable {
        WorkspaceRoot::PrivateMutable(root.clone())
    } else {
        WorkspaceRoot::SourceReadOnly(root.clone())
    };
    WorkspaceRecord {
        summary: WorkspaceSummary {
            id: WorkspaceId(id),
            name: String::from(name),
            display_path: root.to_string_lossy().into_owned(),
            capability: workspace_root.capability(),
        },
        root: workspace_root,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use md_web_contracts::domains::fs_git_ide::{
        PrivateWorkspaceCapability, WorkspaceCapability, WorkspaceId,
    };

    use super::{PrivateWorkspaceRoot, WorkspaceRegistry};

    fn task_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("md-fs-git-ide-{name}-{}", std::process::id()))
    }

    #[test]
    fn registered_directory_is_read_only_source() -> Result<(), Box<dyn std::error::Error>> {
        let root = task_dir("registered-root");
        fs::create_dir_all(&root)?;
        let registry = WorkspaceRegistry::from_paths([root.clone()]);
        let id = WorkspaceId(String::from("source-1"));
        assert_eq!(registry.resolve_source(&id)?, fs::canonicalize(&root)?);
        assert!(matches!(
            registry.resolve_mutable(&id),
            Err(super::DomainError::ReadOnlyWorkspace)
        ));
        assert_eq!(
            registry.list()[0].capability,
            WorkspaceCapability::SourceReadOnly
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn private_path_requires_explicit_typed_authority() -> Result<(), Box<dyn std::error::Error>> {
        let base = task_dir("private-authority");
        let authority = PrivateWorkspaceRoot::new(base.join("owned"))?;
        let private = authority.path().join("wt-1");
        let outside = base.join("outside");
        let source = base.join("source");
        fs::create_dir_all(&private)?;
        fs::create_dir_all(&outside)?;
        fs::create_dir_all(&source)?;
        let registry = WorkspaceRegistry::from_paths([source]).with_private_workspaces(
            &authority,
            [
                PrivateWorkspaceCapability {
                    id: String::from("wt-1"),
                    workspace_id: WorkspaceId(String::from("private-wt-1")),
                    source_workspace_id: WorkspaceId(String::from("source-1")),
                    path: private.to_string_lossy().into_owned(),
                },
                PrivateWorkspaceCapability {
                    id: String::from("outside"),
                    workspace_id: WorkspaceId(String::from("private-outside")),
                    source_workspace_id: WorkspaceId(String::from("source-1")),
                    path: outside.to_string_lossy().into_owned(),
                },
            ],
        );
        let summary = registry.list().pop().ok_or("missing private")?;
        assert_eq!(summary.capability, WorkspaceCapability::PrivateMutable);
        assert_eq!(
            registry.resolve_mutable(&summary.id)?,
            fs::canonicalize(private)?
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn unavailable_roots_are_not_capabilities() {
        assert!(
            WorkspaceRegistry::from_paths([task_dir("missing-root")])
                .list()
                .is_empty()
        );
    }

    #[test]
    fn unknown_id_is_rejected() {
        assert!(
            WorkspaceRegistry::default()
                .resolve(&WorkspaceId(String::from("source-1")))
                .is_err()
        );
    }
}
