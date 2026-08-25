use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use md_web_contracts::domains::memory_skills::{
    BaseSkillCatalogEntry, BaseSkillCatalogSnapshot, BaseSkillSelectionRequest,
    BaseSkillSourceKind, BaseSkillSourceView, RoleSkillAssignment, SkillCompatibility,
    SoftwareTeamRole, TeamSkillAssignments,
};
use serde::{Deserialize, Serialize};

use super::{DomainError, ProcessControl};

const MAX_SKILL_INSTRUCTIONS: u64 = 512 * 1024;
const MAX_OPENAI_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_CATALOG_SKILLS: usize = 2_000;
const OPENAI_SKILLS_API: &str = "https://api.openai.com/v1";
const AUTHORITATIVE_LOCAL: [&str; 2] = ["local-development", "web-project-standards"];

#[derive(Clone, Debug, Deserialize)]
struct SourceConfig {
    id: String,
    name: String,
    kind: BaseSkillSourceKind,
    repository: String,
    #[serde(default = "default_reference")]
    reference: String,
    #[serde(default)]
    official: bool,
    token_env: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedCatalog {
    sources: Vec<BaseSkillSourceView>,
    skills: Vec<BaseSkillCatalogEntry>,
    cached_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssignedSkillInstruction {
    pub skill_id: String,
    pub path: PathBuf,
    pub instructions: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSkillInjection {
    pub agent_id: String,
    pub skills: Vec<AssignedSkillInstruction>,
}

pub struct BaseSkillService {
    root: PathBuf,
    sources: Vec<SourceConfig>,
    authoritative_roots: Vec<PathBuf>,
    process: ProcessControl,
}

impl BaseSkillService {
    pub fn from_environment(root: PathBuf) -> Result<Self, DomainError> {
        Self::with_process_control(root, ProcessControl::default())
    }

    pub fn with_process_control(
        root: PathBuf,
        process: ProcessControl,
    ) -> Result<Self, DomainError> {
        let mut sources = configured_sources()?;
        if env_flag("MD_ENABLE_OFFICIAL_SKILL_SOURCES", false) {
            sources.extend(official_sources());
        }
        deduplicate_sources(&mut sources);
        let authoritative_roots = env::var_os("MD_AUTHORITATIVE_SKILL_ROOTS")
            .map(|value| {
                env::split_paths(&value)
                    .filter(|path| path.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        Ok(Self {
            root,
            sources,
            authoritative_roots,
            process,
        })
    }

    pub fn cached_catalog(&self) -> BaseSkillCatalogSnapshot {
        let path = self.root.join("catalog.json");
        match fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<CachedCatalog>(&text).ok())
        {
            Some(mut cached) => {
                for skill in &mut cached.skills {
                    skill.installed = self
                        .root
                        .join("installed")
                        .join(safe_id(&skill.name))
                        .join("SKILL.md")
                        .is_file();
                }
                BaseSkillCatalogSnapshot {
                    sources: cached.sources,
                    skills: cached.skills,
                    cached_at_ms: cached.cached_at_ms,
                    stale: false,
                    error: None,
                }
            }
            None => BaseSkillCatalogSnapshot {
                sources: self.source_views(),
                skills: Vec::new(),
                cached_at_ms: 0,
                stale: true,
                error: None,
            },
        }
    }

    pub fn openai_project_skills_enabled(&self) -> bool {
        self.sources
            .iter()
            .any(|source| source.kind == BaseSkillSourceKind::OpenAiProjectSkillsApi)
    }

    pub fn refresh_catalog(&self) -> Result<BaseSkillCatalogSnapshot, DomainError> {
        fs::create_dir_all(self.root.join("sources"))?;
        let mut skills = Vec::new();
        for source in &self.sources {
            if source.kind == BaseSkillSourceKind::OpenAiProjectSkillsApi {
                continue;
            }
            let checkout = self.refresh_source(source)?;
            let version = git_output(&self.process, &checkout, ["rev-parse", "HEAD"], None)?;
            skills.extend(scan_skills(source, &checkout, &version)?);
            if skills.len() > MAX_CATALOG_SKILLS {
                return Err(DomainError::InvalidInput("base skill catalog is too large"));
            }
        }
        let cached_at_ms = epoch_millis();
        let cached = CachedCatalog {
            sources: self.source_views(),
            skills,
            cached_at_ms,
        };
        atomic_json(&self.root.join("catalog.json"), &cached)?;
        Ok(BaseSkillCatalogSnapshot {
            sources: cached.sources,
            skills: cached.skills,
            cached_at_ms,
            stale: false,
            error: None,
        })
    }

    /// Explicit-refresh only. The caller owns server-side secret hydration and
    /// must never pass the API key across a browser/server-function boundary.
    pub async fn refresh_openai_project_skills(
        &self,
        api_key: &str,
    ) -> Result<BaseSkillCatalogSnapshot, DomainError> {
        let source = self
            .sources
            .iter()
            .find(|source| source.kind == BaseSkillSourceKind::OpenAiProjectSkillsApi)
            .ok_or(DomainError::Unavailable(
                "OpenAI project skills source is disabled",
            ))?;
        let client = OpenAiSkillsClient::new(OPENAI_SKILLS_API)?;
        let skills = client
            .fetch_and_cache(source, api_key, &self.root, &self.process)
            .await?;
        let mut cached = read_cached_catalog(&self.root).unwrap_or(CachedCatalog {
            sources: self.source_views_with_openai(true),
            skills: Vec::new(),
            cached_at_ms: 0,
        });
        cached.skills.retain(|entry| entry.source_id != source.id);
        cached.skills.extend(skills);
        if cached.skills.len() > MAX_CATALOG_SKILLS {
            return Err(DomainError::InvalidInput("base skill catalog is too large"));
        }
        cached.sources = self.source_views_with_openai(true);
        cached.cached_at_ms = epoch_millis();
        atomic_json(&self.root.join("catalog.json"), &cached)?;
        Ok(BaseSkillCatalogSnapshot {
            sources: cached.sources,
            skills: cached.skills,
            cached_at_ms: cached.cached_at_ms,
            stale: false,
            error: None,
        })
    }

    pub fn install_selection(
        &self,
        request: &BaseSkillSelectionRequest,
    ) -> Result<Vec<BaseSkillCatalogEntry>, DomainError> {
        if !request.confirmed {
            return Err(DomainError::InvalidInput(
                "explicit onboarding confirmation is required",
            ));
        }
        let catalog = self.cached_catalog();
        let by_id: BTreeMap<&str, &BaseSkillCatalogEntry> = catalog
            .skills
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect();
        let mut installed = Vec::new();
        for requested in request.skill_ids.iter().take(64) {
            let entry = by_id
                .get(requested.as_str())
                .ok_or(DomainError::InvalidInput("unknown base skill selection"))?;
            if AUTHORITATIVE_LOCAL.contains(&entry.name.as_str()) {
                return Err(DomainError::InvalidInput(
                    "software standards must use the authoritative local skill",
                ));
            }
            let source = self
                .sources
                .iter()
                .find(|source| source.id == entry.source_id)
                .ok_or(DomainError::InvalidInput("unknown base skill source"))?;
            let source_root = self.root.join("sources").join(&source.id).join("checkout");
            let skill_root = contained(&source_root, &entry.relative_path)?;
            let destination = self.root.join("installed").join(safe_id(&entry.name));
            copy_skill_atomic(&skill_root, &destination)?;
            let mut record = (*entry).clone();
            record.installed = true;
            atomic_json(&destination.join("provenance.json"), &record)?;
            installed.push(record);
        }
        Ok(installed)
    }

    pub fn load_assignments(&self) -> TeamSkillAssignments {
        fs::read_to_string(self.root.join("assignments.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(TeamSkillAssignments::minimum_software_team)
    }

    pub fn save_assignments(
        &self,
        mut assignments: TeamSkillAssignments,
    ) -> Result<TeamSkillAssignments, DomainError> {
        validate_assignments(&assignments)?;
        assignments.updated_at_ms = epoch_millis();
        atomic_json(&self.root.join("assignments.json"), &assignments)?;
        Ok(assignments)
    }

    pub fn injection_for(
        &self,
        agent_id: &str,
        task_domains: &[String],
    ) -> Result<AgentSkillInjection, DomainError> {
        let assignments = self.load_assignments();
        let selected: Vec<&RoleSkillAssignment> = assignments
            .assignments
            .iter()
            .filter(|assignment| assignment.agent_id == agent_id)
            .filter(|assignment| {
                assignment.role != SoftwareTeamRole::DomainSpecialist
                    || assignment
                        .task_condition
                        .as_deref()
                        .is_some_and(|condition| {
                            task_domains
                                .iter()
                                .any(|domain| domain.eq_ignore_ascii_case(condition))
                        })
            })
            .collect();
        let mut seen = BTreeSet::new();
        let mut skills = Vec::new();
        for skill_id in selected
            .into_iter()
            .flat_map(|assignment| &assignment.skill_ids)
        {
            if !seen.insert(skill_id.clone()) {
                continue;
            }
            let path = self
                .resolve_installed(skill_id)
                .ok_or(DomainError::Unavailable(
                    "assigned base skill is not installed",
                ))?;
            let instructions_path = path.join("SKILL.md");
            if fs::metadata(&instructions_path)?.len() > MAX_SKILL_INSTRUCTIONS {
                return Err(DomainError::InvalidInput(
                    "skill instructions are too large",
                ));
            }
            skills.push(AssignedSkillInstruction {
                skill_id: skill_id.clone(),
                path,
                instructions: fs::read_to_string(instructions_path)?,
            });
        }
        Ok(AgentSkillInjection {
            agent_id: agent_id.to_owned(),
            skills,
        })
    }

    fn resolve_installed(&self, skill_id: &str) -> Option<PathBuf> {
        let managed = self.root.join("installed").join(skill_id);
        if managed.join("SKILL.md").is_file() {
            return Some(managed);
        }
        self.authoritative_roots
            .iter()
            .map(|root| root.join(skill_id))
            .find(|path| path.join("SKILL.md").is_file())
    }

    fn source_views(&self) -> Vec<BaseSkillSourceView> {
        self.source_views_with_openai(false)
    }

    fn source_views_with_openai(&self, openai_authenticated: bool) -> Vec<BaseSkillSourceView> {
        self.sources
            .iter()
            .map(|source| BaseSkillSourceView {
                id: source.id.clone(),
                name: source.name.clone(),
                kind: source.kind,
                repository: source.repository.clone(),
                reference: source.reference.clone(),
                official: source.official,
                authentication_configured: if source.kind
                    == BaseSkillSourceKind::OpenAiProjectSkillsApi
                {
                    openai_authenticated
                } else {
                    source
                        .token_env
                        .as_deref()
                        .is_some_and(|key| env::var_os(key).is_some())
                },
            })
            .collect()
    }

    fn refresh_source(&self, source: &SourceConfig) -> Result<PathBuf, DomainError> {
        validate_source(source)?;
        let source_dir = self.root.join("sources").join(&source.id);
        let staging = source_dir.with_extension("refreshing");
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        if source_dir.exists() {
            fs::remove_dir_all(&source_dir)?;
        }
        fs::create_dir_all(&staging)?;
        let checkout = staging.join("checkout");
        let token = source
            .token_env
            .as_deref()
            .and_then(|key| env::var(key).ok());
        git_clone(&self.process, source, &checkout, token.as_deref())?;
        fs::rename(staging, &source_dir)?;
        Ok(source_dir.join("checkout"))
    }
}

fn official_sources() -> Vec<SourceConfig> {
    let mut sources = vec![
        SourceConfig {
            id: String::from("openai-project-skills"),
            name: String::from("OpenAI Project Skills API"),
            kind: BaseSkillSourceKind::OpenAiProjectSkillsApi,
            repository: String::from("https://api.openai.com/v1/skills"),
            reference: String::from("latest"),
            official: true,
            token_env: None,
        },
        SourceConfig {
            id: String::from("openai-plugins"),
            name: String::from("OpenAI Plugins GitHub Marketplace"),
            kind: BaseSkillSourceKind::OpenAiPluginMarketplace,
            repository: String::from("https://github.com/openai/plugins.git"),
            reference: String::from("main"),
            official: true,
            token_env: None,
        },
        SourceConfig {
            id: String::from("anthropic-agent-skills"),
            name: String::from("Anthropic Agent Skills"),
            kind: BaseSkillSourceKind::AnthropicAgentSkills,
            repository: String::from("https://github.com/anthropics/skills.git"),
            reference: String::from("main"),
            official: true,
            token_env: None,
        },
    ];
    if env_flag("MD_ENABLE_ANTHROPIC_PLUGIN_MARKETPLACE", false) {
        sources.push(SourceConfig {
            id: String::from("anthropic-claude-plugins"),
            name: String::from("Anthropic Claude Plugins Official"),
            kind: BaseSkillSourceKind::AnthropicPluginMarketplace,
            repository: String::from("https://github.com/anthropics/claude-plugins-official.git"),
            reference: String::from("main"),
            official: true,
            token_env: None,
        });
    }
    sources
}

fn configured_sources() -> Result<Vec<SourceConfig>, DomainError> {
    let Some(value) = env::var("MD_BASE_SKILL_SOURCES_JSON").ok() else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&value).map_err(DomainError::Serialization)
}

struct OpenAiSkillsClient {
    client: reqwest::Client,
    base_url: String,
}

struct OpenAiSkillMetadata {
    id: String,
    name: String,
    description: String,
    version: String,
    license: Option<String>,
}

impl OpenAiSkillsClient {
    fn new(base_url: &str) -> Result<Self, DomainError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| DomainError::Unavailable("OpenAI skills client is unavailable"))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    async fn fetch_and_cache(
        &self,
        source: &SourceConfig,
        api_key: &str,
        root: &Path,
        process: &ProcessControl,
    ) -> Result<Vec<BaseSkillCatalogEntry>, DomainError> {
        if api_key.trim().is_empty() {
            return Err(DomainError::Unavailable(
                "OpenAI project API key is unavailable",
            ));
        }
        let metadata = self.list(api_key).await?;
        let source_dir = root.join("sources").join(&source.id);
        let staging = source_dir.with_extension("refreshing");
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        fs::create_dir_all(staging.join("checkout"))?;
        let result = self
            .materialize(source, api_key, &staging, process, metadata)
            .await;
        let entries = match result {
            Ok(entries) => entries,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };
        if source_dir.exists() {
            fs::remove_dir_all(&source_dir)?;
        }
        fs::rename(staging, source_dir)?;
        Ok(entries)
    }

    async fn materialize(
        &self,
        source: &SourceConfig,
        api_key: &str,
        staging: &Path,
        process: &ProcessControl,
        metadata: Vec<OpenAiSkillMetadata>,
    ) -> Result<Vec<BaseSkillCatalogEntry>, DomainError> {
        let mut entries = Vec::new();
        for skill in metadata.into_iter().take(MAX_CATALOG_SKILLS) {
            let relative_path = safe_id(&skill.id);
            if relative_path.is_empty() {
                continue;
            }
            let (content, resolved_version) =
                self.content(api_key, &skill.id, &skill.version).await?;
            let skill_root = staging.join("checkout").join(&relative_path);
            fs::create_dir_all(&skill_root)?;
            materialize_openai_skill(process, &skill_root, &content)?;
            let text = fs::read_to_string(skill_root.join("SKILL.md"))?;
            let (manifest_name, manifest_description) =
                parse_frontmatter(&text, Path::new(&skill.name));
            entries.push(BaseSkillCatalogEntry {
                id: format!("{}--{}", source.id, safe_id(&manifest_name)),
                source_id: source.id.clone(),
                name: manifest_name,
                description: if manifest_description.is_empty() {
                    skill.description
                } else {
                    manifest_description
                },
                relative_path,
                provenance: format!("{}/skills/{}", self.base_url, skill.id),
                version: resolved_version,
                license: skill.license,
                compatibility: vec![
                    SkillCompatibility::CodexPlugin,
                    SkillCompatibility::SharedAgentSkill,
                ],
                installed: false,
            });
        }
        Ok(entries)
    }

    async fn list(&self, api_key: &str) -> Result<Vec<OpenAiSkillMetadata>, DomainError> {
        let mut after = None;
        let mut skills = Vec::new();
        for _ in 0..20 {
            let mut url = format!("{}/skills?limit=100", self.base_url);
            if let Some(cursor) = after.as_deref() {
                url.push_str("&after=");
                url.push_str(cursor);
            }
            let bytes = self.get_bounded(&url, api_key).await?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let page = parse_openai_skill_page(&value)?;
            let has_more = value
                .get("has_more")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            after = page.last().map(|skill| skill.id.clone());
            skills.extend(page);
            if !has_more || after.is_none() || skills.len() >= MAX_CATALOG_SKILLS {
                break;
            }
        }
        Ok(skills)
    }

    async fn content(
        &self,
        api_key: &str,
        skill_id: &str,
        version: &str,
    ) -> Result<(Vec<u8>, String), DomainError> {
        if !safe_api_segment(skill_id) || (!version.is_empty() && !safe_api_segment(version)) {
            return Err(DomainError::InvalidInput("invalid OpenAI skill identifier"));
        }
        let resolved_version = if version.is_empty() || version == "latest" {
            self.latest_version(api_key, skill_id).await?
        } else {
            Some(version.to_owned())
        };
        if let Some(version) = resolved_version.as_deref() {
            let url = format!(
                "{}/skills/{skill_id}/versions/{version}/content",
                self.base_url
            );
            if let Some(content) = self.get_optional_bounded(&url, api_key).await? {
                return Ok((content, version.to_owned()));
            }
        }
        Ok((
            self.get_bounded(
                &format!("{}/skills/{skill_id}/content", self.base_url),
                api_key,
            )
            .await?,
            String::from("latest"),
        ))
    }

    async fn latest_version(
        &self,
        api_key: &str,
        skill_id: &str,
    ) -> Result<Option<String>, DomainError> {
        let bytes = self
            .get_optional_bounded(
                &format!("{}/skills/{skill_id}/versions?limit=1", self.base_url),
                api_key,
            )
            .await?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        Ok(value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .and_then(|versions| versions.first())
            .map(|version| scalar_version(Some(version)))
            .filter(|version| version != "latest" && safe_api_segment(version)))
    }

    async fn get_bounded(&self, url: &str, api_key: &str) -> Result<Vec<u8>, DomainError> {
        self.get_optional_bounded(url, api_key)
            .await?
            .ok_or(DomainError::Unavailable(
                "OpenAI skill content was not found",
            ))
    }

    async fn get_optional_bounded(
        &self,
        url: &str,
        api_key: &str,
    ) -> Result<Option<Vec<u8>>, DomainError> {
        let mut response = self
            .client
            .get(url)
            .bearer_auth(api_key)
            .send()
            .await
            .map_err(|_| DomainError::Unavailable("OpenAI skills request failed"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(DomainError::Unavailable("OpenAI skills request failed"));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_OPENAI_RESPONSE_BYTES as u64)
        {
            return Err(DomainError::InvalidInput(
                "OpenAI skill response is too large",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DomainError::Unavailable("OpenAI skill response failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_OPENAI_RESPONSE_BYTES {
                return Err(DomainError::InvalidInput(
                    "OpenAI skill response is too large",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }
}

fn parse_openai_skill_page(
    value: &serde_json::Value,
) -> Result<Vec<OpenAiSkillMetadata>, DomainError> {
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or(DomainError::InvalidInput(
            "OpenAI skills response is invalid",
        ))?;
    let mut skills = Vec::new();
    for item in data {
        let Some(id) = item.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !safe_api_segment(id) {
            continue;
        }
        let name = item
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(id)
            .to_owned();
        let description = item
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let version = scalar_version(item.get("latest_version").or_else(|| item.get("version")));
        let license = item
            .get("license")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        skills.push(OpenAiSkillMetadata {
            id: id.to_owned(),
            name,
            description,
            version,
            license,
        });
    }
    Ok(skills)
}

fn scalar_version(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(value)) => value.clone(),
        Some(serde_json::Value::Number(value)) => value.to_string(),
        Some(serde_json::Value::Object(value)) => value
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("latest")
            .to_owned(),
        _ => String::from("latest"),
    }
}

fn safe_api_segment(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn materialize_openai_skill(
    process: &ProcessControl,
    destination: &Path,
    content: &[u8],
) -> Result<(), DomainError> {
    if let Ok(text) = std::str::from_utf8(content)
        && text.starts_with("---")
    {
        fs::write(destination.join("SKILL.md"), content)?;
        return Ok(());
    }
    if !content.starts_with(b"PK") {
        return Err(DomainError::InvalidInput(
            "OpenAI skill content is not Agent Skills compatible",
        ));
    }
    let archive = destination.join("content.zip");
    fs::write(&archive, content)?;
    let mut list = Command::new("unzip");
    list.args(["-Z1"]).arg(&archive);
    let listing = process.run(&mut list)?;
    if !listing.status.success() {
        return Err(DomainError::InvalidInput("OpenAI skill archive is invalid"));
    }
    let listing_text = String::from_utf8_lossy(&listing.stdout);
    let entry = listing_text
        .lines()
        .filter(|path| {
            !path.starts_with('/')
                && !path.split('/').any(|part| matches!(part, "" | "." | ".."))
                && (*path == "SKILL.md" || path.ends_with("/SKILL.md"))
        })
        .min_by_key(|path| path.matches('/').count())
        .ok_or(DomainError::InvalidInput(
            "OpenAI skill archive has no SKILL.md",
        ))?
        .to_owned();
    let mut extract = Command::new("unzip");
    extract.args(["-p"]).arg(&archive).arg(&entry);
    let output = process.run(&mut extract)?;
    if !output.status.success()
        || output.stdout.len() > MAX_SKILL_INSTRUCTIONS as usize
        || !output.stdout.starts_with(b"---")
    {
        return Err(DomainError::InvalidInput(
            "OpenAI skill instructions are incompatible",
        ));
    }
    fs::write(destination.join("SKILL.md"), output.stdout)?;
    fs::remove_file(archive)?;
    Ok(())
}

fn scan_skills(
    source: &SourceConfig,
    checkout: &Path,
    version: &str,
) -> Result<Vec<BaseSkillCatalogEntry>, DomainError> {
    let declared_paths = marketplace_skill_paths(source, checkout);
    let mut manifests = Vec::new();
    collect_skill_manifests(checkout, checkout, 0, &mut manifests)?;
    manifests
        .into_iter()
        .map(|manifest| {
            let relative = manifest
                .parent()
                .and_then(|path| path.strip_prefix(checkout).ok())
                .ok_or(DomainError::OutsideManagedRoot)?;
            let text = fs::read_to_string(&manifest)?;
            let (name, description) = parse_frontmatter(&text, relative);
            let relative_path = relative.to_string_lossy().replace('\\', "/");
            let id = format!("{}--{}", source.id, safe_id(&name));
            let license = find_license(manifest.parent().unwrap_or(checkout), checkout);
            let declared_by_marketplace =
                declared_paths.iter().any(|path| relative.starts_with(path));
            let compatibility = match source.kind {
                BaseSkillSourceKind::OpenAiProjectSkillsApi => vec![
                    SkillCompatibility::CodexPlugin,
                    SkillCompatibility::SharedAgentSkill,
                ],
                BaseSkillSourceKind::OpenAiPluginMarketplace => vec![
                    SkillCompatibility::CodexPlugin,
                    SkillCompatibility::SharedAgentSkill,
                ],
                BaseSkillSourceKind::AnthropicPluginMarketplace if declared_by_marketplace => vec![
                    SkillCompatibility::ClaudePlugin,
                    SkillCompatibility::SharedAgentSkill,
                ],
                BaseSkillSourceKind::AnthropicPluginMarketplace => {
                    vec![SkillCompatibility::SharedAgentSkill]
                }
                BaseSkillSourceKind::AnthropicAgentSkills
                | BaseSkillSourceKind::GitHubRepository => {
                    vec![SkillCompatibility::SharedAgentSkill]
                }
            };
            Ok(BaseSkillCatalogEntry {
                id,
                source_id: source.id.clone(),
                name,
                description,
                relative_path,
                provenance: source.repository.clone(),
                version: version.trim().to_owned(),
                license,
                compatibility,
                installed: false,
            })
        })
        .collect()
}

fn marketplace_skill_paths(source: &SourceConfig, checkout: &Path) -> Vec<PathBuf> {
    let manifest = match source.kind {
        BaseSkillSourceKind::OpenAiProjectSkillsApi => return Vec::new(),
        BaseSkillSourceKind::OpenAiPluginMarketplace => {
            checkout.join(".agents/plugins/marketplace.json")
        }
        BaseSkillSourceKind::AnthropicAgentSkills
        | BaseSkillSourceKind::AnthropicPluginMarketplace => {
            checkout.join(".claude-plugin/marketplace.json")
        }
        BaseSkillSourceKind::GitHubRepository => return Vec::new(),
    };
    let Some(value) = fs::read_to_string(manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
    else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    collect_declared_skill_paths(&value, &mut paths);
    paths
        .into_iter()
        .filter_map(|path| {
            let cleaned = path.trim_start_matches("./");
            (!cleaned.is_empty()
                && !cleaned
                    .split('/')
                    .any(|part| matches!(part, "" | "." | "..")))
            .then(|| PathBuf::from(cleaned))
        })
        .collect()
}

fn collect_declared_skill_paths(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                if key == "skills" {
                    match value {
                        serde_json::Value::String(path) => paths.push(path.clone()),
                        serde_json::Value::Array(items) => paths.extend(
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_owned),
                        ),
                        _ => {}
                    }
                }
                collect_declared_skill_paths(value, paths);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_declared_skill_paths(item, paths);
            }
        }
        _ => {}
    }
}

fn collect_skill_manifests(
    root: &Path,
    current: &Path,
    depth: usize,
    manifests: &mut Vec<PathBuf>,
) -> Result<(), DomainError> {
    if depth > 8 || manifests.len() >= MAX_CATALOG_SKILLS {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let path = entry.path();
        if metadata.is_dir() && path.starts_with(root) && entry.file_name() != ".git" {
            collect_skill_manifests(root, &path, depth + 1, manifests)?;
        } else if metadata.is_file() && entry.file_name() == "SKILL.md" {
            manifests.push(path);
        }
    }
    Ok(())
}

fn parse_frontmatter(text: &str, fallback: &Path) -> (String, String) {
    let body = text
        .strip_prefix("---\n")
        .and_then(|text| text.split_once("\n---"));
    let mut name = None;
    let mut description = None;
    if let Some((frontmatter, _)) = body {
        for line in frontmatter.lines() {
            if let Some(value) = line.strip_prefix("name:") {
                name = Some(clean_scalar(value));
            } else if let Some(value) = line.strip_prefix("description:") {
                description = Some(clean_scalar(value));
            }
        }
    }
    (
        name.unwrap_or_else(|| {
            fallback.file_name().map_or_else(
                || String::from("skill"),
                |name| name.to_string_lossy().into(),
            )
        }),
        description.unwrap_or_default(),
    )
}

fn clean_scalar(value: &str) -> String {
    value.trim().trim_matches(['"', '\'']).to_owned()
}

fn find_license(skill: &Path, root: &Path) -> Option<String> {
    [skill, root].into_iter().find_map(|directory| {
        ["LICENSE", "LICENSE.md", "LICENSE.txt"]
            .into_iter()
            .map(|name| directory.join(name))
            .find(|path| path.is_file())
            .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into()))
    })
}

fn git_clone(
    process: &ProcessControl,
    source: &SourceConfig,
    destination: &Path,
    token: Option<&str>,
) -> Result<(), DomainError> {
    let mut command = Command::new("git");
    command.args([
        "-c",
        "protocol.file.allow=never",
        "clone",
        "--depth",
        "1",
        "--filter=blob:none",
        "--no-recurse-submodules",
        "--branch",
        &source.reference,
        &source.repository,
    ]);
    command.arg(destination);
    if let Some(token) = token {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Bearer {token}"),
            );
    }
    let output = process.run(&mut command)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(DomainError::Unavailable("base skill source fetch failed"))
    }
}

fn git_output<const N: usize>(
    process: &ProcessControl,
    cwd: &Path,
    args: [&str; N],
    token: Option<&str>,
) -> Result<String, DomainError> {
    let mut command = Command::new("git");
    command.current_dir(cwd).args(args);
    if let Some(token) = token {
        command
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env(
                "GIT_CONFIG_VALUE_0",
                format!("Authorization: Bearer {token}"),
            );
    }
    let output = process.run(&mut command)?;
    if !output.status.success() {
        return Err(DomainError::Unavailable("git metadata is unavailable"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn validate_source(source: &SourceConfig) -> Result<(), DomainError> {
    if source.kind == BaseSkillSourceKind::OpenAiProjectSkillsApi {
        return if safe_id(&source.id) == source.id
            && source.repository == "https://api.openai.com/v1/skills"
        {
            Ok(())
        } else {
            Err(DomainError::InvalidInput("invalid OpenAI skills source"))
        };
    }
    if safe_id(&source.id) != source.id
        || !source.repository.starts_with("https://github.com/")
        || source.reference.is_empty()
        || source.reference.contains("..")
    {
        return Err(DomainError::InvalidInput("invalid base skill source"));
    }
    Ok(())
}

fn read_cached_catalog(root: &Path) -> Option<CachedCatalog> {
    fs::read_to_string(root.join("catalog.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

fn validate_assignments(assignments: &TeamSkillAssignments) -> Result<(), DomainError> {
    if assignments.assignments.len() > 64
        || assignments.assignments.iter().any(|assignment| {
            safe_id(&assignment.agent_id) != assignment.agent_id
                || assignment.skill_ids.len() > 32
                || assignment
                    .skill_ids
                    .iter()
                    .any(|skill| safe_id(skill) != *skill)
                || (assignment.role == SoftwareTeamRole::DomainSpecialist
                    && assignment
                        .task_condition
                        .as_deref()
                        .is_none_or(str::is_empty))
        })
    {
        return Err(DomainError::InvalidInput("invalid role skill assignments"));
    }
    Ok(())
}

fn contained(root: &Path, relative: &str) -> Result<PathBuf, DomainError> {
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    if path == root || !path.starts_with(&root) || !path.join("SKILL.md").is_file() {
        return Err(DomainError::OutsideManagedRoot);
    }
    Ok(path)
}

fn copy_skill_atomic(source: &Path, destination: &Path) -> Result<(), DomainError> {
    if destination.exists() {
        return Err(DomainError::InvalidInput("base skill is already installed"));
    }
    let parent = destination
        .parent()
        .ok_or(DomainError::OutsideManagedRoot)?;
    fs::create_dir_all(parent)?;
    let temp = destination.with_extension("installing");
    fs::create_dir(&temp)?;
    copy_tree(source, &temp, source, 0)?;
    fs::rename(temp, destination)?;
    Ok(())
}

fn copy_tree(root: &Path, target: &Path, current: &Path, depth: usize) -> Result<(), DomainError> {
    if depth > 8 {
        return Err(DomainError::InvalidInput(
            "base skill nesting exceeds limit",
        ));
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| DomainError::OutsideManagedRoot)?
            .to_owned();
        let output = target.join(relative);
        if metadata.is_dir() {
            fs::create_dir_all(&output)?;
            copy_tree(root, target, &entry.path(), depth + 1)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), output)?;
        }
    }
    Ok(())
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), DomainError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("json.writing");
    fs::write(&temp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temp, path)?;
    Ok(())
}

fn safe_id(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_owned()
}

fn deduplicate_sources(sources: &mut Vec<SourceConfig>) {
    let mut seen = BTreeSet::new();
    sources.retain(|source| seen.insert(source.id.clone()));
}

fn default_reference() -> String {
    String::from("main")
}

fn env_flag(key: &str, default: bool) -> bool {
    env::var(key)
        .ok()
        .map(|value| !matches!(value.trim(), "0" | "false" | "off"))
        .unwrap_or(default)
}

fn epoch_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path;

    use super::{
        OpenAiSkillsClient, collect_declared_skill_paths, official_sources, parse_frontmatter,
        safe_id, validate_assignments,
    };
    use md_web_contracts::domains::memory_skills::TeamSkillAssignments;

    #[test]
    fn standard_skill_frontmatter_is_catalogued_without_running_scripts() {
        let (name, description) = parse_frontmatter(
            "---\nname: webapp-testing\ndescription: Browser checks\n---\nbody",
            Path::new("fallback"),
        );
        assert_eq!(name, "webapp-testing");
        assert_eq!(description, "Browser checks");
    }

    #[test]
    fn minimum_team_assignments_are_valid() {
        assert!(validate_assignments(&TeamSkillAssignments::minimum_software_team()).is_ok());
    }

    #[test]
    fn ids_are_normalized_for_durable_paths() {
        assert_eq!(safe_id("Frontend Design"), "frontend-design");
    }

    #[test]
    fn anthropic_marketplace_skill_groups_are_parsed() {
        let value = serde_json::json!({
            "plugins": [{ "skills": ["./skills/webapp-testing", "./skills/frontend-design"] }]
        });
        let mut paths = Vec::new();
        collect_declared_skill_paths(&value, &mut paths);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn official_sources_distinguish_project_api_from_github_marketplace() {
        let sources = official_sources();
        assert!(sources.iter().any(|source| {
            source.id == "openai-project-skills"
                && source.repository == "https://api.openai.com/v1/skills"
        }));
        assert!(sources.iter().any(|source| {
            source.id == "openai-plugins"
                && source.repository == "https://github.com/openai/plugins.git"
        }));
    }

    #[tokio::test]
    async fn openai_project_api_list_and_version_content_use_mock_http()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let server = std::thread::spawn(move || -> std::io::Result<()> {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept()?;
                let mut request = [0_u8; 4096];
                let read = stream.read(&mut request)?;
                let request = String::from_utf8_lossy(&request[..read]);
                assert!(
                    request.contains("authorization: Bearer test-secret")
                        || request.contains("Authorization: Bearer test-secret")
                );
                let body = if request.starts_with("GET /skills?limit=100 ") {
                    r#"{"data":[{"id":"sk_test","name":"Test skill","description":"Mocked","license":"MIT"}],"has_more":false}"#
                } else if request.starts_with("GET /skills/sk_test/versions?limit=1 ") {
                    r#"{"data":[{"version":"2"}],"has_more":false}"#
                } else if request.starts_with("GET /skills/sk_test/versions/2/content ") {
                    "---\nname: test-skill\ndescription: Mock content\n---\nInstructions"
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "unexpected mock request path",
                    ));
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )?;
            }
            Ok(())
        });
        let client = OpenAiSkillsClient::new(&format!("http://{address}"))?;

        let page = client.list("test-secret").await?;
        assert_eq!(page.len(), 1);
        let skill = page.first().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "mock skill is missing")
        })?;
        assert_eq!(skill.version, "latest");
        let (content, version) = client
            .content("test-secret", &skill.id, &skill.version)
            .await?;
        assert_eq!(version, "2");
        assert!(content.starts_with(b"---\nname: test-skill"));
        match server.join() {
            Ok(result) => result?,
            Err(_) => {
                return Err(std::io::Error::other("mock API thread panicked").into());
            }
        }
        Ok(())
    }
}
