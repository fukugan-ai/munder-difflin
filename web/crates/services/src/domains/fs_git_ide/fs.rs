use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use md_web_contracts::domains::fs_git_ide::{
    AbsoluteFileStat, BinaryFile, DirEntry, FileStat, TextFile, WorkspaceId, WriteFileRequest,
    WriteFileResult,
};

use super::{DomainError, WorkspaceRegistry};

const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 10 * 1024 * 1024;

/// Sandboxed filesystem operations over server-owned workspace IDs.
pub struct FsService;

impl FsService {
    pub fn stat_absolute(
        registry: &WorkspaceRegistry,
        absolute_path: &str,
    ) -> Result<AbsoluteFileStat, DomainError> {
        let path = Path::new(absolute_path);
        match registry.authorize_absolute(path) {
            Ok(canonical) => {
                let metadata =
                    std::fs::symlink_metadata(&canonical).map_err(|_| DomainError::Io)?;
                Ok(AbsoluteFileStat {
                    exists: true,
                    is_file: metadata.is_file(),
                    path: canonical.to_string_lossy().into_owned(),
                })
            }
            Err(DomainError::NotFound) => Ok(AbsoluteFileStat {
                exists: false,
                is_file: false,
                path: String::new(),
            }),
            Err(error) => Err(error),
        }
    }

    pub fn list_dir(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        rel_path: &str,
    ) -> Result<Vec<DirEntry>, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let directory = secure_existing_path(root, rel_path)?;
        if !directory.is_dir() {
            return Err(DomainError::InvalidPath);
        }
        let read_dir = std::fs::read_dir(directory).map_err(|_| DomainError::Io)?;
        let mut entries = Vec::new();
        for entry in read_dir {
            let entry = entry.map_err(|_| DomainError::Io)?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|_| DomainError::Io)?;
            let symlink = metadata.file_type().is_symlink();
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok())
                .unwrap_or(0);
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: !symlink && metadata.is_dir(),
                size: if symlink { 0 } else { metadata.len() },
                mtime_ms,
            });
        }
        entries.sort_by(|left, right| {
            right
                .is_dir
                .cmp(&left.is_dir)
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(entries)
    }

    pub fn read_text(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        rel_path: &str,
    ) -> Result<TextFile, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let path = secure_existing_path(root, rel_path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| DomainError::NotFound)?;
        if !metadata.is_file() {
            return Err(DomainError::NotRegularFile);
        }
        if metadata.len() > MAX_TEXT_BYTES {
            return Err(DomainError::FileTooLarge);
        }
        let bytes = read_limited(&path, MAX_TEXT_BYTES)?;
        if bytes.contains(&0) {
            return Err(DomainError::BinaryFile);
        }
        let content = String::from_utf8(bytes).map_err(|_| DomainError::BinaryFile)?;
        Ok(TextFile {
            rel_path: String::from(rel_path),
            size: metadata.len(),
            content,
        })
    }

    pub fn read_binary(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        rel_path: &str,
    ) -> Result<BinaryFile, DomainError> {
        let root = registry.resolve(workspace_id)?;
        let path = secure_existing_path(root, rel_path)?;
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| DomainError::NotFound)?;
        if !metadata.is_file() {
            return Err(DomainError::NotRegularFile);
        }
        if metadata.len() > MAX_BINARY_BYTES {
            return Err(DomainError::FileTooLarge);
        }
        let bytes = read_limited(&path, MAX_BINARY_BYTES)?;
        Ok(BinaryFile {
            rel_path: String::from(rel_path),
            mime: mime_for_path(&path),
            size: metadata.len(),
            bytes,
        })
    }

    pub fn write_text(
        registry: &WorkspaceRegistry,
        request: &WriteFileRequest,
    ) -> Result<WriteFileResult, DomainError> {
        let byte_count =
            u64::try_from(request.content.len()).map_err(|_| DomainError::FileTooLarge)?;
        if byte_count > MAX_TEXT_BYTES {
            return Err(DomainError::FileTooLarge);
        }
        let root = registry.resolve_mutable(&request.workspace_id)?;
        let path = secure_write_path(root, &request.rel_path)?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|_| DomainError::Io)?;
        file.write_all(request.content.as_bytes())
            .map_err(|_| DomainError::Io)?;
        Ok(WriteFileResult {
            rel_path: request.rel_path.clone(),
            bytes_written: byte_count,
        })
    }

    pub fn stat(
        registry: &WorkspaceRegistry,
        workspace_id: &WorkspaceId,
        rel_path: &str,
    ) -> Result<FileStat, DomainError> {
        let root = registry.resolve(workspace_id)?;
        match secure_existing_path(root, rel_path) {
            Ok(path) => {
                let metadata =
                    std::fs::symlink_metadata(path).map_err(|_| DomainError::NotFound)?;
                Ok(FileStat {
                    exists: true,
                    is_file: metadata.is_file(),
                    rel_path: String::from(rel_path),
                })
            }
            Err(DomainError::NotFound) => Ok(FileStat {
                exists: false,
                is_file: false,
                rel_path: String::from(rel_path),
            }),
            Err(error) => Err(error),
        }
    }
}

pub(crate) fn secure_existing_path(root: &Path, rel_path: &str) -> Result<PathBuf, DomainError> {
    let candidate = lexical_join(root, rel_path)?;
    reject_symlink_components(root, &candidate, false)?;
    let canonical = std::fs::canonicalize(&candidate).map_err(|_| DomainError::NotFound)?;
    if !canonical.starts_with(root) {
        return Err(DomainError::InvalidPath);
    }
    Ok(canonical)
}

fn secure_write_path(root: &Path, rel_path: &str) -> Result<PathBuf, DomainError> {
    let candidate = lexical_join(root, rel_path)?;
    reject_symlink_components(root, &candidate, true)?;
    let parent = candidate.parent().ok_or(DomainError::InvalidPath)?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(|_| DomainError::InvalidPath)?;
    if !canonical_parent.starts_with(root) {
        return Err(DomainError::InvalidPath);
    }
    Ok(candidate)
}

fn lexical_join(root: &Path, rel_path: &str) -> Result<PathBuf, DomainError> {
    if rel_path.contains('\0') {
        return Err(DomainError::InvalidPath);
    }
    let rel = Path::new(rel_path);
    let mut candidate = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => candidate.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(DomainError::InvalidPath);
            }
        }
    }
    Ok(candidate)
}

fn reject_symlink_components(
    root: &Path,
    candidate: &Path,
    allow_missing_leaf: bool,
) -> Result<(), DomainError> {
    let relative = candidate
        .strip_prefix(root)
        .map_err(|_| DomainError::InvalidPath)?;
    let mut cursor = root.to_path_buf();
    let count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        cursor.push(component.as_os_str());
        match std::fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(DomainError::InvalidPath);
            }
            Ok(_) => {}
            Err(_) if allow_missing_leaf && index + 1 == count => {}
            Err(_) => return Err(DomainError::NotFound),
        }
    }
    Ok(())
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, DomainError> {
    let file = File::open(path).map_err(|_| DomainError::Io)?;
    let mut bytes = Vec::with_capacity(usize::try_from(limit.min(64 * 1024)).unwrap_or(0));
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DomainError::Io)?;
    if u64::try_from(bytes.len()).map_err(|_| DomainError::FileTooLarge)? > limit {
        return Err(DomainError::FileTooLarge);
    }
    Ok(bytes)
}

fn mime_for_path(path: &Path) -> String {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    String::from(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use md_web_contracts::domains::fs_git_ide::{
        PrivateWorkspaceCapability, WorkspaceId, WriteFileRequest,
    };

    use super::{FsService, WorkspaceRegistry};
    use crate::domains::fs_git_ide::PrivateWorkspaceRoot;

    fn workspace(
        name: &str,
    ) -> Result<(PathBuf, PathBuf, WorkspaceRegistry, WorkspaceId), Box<dyn std::error::Error>>
    {
        let base =
            std::env::temp_dir().join(format!("md-fs-service-{name}-{}", std::process::id()));
        if base.exists() {
            fs::remove_dir_all(&base)?;
        }
        let authority = PrivateWorkspaceRoot::new(base.join("owned"))?;
        let root = authority.path().join("wt-test");
        fs::create_dir_all(&root)?;
        let source = base.join("source");
        fs::create_dir_all(&source)?;
        let registry = WorkspaceRegistry::from_paths([source]).with_private_workspaces(
            &authority,
            [PrivateWorkspaceCapability {
                id: String::from("wt-test"),
                workspace_id: WorkspaceId(String::from("private-wt-test")),
                source_workspace_id: WorkspaceId(String::from("source-1")),
                path: root.to_string_lossy().into_owned(),
            }],
        );
        Ok((
            base,
            root,
            registry,
            WorkspaceId(String::from("private-wt-test")),
        ))
    }

    #[test]
    fn traversal_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let (base, _root, registry, id) = workspace("traversal")?;
        assert!(FsService::read_text(&registry, &id, "../outside").is_err());
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn text_round_trip_stays_inside_root() -> Result<(), Box<dyn std::error::Error>> {
        let (base, _root, registry, id) = workspace("round-trip")?;
        let request = WriteFileRequest {
            workspace_id: id.clone(),
            rel_path: String::from("notes.txt"),
            content: String::from("hello"),
        };
        FsService::write_text(&registry, &request)?;
        assert_eq!(
            FsService::read_text(&registry, &id, "notes.txt")?.content,
            "hello"
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn binary_file_is_not_text() -> Result<(), Box<dyn std::error::Error>> {
        let (base, root, registry, id) = workspace("binary")?;
        fs::write(root.join("image.png"), [0_u8, 1, 2])?;
        assert!(matches!(
            FsService::read_text(&registry, &id, "image.png"),
            Err(super::DomainError::BinaryFile)
        ));
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn list_directories_before_files() -> Result<(), Box<dyn std::error::Error>> {
        let (base, root, registry, id) = workspace("list")?;
        fs::write(root.join("a.txt"), "a")?;
        fs::create_dir(root.join("z-dir"))?;
        let entries = FsService::list_dir(&registry, &id, "")?;
        assert!(entries.first().is_some_and(|entry| entry.is_dir));
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn stat_reports_missing_leaf() -> Result<(), Box<dyn std::error::Error>> {
        let (base, _root, registry, id) = workspace("stat")?;
        assert!(!FsService::stat(&registry, &id, "missing.txt")?.exists);
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn binary_read_preserves_mime() -> Result<(), Box<dyn std::error::Error>> {
        let (base, root, registry, id) = workspace("mime")?;
        fs::write(root.join("image.PNG"), [1_u8, 2, 3])?;
        assert_eq!(
            FsService::read_binary(&registry, &id, "image.PNG")?.mime,
            "image/png"
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }

    #[test]
    fn absolute_stat_is_metadata_only_and_registry_confined()
    -> Result<(), Box<dyn std::error::Error>> {
        let (base, root, registry, _) = workspace("absolute-stat")?;
        let file = root.join("inside.txt");
        fs::write(&file, "inside")?;
        let stat = FsService::stat_absolute(&registry, &file.to_string_lossy())?;
        assert!(stat.exists && stat.is_file);
        assert!(
            FsService::stat_absolute(&registry, &std::env::temp_dir().to_string_lossy()).is_err()
        );
        fs::remove_dir_all(base)?;
        Ok(())
    }
}
