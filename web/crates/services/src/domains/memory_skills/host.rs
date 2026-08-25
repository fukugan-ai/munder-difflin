use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::config_onboarding::PublicConfig;
use md_web_contracts::domains::memory_skills::{
    CatalogSkill, EmbeddingModel, KnowledgeIngestResponse, KnowledgeUploadRequest,
    SkillActionResponse, SkillCatalogResponse, SkillProvider, SkillScope,
};

use super::{
    ActivityService, BaseSkillService, DomainError, KnowledgeService, MemoryService,
    ProcessControl, SkillRoot, SkillService,
};

const CATALOG_REPOSITORY: &str = "https://github.com/abubakarsiddik31/claude-skills-collection.git";
const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

pub struct MemorySkillsHost {
    pub memory: MemoryService,
    pub knowledge: KnowledgeService,
    pub skills: SkillService,
    pub activity: ActivityService,
    summarizer_cli: Option<PathBuf>,
    data_root: PathBuf,
    local_skill_roots: Vec<PathBuf>,
    process: ProcessControl,
}

pub struct KnowledgeUploadStaging {
    path: PathBuf,
    file: Option<std::fs::File>,
    bytes: usize,
}

impl MemorySkillsHost {
    pub fn from_environment() -> Result<Self, DomainError> {
        let harness = required_absolute_path("MD_HARNESS_HOME")?;
        let enabled = env_flag("MD_MEMORY_ENABLED", true);
        let model = match env::var("MD_MEMORY_MODEL").ok().as_deref() {
            Some("embeddinggemma") => EmbeddingModel::EmbeddingGemma,
            _ => EmbeddingModel::MiniLm,
        };
        Self::from_parts(
            harness,
            enabled,
            model,
            env_flag("MD_KNOWLEDGE_ENABLED", true),
            ProcessControl::default(),
        )
    }

    pub fn from_public_config(config: &PublicConfig) -> Result<Self, DomainError> {
        Self::from_public_config_with_process(config, ProcessControl::default())
    }

    pub fn from_public_config_with_process(
        config: &PublicConfig,
        process: ProcessControl,
    ) -> Result<Self, DomainError> {
        let harness = config
            .harness_home
            .as_deref()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(DomainError::Unavailable(
                "configured harness home is unavailable",
            ))?;
        Self::from_parts(
            harness,
            config.semantic_memory,
            match config.embedding_model {
                md_web_contracts::domains::config_onboarding::EmbeddingModel::MiniLm => {
                    EmbeddingModel::MiniLm
                }
                md_web_contracts::domains::config_onboarding::EmbeddingModel::EmbeddingGemma => {
                    EmbeddingModel::EmbeddingGemma
                }
            },
            true,
            process,
        )
    }

    fn from_parts(
        harness: PathBuf,
        enabled: bool,
        model: EmbeddingModel,
        knowledge_enabled: bool,
        process: ProcessControl,
    ) -> Result<Self, DomainError> {
        let data_root =
            optional_absolute_path("MD_APP_DATA_ROOT").unwrap_or_else(|| harness.join("web-data"));
        let knowledge_root = optional_absolute_path("MD_KNOWLEDGE_ROOT")
            .unwrap_or_else(|| data_root.join("knowledge"));
        let project = optional_absolute_path("MD_PROJECT_ROOT");
        let user = env::var_os("HOME").map(PathBuf::from);
        let mut roots = Vec::new();
        if let Some(home) = user.as_deref() {
            roots.extend([
                skill_root(home.join(".claude/skills"), SkillProvider::Claude),
                skill_root(
                    home.join(".config/opencode/plugin"),
                    SkillProvider::OpenCode,
                ),
                skill_root(home.join(".codex/skills"), SkillProvider::Codex),
            ]);
        }
        if let Some(project) = project.as_deref() {
            roots.extend([
                project_skill_root(project.join(".claude/skills"), SkillProvider::Claude),
                project_skill_root(project.join(".opencode/plugin"), SkillProvider::OpenCode),
                project_skill_root(project.join(".codex/skills"), SkillProvider::Codex),
            ]);
        }
        let local_skill_roots = roots.iter().map(|root| root.path.clone()).collect();
        let install_root = user
            .as_deref()
            .map(|home| home.join(".claude/skills"))
            .ok_or(DomainError::Unavailable("HOME is not configured"))?;
        let semantic_cli = resolve_executable("MD_MEMPALACE_BIN", "mempalace");
        let summarizer_cli = resolve_executable("MD_SUMMARIZER_BIN", "claude");
        Ok(Self {
            memory: MemoryService::with_process_control(
                harness.clone(),
                semantic_cli,
                enabled,
                model,
                process.clone(),
            ),
            knowledge: KnowledgeService::new(knowledge_root, knowledge_enabled),
            skills: SkillService::new(roots, install_root),
            activity: ActivityService::new(harness.join("hive/log.jsonl")),
            summarizer_cli,
            data_root,
            local_skill_roots,
            process,
        })
    }

    pub fn reflect(
        &self,
        agent_id: &str,
    ) -> Result<md_web_contracts::domains::memory_skills::ReflectResult, DomainError> {
        let cli = self
            .summarizer_cli
            .as_deref()
            .ok_or(DomainError::Unavailable("summarizer CLI is not installed"))?;
        self.memory.reflect_agent(agent_id, cli)
    }

    pub fn ingest_upload(
        &self,
        request: &KnowledgeUploadRequest,
    ) -> Result<KnowledgeIngestResponse, DomainError> {
        if request.bytes.len() > MAX_UPLOAD_BYTES {
            return Err(DomainError::InvalidInput("uploaded file exceeds the limit"));
        }
        let mut staging = self.begin_upload()?;
        staging.write_chunk(&request.bytes)?;
        staging.finish(
            self,
            &request.source_name,
            request.title.as_deref(),
            &request.tags,
            request.caption.as_deref(),
        )
    }

    pub fn begin_upload(&self) -> Result<KnowledgeUploadStaging, DomainError> {
        let staging = self.data_root.join("upload-staging");
        fs::create_dir_all(&staging)?;
        let path = staging.join(format!("{}.upload", unique_id()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(KnowledgeUploadStaging {
            path,
            file: Some(file),
            bytes: 0,
        })
    }

    pub fn base_skills(&self) -> Result<BaseSkillService, DomainError> {
        BaseSkillService::with_process_control_and_authoritative_roots(
            self.data_root.join("base-skills"),
            self.process.clone(),
            self.local_skill_roots.clone(),
        )
    }

    pub fn catalog(&self, force: bool) -> SkillCatalogResponse {
        let cache = self.data_root.join("skills-catalog.md");
        let refresh_error = if force {
            self.refresh_catalog(&cache)
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
        let markdown = fs::read_to_string(&cache).unwrap_or_default();
        let skills = self.skills.parse_catalog(&markdown);
        SkillCatalogResponse {
            skills,
            fetched_at_ms: modified_millis(&cache).unwrap_or(0),
            stale: refresh_error.is_some(),
            error: refresh_error,
        }
    }

    pub fn install_catalog_skill(
        &self,
        entry: &CatalogSkill,
    ) -> Result<SkillActionResponse, DomainError> {
        let source = GitHubSource::parse(&entry.url)
            .ok_or(DomainError::InvalidInput("unsupported skill source"))?;
        let staging = self.data_root.join("skill-staging").join(unique_id());
        fs::create_dir_all(&staging)?;
        let checkout = staging.join("checkout");
        let mut command = Command::new("git");
        command.args([
            "-c",
            "protocol.file.allow=never",
            "clone",
            "--depth",
            "1",
            "--filter=blob:none",
            "--no-recurse-submodules",
        ]);
        if let Some(reference) = source.reference.as_deref() {
            command.args(["--branch", reference]);
        }
        command.arg(&source.repository).arg(&checkout);
        let clone = self.process.run(&mut command)?;
        if !clone.status.success() {
            let _ = fs::remove_dir_all(&staging);
            return Ok(SkillActionResponse {
                ok: false,
                managed_id: None,
                error: Some(String::from("skill source could not be fetched")),
                unsupported: false,
            });
        }
        let source_dir = source
            .path
            .as_deref()
            .map_or_else(|| checkout.clone(), |path| checkout.join(path));
        let result = self.skills.install_from_staging(&source_dir, &entry.name);
        let _ = fs::remove_dir_all(staging);
        result
    }

    fn refresh_catalog(&self, cache: &Path) -> Result<(), DomainError> {
        let staging = self.data_root.join("catalog-staging").join(unique_id());
        fs::create_dir_all(&staging)?;
        let checkout = staging.join("checkout");
        let mut command = Command::new("git");
        command
            .args([
                "-c",
                "protocol.file.allow=never",
                "clone",
                "--depth",
                "1",
                "--filter=blob:none",
                "--no-recurse-submodules",
                CATALOG_REPOSITORY,
            ])
            .arg(&checkout);
        let output = self.process.run(&mut command)?;
        if !output.status.success() {
            let _ = fs::remove_dir_all(staging);
            return Err(DomainError::Unavailable(
                "skills catalog could not be fetched",
            ));
        }
        if let Some(parent) = cache.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp = cache.with_extension("md.refreshing");
        fs::copy(checkout.join("README.md"), &temp)?;
        fs::rename(temp, cache)?;
        let _ = fs::remove_dir_all(staging);
        Ok(())
    }
}

impl KnowledgeUploadStaging {
    pub fn write_chunk(&mut self, chunk: &[u8]) -> Result<(), DomainError> {
        self.bytes = self
            .bytes
            .checked_add(chunk.len())
            .ok_or(DomainError::InvalidInput("upload size overflow"))?;
        if self.bytes > MAX_UPLOAD_BYTES {
            return Err(DomainError::InvalidInput("uploaded file exceeds the limit"));
        }
        self.file
            .as_mut()
            .ok_or(DomainError::Unavailable("upload staging is closed"))?
            .write_all(chunk)?;
        Ok(())
    }

    pub fn finish(
        mut self,
        host: &MemorySkillsHost,
        source_name: &str,
        title: Option<&str>,
        tags: &[String],
        caption: Option<&str>,
    ) -> Result<KnowledgeIngestResponse, DomainError> {
        let file = self
            .file
            .take()
            .ok_or(DomainError::Unavailable("upload staging is closed"))?;
        file.sync_all()?;
        drop(file);
        host.knowledge
            .ingest_uploaded_file(&self.path, source_name, title, tags, caption)
    }
}

impl Drop for KnowledgeUploadStaging {
    fn drop(&mut self) {
        let _ = self.file.take();
        let _ = fs::remove_file(&self.path);
    }
}

fn skill_root(path: PathBuf, provider: SkillProvider) -> SkillRoot {
    SkillRoot {
        path,
        provider,
        scope: SkillScope::User,
    }
}

fn project_skill_root(path: PathBuf, provider: SkillProvider) -> SkillRoot {
    SkillRoot {
        path,
        provider,
        scope: SkillScope::Project,
    }
}

fn required_absolute_path(key: &'static str) -> Result<PathBuf, DomainError> {
    optional_absolute_path(key).ok_or(DomainError::Unavailable(key))
}

fn optional_absolute_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn env_flag(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| !matches!(value.trim(), "0" | "false" | "off"))
        .unwrap_or(default)
}

fn resolve_executable(key: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = optional_absolute_path(key).filter(|path| path.is_file()) {
        return Some(path);
    }
    env::split_paths(&env::var_os("PATH")?).find_map(|dir| {
        let candidate = dir.join(name);
        candidate.is_file().then_some(candidate)
    })
}

fn unique_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos())
        .to_string()
}

fn modified_millis(path: &Path) -> Option<i64> {
    let millis = fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis();
    i64::try_from(millis).ok()
}

struct GitHubSource {
    repository: String,
    reference: Option<String>,
    path: Option<String>,
}

impl GitHubSource {
    fn parse(url: &str) -> Option<Self> {
        let clean = url.trim().split(['?', '#']).next()?.trim_end_matches('/');
        let path = clean.strip_prefix("https://github.com/")?;
        let mut parts = path.split('/');
        let owner = safe_remote_part(parts.next()?)?;
        let repo = safe_remote_part(parts.next()?.trim_end_matches(".git"))?;
        let remaining: Vec<&str> = parts.collect();
        let (reference, skill_path) = if remaining.is_empty() {
            (None, None)
        } else if remaining.len() >= 3 && remaining[0] == "tree" {
            let reference = safe_remote_part(remaining[1])?;
            let path = remaining[2..]
                .iter()
                .map(|part| safe_remote_part(part))
                .collect::<Option<Vec<_>>>()?
                .join("/");
            (Some(reference.to_owned()), Some(path))
        } else {
            return None;
        };
        Some(Self {
            repository: format!("https://github.com/{owner}/{repo}.git"),
            reference,
            path: skill_path,
        })
    }
}

fn safe_remote_part(value: &str) -> Option<&str> {
    (!value.is_empty()
        && value.len() <= 128
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then_some(value)
}

#[cfg(test)]
mod tests {
    use super::GitHubSource;

    #[test]
    fn github_tree_source_is_bounded_to_repository_path() {
        let source =
            GitHubSource::parse("https://github.com/acme/skills/tree/main/packages/review")
                .unwrap_or_else(|| panic!("valid source"));
        assert_eq!(source.reference.as_deref(), Some("main"));
        assert_eq!(source.path.as_deref(), Some("packages/review"));
    }

    #[test]
    fn non_https_and_parent_paths_are_rejected() {
        assert!(GitHubSource::parse("file:///tmp/skill").is_none());
        assert!(GitHubSource::parse("https://github.com/acme/repo/tree/main/../x").is_none());
    }
}
