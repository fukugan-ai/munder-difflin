use std::fmt::{Display, Formatter};
use std::future::Future;
use std::time::Duration;

use md_web_contracts::domains::config_onboarding::{
    AriaSpawnRecipe, ChangeHomeRequest, ConfigPatch, ConfigRuntimeReceipt,
    ConfirmTeamInitializedRequest, ConfirmTeamInitializedResult, FinishOnboardingRequest,
    FinishOnboardingResult, MAX_AGENT_TOKEN_CAP, OnboardingNavigation, OnboardingPhase,
    OnboardingRepairReceipt, PublicConfig, RuntimeReinitialize, SetAgentTokenCapRequest, TeamRole,
};
use md_web_contracts::domains::memory_skills::LocalSkill;
use md_web_contracts::domains::persistence::{AppConfigWrite, Namespace};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};

use crate::domains::persistence::{PgPersistenceRepository, RepositoryError};

const RECENT_HIVE_LIMIT: usize = 6;

/// Persistence failure returned by the PostgreSQL adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigRepositoryError {
    Unavailable,
    Conflict,
    InvalidRow,
    WriteFailed,
}

impl Display for ConfigRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "configuration repository is unavailable",
            Self::Conflict => "configuration revision changed",
            Self::InvalidRow => "configuration repository returned an invalid row",
            Self::WriteFailed => "configuration write failed",
        })
    }
}

impl std::error::Error for ConfigRepositoryError {}

/// Application-level failure with validation separated from storage failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigServiceError {
    InvalidValue,
    Conflict,
    Repository(ConfigRepositoryError),
}

impl Display for ConfigServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidValue => "configuration contains an invalid value",
            Self::Conflict => "configuration changed; reload and retry",
            Self::Repository(error) => return Display::fmt(error, formatter),
        })
    }
}

impl std::error::Error for ConfigServiceError {}

/// PostgreSQL-backed repository boundary.
///
/// The adapter must perform `save_if_revision` as one transaction using
/// `UPDATE ... WHERE revision = expected_revision RETURNING revision`. Returning
/// zero rows is `Conflict`; the adapter must never retry a stale full snapshot.
pub trait ConfigRepository: Sync {
    fn load(&self) -> impl Future<Output = Result<PublicConfig, ConfigRepositoryError>> + Send;

    fn save_if_revision(
        &self,
        expected_revision: i64,
        config: PublicConfig,
    ) -> impl Future<Output = Result<PublicConfig, ConfigRepositoryError>> + Send;
}

impl ConfigRepository for PgPersistenceRepository {
    async fn load(&self) -> Result<PublicConfig, ConfigRepositoryError> {
        match self
            .load_app_config()
            .await
            .map_err(map_persistence_error)?
        {
            Some(document) => decode_document(&document.payload_json, document.revision),
            None => Ok(PublicConfig::default()),
        }
    }

    async fn save_if_revision(
        &self,
        expected_revision: i64,
        mut config: PublicConfig,
    ) -> Result<PublicConfig, ConfigRepositoryError> {
        config.revision = expected_revision
            .checked_add(1)
            .ok_or(ConfigRepositoryError::InvalidRow)?;
        let payload_json =
            serde_json::to_string(&config).map_err(|_| ConfigRepositoryError::InvalidRow)?;
        let document = self
            .write_app_config(&AppConfigWrite {
                expected_revision,
                payload_json,
            })
            .await
            .map_err(map_persistence_error)?;
        decode_document(&document.payload_json, document.revision)
    }
}

/// Creates the PostgreSQL configuration adapter from the canonical `MD_PG_*`
/// environment. This build only accepts loopback while its SQLx pool has no TLS.
pub async fn connect_from_environment() -> Result<PgPersistenceRepository, ConfigRepositoryError> {
    let host = required_environment("MD_PG_HOST")?;
    if !matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1") {
        return Err(ConfigRepositoryError::Unavailable);
    }
    if std::env::var("MD_PG_TLS_CA")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        return Err(ConfigRepositoryError::Unavailable);
    }
    let port = std::env::var("MD_PG_PORT")
        .ok()
        .map_or(Ok(5432_u16), |value| {
            value
                .parse::<u16>()
                .map_err(|_| ConfigRepositoryError::Unavailable)
        })?;
    let namespace = Namespace::parse(required_environment("MD_PG_NAMESPACE")?)
        .ok_or(ConfigRepositoryError::Unavailable)?;
    let options = PgConnectOptions::new_without_pgpass()
        .host(&host)
        .port(port)
        .database(&required_environment("MD_PG_DATABASE")?)
        .username(&required_environment("MD_PG_USER")?)
        .password(&required_environment("MD_PG_PASSWORD")?)
        .ssl_mode(PgSslMode::Disable)
        .options([
            ("statement_timeout", "5000"),
            ("lock_timeout", "2000"),
            ("search_path", "pg_catalog"),
        ]);
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(4))
        .connect_with(options)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    Ok(PgPersistenceRepository::new(pool, namespace))
}

/// Loads the current browser-safe configuration.
pub async fn load_config(
    repository: &impl ConfigRepository,
) -> Result<PublicConfig, ConfigServiceError> {
    repository
        .load()
        .await
        .map_err(ConfigServiceError::Repository)
}

/// Validates and applies a public patch through a compare-and-swap write.
pub async fn patch_config(
    repository: &impl ConfigRepository,
    patch: ConfigPatch,
) -> Result<PublicConfig, ConfigServiceError> {
    let expected_revision = patch.expected_revision;
    let mut current = repository
        .load()
        .await
        .map_err(ConfigServiceError::Repository)?;
    if current.revision != expected_revision {
        return Err(ConfigServiceError::Conflict);
    }
    validate_patch(&patch)?;
    patch.apply_to(&mut current);
    repository
        .save_if_revision(expected_revision, current)
        .await
        .map_err(map_repository_error)
}

pub async fn set_agent_token_cap(
    repository: &impl ConfigRepository,
    request: SetAgentTokenCapRequest,
) -> Result<ConfigRuntimeReceipt, ConfigServiceError> {
    let agent_id = request.agent_id.trim();
    if agent_id.is_empty()
        || agent_id.len() > 128
        || agent_id
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        || request
            .token_cap
            .is_some_and(|cap| cap == 0 || cap > MAX_AGENT_TOKEN_CAP)
    {
        return Err(ConfigServiceError::InvalidValue);
    }
    let mut current = repository
        .load()
        .await
        .map_err(ConfigServiceError::Repository)?;
    if current.revision != request.expected_revision {
        return Err(ConfigServiceError::Conflict);
    }
    match request.token_cap {
        Some(cap) => {
            current.agent_token_caps.insert(String::from(agent_id), cap);
        }
        None => {
            current.agent_token_caps.remove(agent_id);
        }
    }
    let config = repository
        .save_if_revision(request.expected_revision, current)
        .await
        .map_err(map_repository_error)?;
    Ok(ConfigRuntimeReceipt {
        config,
        reinitialize: RuntimeReinitialize::AgentBudgets,
    })
}

pub async fn change_home(
    repository: &impl ConfigRepository,
    request: ChangeHomeRequest,
) -> Result<ConfigRuntimeReceipt, ConfigServiceError> {
    if !std::path::Path::new(request.harness_home.trim()).is_absolute() {
        return Err(ConfigServiceError::InvalidValue);
    }
    let mut current = repository
        .load()
        .await
        .map_err(ConfigServiceError::Repository)?;
    if current.revision != request.expected_revision {
        return Err(ConfigServiceError::Conflict);
    }
    let home = request.harness_home.trim();
    current.harness_home = Some(String::from(home));
    push_recent_hive(&mut current.recent_hives, home);
    let config = repository
        .save_if_revision(request.expected_revision, current)
        .await
        .map_err(map_repository_error)?;
    Ok(ConfigRuntimeReceipt {
        config,
        reinitialize: RuntimeReinitialize::HarnessHome,
    })
}

/// Finishes onboarding atomically and records the selected home in recent hives.
pub async fn finish_onboarding(
    repository: &impl ConfigRepository,
    request: FinishOnboardingRequest,
    resolved_skills: &[LocalSkill],
) -> Result<FinishOnboardingResult, ConfigServiceError> {
    request
        .validate()
        .map_err(|_| ConfigServiceError::InvalidValue)?;
    let role_skill_assignments =
        super::resolve_minimal_team(resolved_skills, &request.base_skill_managed_id)
            .map_err(|_| ConfigServiceError::InvalidValue)?;
    let mut current = repository
        .load()
        .await
        .map_err(ConfigServiceError::Repository)?;
    if current.revision != request.expected_revision {
        return Err(ConfigServiceError::Conflict);
    }

    let previous_phase = current.onboarding_phase;
    let repairing = current.requires_onboarding()
        && (current.onboarding_complete
            || current.onboarding_phase == OnboardingPhase::RepairRequired);
    let home = request.harness_home.trim();
    let workspace = request.workspace_cwd.trim();
    current.onboarding_complete = false;
    current.onboarding_phase = OnboardingPhase::TeamStarting;
    current.team_initialized = false;
    current.onboarding_repairing = repairing;
    current.audience = request.audience;
    current.harness_home = Some(String::from(home));
    current.registered_repos = normalized_non_empty(request.registered_repos);
    current.workspace_cwd = Some(String::from(workspace));
    current.auto_mode = request.auto_mode;
    current.god_provider = request.god_provider;
    current.god_model = request.god_model.filter(|model| !model.trim().is_empty());
    current.onboarding_role_skills = role_skill_assignments;
    current.telemetry_enabled = request.telemetry_enabled;
    push_recent_hive(&mut current.recent_hives, home);

    let persisted = repository
        .save_if_revision(request.expected_revision, current)
        .await
        .map_err(map_repository_error)?;
    let command = persisted
        .god_provider
        .aria_command()
        .ok_or(ConfigServiceError::InvalidValue)?;
    let model = persisted
        .god_model
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigServiceError::InvalidValue)?;
    Ok(FinishOnboardingResult {
        aria: AriaSpawnRecipe {
            id: String::from("god"),
            name: String::from("Aria"),
            provider: persisted.god_provider.clone(),
            model,
            command: String::from(command),
            cwd: persisted.workspace_cwd.clone().unwrap_or_default(),
            orchestrator: true,
        },
        role_skill_assignments: persisted.onboarding_role_skills.clone(),
        config: persisted,
        repair: repairing.then_some(OnboardingRepairReceipt {
            previous_phase,
            configuration_repaired: true,
        }),
    })
}

/// Completes the onboarding saga only after the runtime confirms all three roles.
pub async fn confirm_team_initialized(
    repository: &impl ConfigRepository,
    request: ConfirmTeamInitializedRequest,
) -> Result<ConfirmTeamInitializedResult, ConfigServiceError> {
    if !has_exact_roles(&request.initialized_roles) {
        return Err(ConfigServiceError::InvalidValue);
    }
    let mut current = repository
        .load()
        .await
        .map_err(ConfigServiceError::Repository)?;
    if current.revision != request.expected_revision {
        return Err(ConfigServiceError::Conflict);
    }
    if current.onboarding_phase != OnboardingPhase::TeamStarting
        || !has_exact_assignment_roles(&current)
        || current.workspace_cwd.as_ref().is_none_or(|workspace| {
            !current
                .registered_repos
                .iter()
                .any(|repo| repo == workspace)
        })
    {
        return Err(ConfigServiceError::InvalidValue);
    }
    let repair = current
        .onboarding_repairing
        .then_some(OnboardingRepairReceipt {
            previous_phase: OnboardingPhase::RepairRequired,
            configuration_repaired: true,
        });
    current.team_initialized = true;
    current.onboarding_phase = OnboardingPhase::Complete;
    current.onboarding_complete = true;
    current.onboarding_repairing = false;
    let config = repository
        .save_if_revision(request.expected_revision, current)
        .await
        .map_err(map_repository_error)?;
    Ok(ConfirmTeamInitializedResult {
        config,
        navigation: OnboardingNavigation::Office,
        repair,
    })
}

fn has_exact_roles(roles: &[TeamRole]) -> bool {
    roles.len() == 3
        && [TeamRole::Aria, TeamRole::Implementer, TeamRole::Verifier]
            .into_iter()
            .all(|role| roles.iter().filter(|candidate| **candidate == role).count() == 1)
}

fn has_exact_assignment_roles(config: &PublicConfig) -> bool {
    let roles = config
        .onboarding_role_skills
        .iter()
        .map(|assignment| assignment.role)
        .collect::<Vec<_>>();
    has_exact_roles(&roles)
}

fn validate_patch(patch: &ConfigPatch) -> Result<(), ConfigServiceError> {
    if patch.multi_floor == Some(true) {
        return Err(ConfigServiceError::InvalidValue);
    }
    for value in [
        patch.harness_home.as_deref(),
        patch.default_command.as_deref(),
        patch.default_model.as_deref(),
        patch.god_model.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(ConfigServiceError::InvalidValue);
        }
    }
    if patch
        .realtime_idle_disconnect_ms
        .is_some_and(|value| value > 86_400_000)
    {
        return Err(ConfigServiceError::InvalidValue);
    }
    Ok(())
}

fn normalized_non_empty(values: Vec<String>) -> Vec<String> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || result.iter().any(|existing| existing == trimmed) {
            continue;
        }
        result.push(String::from(trimmed));
    }
    result
}

fn push_recent_hive(recent: &mut Vec<String>, home: &str) {
    recent.retain(|value| value != home);
    recent.insert(0, String::from(home));
    recent.truncate(RECENT_HIVE_LIMIT);
}

fn required_environment(key: &'static str) -> Result<String, ConfigRepositoryError> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigRepositoryError::Unavailable)
}

fn decode_document(
    payload_json: &str,
    revision: i64,
) -> Result<PublicConfig, ConfigRepositoryError> {
    if revision < 0 {
        return Err(ConfigRepositoryError::InvalidRow);
    }
    let mut config: PublicConfig =
        serde_json::from_str(payload_json).map_err(|_| ConfigRepositoryError::InvalidRow)?;
    config.revision = revision;
    normalize_onboarding_state(&mut config);
    Ok(config)
}

fn normalize_onboarding_state(config: &mut PublicConfig) {
    if config.onboarding_ready() {
        return;
    }
    if config.onboarding_complete || config.onboarding_phase == OnboardingPhase::Complete {
        config.onboarding_complete = false;
        config.team_initialized = false;
        config.onboarding_phase = OnboardingPhase::RepairRequired;
        config.onboarding_repairing = true;
    } else {
        config.onboarding_complete = false;
    }
}

pub(super) fn map_persistence_error(error: RepositoryError) -> ConfigRepositoryError {
    match error {
        RepositoryError::Conflict => ConfigRepositoryError::Conflict,
        RepositoryError::InvalidInput(_) | RepositoryError::InvalidData(_) => {
            ConfigRepositoryError::InvalidRow
        }
        RepositoryError::Database(_) => ConfigRepositoryError::Unavailable,
        RepositoryError::NotFound | RepositoryError::SequenceExhausted => {
            ConfigRepositoryError::WriteFailed
        }
    }
}

const fn map_repository_error(error: ConfigRepositoryError) -> ConfigServiceError {
    match error {
        ConfigRepositoryError::Conflict => ConfigServiceError::Conflict,
        other => ConfigServiceError::Repository(other),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use md_web_contracts::domains::config_onboarding::{
        AgentProvider, Audience, ChangeHomeRequest, ConfigPatch, ConfirmTeamInitializedRequest,
        FinishOnboardingRequest, OnboardingPhase, PublicConfig, RuntimeReinitialize,
        SetAgentTokenCapRequest, TeamRole,
    };
    use md_web_contracts::domains::memory_skills::{LocalSkill, SkillProvider, SkillScope};

    use super::{
        ConfigRepository, ConfigRepositoryError, ConfigServiceError, change_home,
        confirm_team_initialized, finish_onboarding, load_config, map_persistence_error,
        normalize_onboarding_state, patch_config, set_agent_token_cap,
    };
    use crate::domains::persistence::RepositoryError;

    struct MemoryRepository {
        config: Mutex<PublicConfig>,
    }

    struct FailingRepository;

    impl ConfigRepository for FailingRepository {
        async fn load(&self) -> Result<PublicConfig, ConfigRepositoryError> {
            Ok(PublicConfig::default())
        }

        async fn save_if_revision(
            &self,
            _expected_revision: i64,
            _config: PublicConfig,
        ) -> Result<PublicConfig, ConfigRepositoryError> {
            Err(ConfigRepositoryError::WriteFailed)
        }
    }

    fn resolved_skills() -> Vec<LocalSkill> {
        [
            "aria-orchestration",
            "graph-engineering",
            "project-documentation",
            "local-development",
            "web-project-standards",
            "perfectionist-reviewer",
            "software-base",
        ]
        .into_iter()
        .map(|name| LocalSkill {
            id: format!("user:{name}"),
            name: String::from(name),
            description: String::new(),
            provider: SkillProvider::Codex,
            scope: SkillScope::User,
            managed_id: format!("0:{name}"),
        })
        .collect()
    }

    impl MemoryRepository {
        fn new(config: PublicConfig) -> Self {
            Self {
                config: Mutex::new(config),
            }
        }
    }

    impl ConfigRepository for MemoryRepository {
        async fn load(&self) -> Result<PublicConfig, ConfigRepositoryError> {
            self.config
                .lock()
                .map(|config| config.clone())
                .map_err(|_| ConfigRepositoryError::Unavailable)
        }

        async fn save_if_revision(
            &self,
            expected_revision: i64,
            mut config: PublicConfig,
        ) -> Result<PublicConfig, ConfigRepositoryError> {
            let mut current = self
                .config
                .lock()
                .map_err(|_| ConfigRepositoryError::Unavailable)?;
            if current.revision != expected_revision {
                return Err(ConfigRepositoryError::Conflict);
            }
            config.revision = expected_revision + 1;
            *current = config.clone();
            Ok(config)
        }
    }

    #[tokio::test]
    async fn load_returns_repository_snapshot() {
        let repository = MemoryRepository::new(PublicConfig::default());

        let result = load_config(&repository).await;

        assert!(matches!(result, Ok(config) if config.revision == 0));
    }

    #[tokio::test]
    async fn stale_patch_is_rejected() {
        let repository = MemoryRepository::new(PublicConfig {
            revision: 2,
            ..PublicConfig::default()
        });
        let patch = ConfigPatch {
            expected_revision: 1,
            auto_update: Some(false),
            ..ConfigPatch::default()
        };

        let result = patch_config(&repository, patch).await;

        assert_eq!(result, Err(ConfigServiceError::Conflict));
    }

    #[tokio::test]
    async fn patch_increments_revision() {
        let repository = MemoryRepository::new(PublicConfig::default());
        let patch = ConfigPatch {
            expected_revision: 0,
            auto_update: Some(false),
            ..ConfigPatch::default()
        };

        let result = patch_config(&repository, patch).await;

        assert!(matches!(result, Ok(config) if config.revision == 1 && !config.auto_update));
    }

    #[tokio::test]
    async fn unavailable_multi_floor_cannot_be_enabled_by_patch() {
        let repository = MemoryRepository::new(PublicConfig::default());

        let result = patch_config(
            &repository,
            ConfigPatch {
                expected_revision: 0,
                multi_floor: Some(true),
                ..ConfigPatch::default()
            },
        )
        .await;

        assert_eq!(result, Err(ConfigServiceError::InvalidValue));
    }

    #[tokio::test]
    async fn onboarding_sets_home_and_deduplicates_repositories() {
        let repository = MemoryRepository::new(PublicConfig::default());
        let request = FinishOnboardingRequest {
            expected_revision: 0,
            audience: Audience::Technical,
            harness_home: String::from(" /srv/hive "),
            registered_repos: vec![String::from("/repo"), String::from("/repo")],
            workspace_cwd: String::from("/repo"),
            auto_mode: true,
            god_provider: AgentProvider::Claude,
            god_model: Some(String::from("claude-opus-4-8")),
            base_skill_managed_id: String::from("0:software-base"),
            telemetry_enabled: false,
        };

        let result = finish_onboarding(&repository, request, &resolved_skills()).await;

        assert!(matches!(
            result,
            Ok(receipt)
                if receipt.config.harness_home.as_deref() == Some("/srv/hive")
                    && receipt.config.registered_repos.len() == 1
                    && receipt.config.harness_home.as_deref() != receipt.config.workspace_cwd.as_deref()
                    && receipt.aria.cwd == "/repo"
                    && receipt.config.onboarding_phase == OnboardingPhase::TeamStarting
                    && !receipt.config.onboarding_complete
                    && receipt.aria.name == "Aria"
                    && receipt.aria.orchestrator
                    && receipt.aria.provider == AgentProvider::Claude
                    && receipt.aria.model == "claude-opus-4-8"
                    && receipt.role_skill_assignments.len() == 3
        ));
    }

    #[tokio::test]
    async fn save_failure_never_returns_a_spawn_recipe() {
        let request = FinishOnboardingRequest {
            expected_revision: 0,
            audience: Audience::Technical,
            harness_home: String::from("/srv/hive"),
            registered_repos: vec![String::from("/srv/repo")],
            workspace_cwd: String::from("/srv/repo"),
            auto_mode: true,
            god_provider: AgentProvider::Claude,
            god_model: Some(String::from("claude-opus-4-8")),
            base_skill_managed_id: String::from("0:software-base"),
            telemetry_enabled: false,
        };

        let result = finish_onboarding(&FailingRepository, request, &resolved_skills()).await;

        assert_eq!(
            result,
            Err(ConfigServiceError::Repository(
                ConfigRepositoryError::WriteFailed
            ))
        );
    }

    #[tokio::test]
    async fn spawn_failure_can_retry_before_final_completion() -> Result<(), ConfigServiceError> {
        let repository = MemoryRepository::new(PublicConfig::default());
        let start = finish_onboarding(
            &repository,
            FinishOnboardingRequest {
                expected_revision: 0,
                audience: Audience::Technical,
                harness_home: String::from("/srv/harness"),
                registered_repos: vec![String::from("/srv/repo")],
                workspace_cwd: String::from("/srv/repo"),
                auto_mode: true,
                god_provider: AgentProvider::Claude,
                god_model: Some(String::from("claude-opus-4-8")),
                base_skill_managed_id: String::from("0:software-base"),
                telemetry_enabled: false,
            },
            &resolved_skills(),
        )
        .await?;

        assert!(!start.config.onboarding_complete);
        assert_eq!(start.config.onboarding_phase, OnboardingPhase::TeamStarting);
        let completed = confirm_team_initialized(
            &repository,
            ConfirmTeamInitializedRequest {
                expected_revision: start.config.revision,
                initialized_roles: vec![TeamRole::Aria, TeamRole::Implementer, TeamRole::Verifier],
            },
        )
        .await?;

        assert!(completed.config.onboarding_ready());
        Ok(())
    }

    #[test]
    fn old_complete_state_without_team_is_migrated_to_repair() {
        let mut config = PublicConfig {
            onboarding_complete: true,
            onboarding_phase: OnboardingPhase::Draft,
            team_initialized: false,
            ..PublicConfig::default()
        };

        normalize_onboarding_state(&mut config);

        assert_eq!(config.onboarding_phase, OnboardingPhase::RepairRequired);
        assert!(config.requires_onboarding());
        assert!(config.onboarding_repairing);
    }

    #[tokio::test]
    async fn repaired_legacy_state_returns_durable_repair_receipt() -> Result<(), ConfigServiceError>
    {
        let repository = MemoryRepository::new(PublicConfig {
            onboarding_phase: OnboardingPhase::RepairRequired,
            onboarding_repairing: true,
            ..PublicConfig::default()
        });
        let start = finish_onboarding(
            &repository,
            FinishOnboardingRequest {
                expected_revision: 0,
                audience: Audience::Technical,
                harness_home: String::from("/srv/harness"),
                registered_repos: vec![String::from("/srv/repo")],
                workspace_cwd: String::from("/srv/repo"),
                auto_mode: true,
                god_provider: AgentProvider::Claude,
                god_model: Some(String::from("claude-opus-4-8")),
                base_skill_managed_id: String::from("0:software-base"),
                telemetry_enabled: false,
            },
            &resolved_skills(),
        )
        .await?;

        assert!(matches!(start.repair, Some(receipt)
            if receipt.previous_phase == OnboardingPhase::RepairRequired
                && receipt.configuration_repaired));
        assert!(start.config.onboarding_repairing);
        Ok(())
    }

    #[tokio::test]
    async fn token_cap_cas_returns_runtime_reinitialize_handoff() {
        let repository = MemoryRepository::new(PublicConfig::default());

        let result = set_agent_token_cap(
            &repository,
            SetAgentTokenCapRequest {
                expected_revision: 0,
                agent_id: String::from("god"),
                token_cap: Some(50_000),
            },
        )
        .await;

        assert!(matches!(result, Ok(receipt)
            if receipt.reinitialize == RuntimeReinitialize::AgentBudgets
                && receipt.config.agent_token_caps.get("god") == Some(&50_000)));
    }

    #[tokio::test]
    async fn change_home_rejects_relative_path_without_writing() {
        let repository = MemoryRepository::new(PublicConfig::default());

        let result = change_home(
            &repository,
            ChangeHomeRequest {
                expected_revision: 0,
                harness_home: String::from("relative/home"),
            },
        )
        .await;

        assert_eq!(result, Err(ConfigServiceError::InvalidValue));
    }

    #[test]
    fn persistence_conflict_preserves_compare_and_swap_meaning() {
        assert_eq!(
            map_persistence_error(RepositoryError::Conflict),
            ConfigRepositoryError::Conflict
        );
    }

    #[test]
    fn database_error_is_exposed_only_as_unavailable() {
        assert_eq!(
            map_persistence_error(RepositoryError::Database(sqlx::Error::RowNotFound)),
            ConfigRepositoryError::Unavailable
        );
    }
}
