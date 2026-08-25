use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{OnboardingPhase, RoleSkillAssignment, TeamRole};

/// Copy register selected during first-run onboarding.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    #[default]
    Technical,
    NonTechnical,
}

/// Agent engine used by the orchestrator and newly-created workers.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    #[default]
    Claude,
    Codex,
    Antigravity,
    Gemini,
    Qwen,
    OpenCode,
    Crush,
    Pi,
    Copilot,
    Cursor,
    Grok,
    Kimi,
    Custom(String),
}

impl AgentProvider {
    /// Stable provider identifier used in persistence and URLs.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Antigravity => "antigravity",
            Self::Gemini => "gemini",
            Self::Qwen => "qwen",
            Self::OpenCode => "opencode",
            Self::Crush => "crush",
            Self::Pi => "pi",
            Self::Copilot => "copilot",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::Kimi => "kimi",
            Self::Custom(value) => value,
        }
    }

    /// Command accepted by the PTY authority for the Aria orchestrator.
    pub fn aria_command(&self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            _ => None,
        }
    }

    /// Supported default model for a newly-created Aria process.
    pub fn recommended_aria_model(&self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude-opus-4-8"),
            Self::Codex => Some("gpt-5.6-codex"),
            _ => None,
        }
    }
}

/// Semantic-memory embedding profile.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingModel {
    #[default]
    MiniLm,
    EmbeddingGemma,
}

/// Terminal palette mirrored to new agent sessions.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalTheme {
    Light,
    #[default]
    Dark,
}

/// Boolean-only secret metadata safe to send to WASM.
///
/// Plaintext credentials deliberately have no representation in this contract.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecretPresence {
    pub slack_signing_secret: bool,
    pub slack_bot_token: bool,
    pub groq_api_key: bool,
    pub openai_api_key: bool,
    pub provider_keys: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProviderKeyId(String);

impl ProviderKeyId {
    pub fn new(value: String) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        (!value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')))
        .then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteOnlyProviderKey(String);

impl WriteOnlyProviderKey {
    pub fn new(value: String) -> Option<Self> {
        let trimmed = value.trim();
        (!trimmed.is_empty() && !trimmed.chars().any(char::is_control))
            .then(|| Self(String::from(trimmed)))
    }

    pub fn expose_to_server(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for WriteOnlyProviderKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WriteOnlyProviderKey([REDACTED])")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderKeyWrite {
    pub provider: ProviderKeyId,
    pub key: WriteOnlyProviderKey,
}

/// Public configuration snapshot returned to a browser client.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PublicConfig {
    /// Compare-and-swap revision. Every successful write increments it.
    pub revision: i64,
    pub onboarding_complete: bool,
    #[serde(default)]
    pub onboarding_phase: OnboardingPhase,
    #[serde(default)]
    pub team_initialized: bool,
    #[serde(default)]
    pub onboarding_repairing: bool,
    pub audience: Audience,
    pub harness_home: Option<String>,
    pub recent_hives: Vec<String>,
    pub registered_repos: Vec<String>,
    #[serde(default)]
    pub workspace_cwd: Option<String>,
    pub auto_mode: bool,
    pub orchestrator_may_spawn: bool,
    pub default_command: String,
    pub default_model: Option<String>,
    pub god_provider: AgentProvider,
    pub god_model: Option<String>,
    /// Resolved skill DTOs for the canonical onboarding team, persisted in PostgreSQL.
    #[serde(default)]
    pub onboarding_role_skills: Vec<RoleSkillAssignment>,
    #[serde(default)]
    pub agent_token_caps: BTreeMap<String, u64>,
    pub semantic_memory: bool,
    pub embedding_model: EmbeddingModel,
    pub notifications: bool,
    pub strong_keepalive: bool,
    pub auto_update: bool,
    pub telemetry_enabled: bool,
    pub multi_floor: bool,
    pub terminal_theme: TerminalTheme,
    pub freeflow_enabled: bool,
    pub realtime_idle_disconnect_ms: u64,
    pub secrets: SecretPresence,
}

impl Default for PublicConfig {
    fn default() -> Self {
        Self {
            revision: 0,
            onboarding_complete: false,
            onboarding_phase: OnboardingPhase::Draft,
            team_initialized: false,
            onboarding_repairing: false,
            audience: Audience::Technical,
            harness_home: None,
            recent_hives: Vec::new(),
            registered_repos: Vec::new(),
            workspace_cwd: None,
            auto_mode: true,
            orchestrator_may_spawn: false,
            default_command: String::from("claude"),
            default_model: Some(String::from("claude-fable-5")),
            god_provider: AgentProvider::Claude,
            god_model: Some(String::from("claude-opus-4-8")),
            onboarding_role_skills: Vec::new(),
            agent_token_caps: BTreeMap::new(),
            semantic_memory: true,
            embedding_model: EmbeddingModel::MiniLm,
            notifications: false,
            strong_keepalive: false,
            auto_update: true,
            telemetry_enabled: true,
            multi_floor: false,
            terminal_theme: TerminalTheme::Dark,
            freeflow_enabled: true,
            realtime_idle_disconnect_ms: 180_000,
            secrets: SecretPresence::default(),
        }
    }
}

impl PublicConfig {
    /// True only for a finalized saga with all three persisted role assignments.
    pub fn onboarding_ready(&self) -> bool {
        self.onboarding_complete
            && self.onboarding_phase == OnboardingPhase::Complete
            && self.team_initialized
            && has_exact_team_roles(&self.onboarding_role_skills)
    }

    /// Routes drafts, interrupted starts and inconsistent legacy snapshots to repair.
    pub fn requires_onboarding(&self) -> bool {
        !self.onboarding_ready()
    }
}

fn has_exact_team_roles(assignments: &[RoleSkillAssignment]) -> bool {
    assignments.len() == 3
        && [TeamRole::Aria, TeamRole::Implementer, TeamRole::Verifier]
            .into_iter()
            .all(|role| assignments.iter().filter(|item| item.role == role).count() == 1)
}

/// Partial browser-safe update. Secret values are handled by server-only APIs.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfigPatch {
    pub expected_revision: i64,
    pub audience: Option<Audience>,
    pub harness_home: Option<String>,
    pub registered_repos: Option<Vec<String>>,
    pub auto_mode: Option<bool>,
    pub orchestrator_may_spawn: Option<bool>,
    pub default_command: Option<String>,
    pub default_model: Option<String>,
    pub god_provider: Option<AgentProvider>,
    pub god_model: Option<String>,
    pub semantic_memory: Option<bool>,
    pub embedding_model: Option<EmbeddingModel>,
    pub notifications: Option<bool>,
    pub strong_keepalive: Option<bool>,
    pub auto_update: Option<bool>,
    pub telemetry_enabled: Option<bool>,
    pub multi_floor: Option<bool>,
    pub terminal_theme: Option<TerminalTheme>,
    pub freeflow_enabled: Option<bool>,
    pub realtime_idle_disconnect_ms: Option<u64>,
}

pub const MAX_AGENT_TOKEN_CAP: u64 = 100_000_000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetAgentTokenCapRequest {
    pub expected_revision: i64,
    pub agent_id: String,
    pub token_cap: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChangeHomeRequest {
    pub expected_revision: i64,
    pub harness_home: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeReinitialize {
    AgentBudgets,
    HarnessHome,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfigRuntimeReceipt {
    pub config: PublicConfig,
    pub reinitialize: RuntimeReinitialize,
}

impl ConfigPatch {
    /// Applies the patch without mutating secret-presence metadata.
    pub fn apply_to(self, config: &mut PublicConfig) {
        if let Some(value) = self.audience {
            config.audience = value;
        }
        if let Some(value) = self.harness_home {
            config.harness_home = Some(value);
        }
        if let Some(value) = self.registered_repos {
            config.registered_repos = value;
        }
        if let Some(value) = self.auto_mode {
            config.auto_mode = value;
        }
        if let Some(value) = self.orchestrator_may_spawn {
            config.orchestrator_may_spawn = value;
        }
        if let Some(value) = self.default_command {
            config.default_command = value;
        }
        if let Some(value) = self.default_model {
            config.default_model = Some(value);
        }
        if let Some(value) = self.god_provider {
            config.god_provider = value;
        }
        if let Some(value) = self.god_model {
            config.god_model = Some(value);
        }
        if let Some(value) = self.semantic_memory {
            config.semantic_memory = value;
        }
        if let Some(value) = self.embedding_model {
            config.embedding_model = value;
        }
        if let Some(value) = self.notifications {
            config.notifications = value;
        }
        if let Some(value) = self.strong_keepalive {
            config.strong_keepalive = value;
        }
        if let Some(value) = self.auto_update {
            config.auto_update = value;
        }
        if let Some(value) = self.telemetry_enabled {
            config.telemetry_enabled = value;
        }
        if let Some(value) = self.multi_floor {
            config.multi_floor = value;
        }
        if let Some(value) = self.terminal_theme {
            config.terminal_theme = value;
        }
        if let Some(value) = self.freeflow_enabled {
            config.freeflow_enabled = value;
        }
        if let Some(value) = self.realtime_idle_disconnect_ms {
            config.realtime_idle_disconnect_ms = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domains::config_onboarding::{OnboardingPhase, RoleSkillAssignment, TeamRole};

    use super::{AgentProvider, ConfigPatch, PublicConfig, SecretPresence, WriteOnlyProviderKey};

    #[test]
    fn defaults_do_not_claim_any_secret() {
        assert_eq!(PublicConfig::default().secrets, SecretPresence::default());
    }

    #[test]
    fn patch_keeps_secret_presence() {
        let mut config = PublicConfig {
            secrets: SecretPresence {
                openai_api_key: true,
                ..SecretPresence::default()
            },
            ..PublicConfig::default()
        };
        ConfigPatch {
            expected_revision: 0,
            auto_update: Some(false),
            ..ConfigPatch::default()
        }
        .apply_to(&mut config);

        assert!(config.secrets.openai_api_key);
    }

    #[test]
    fn custom_provider_preserves_identifier() {
        let provider = AgentProvider::Custom(String::from("local-engine"));

        assert_eq!(provider.as_str(), "local-engine");
    }

    #[test]
    fn aria_profiles_are_limited_to_pty_supported_engines() {
        assert_eq!(AgentProvider::Claude.aria_command(), Some("claude"));
        assert_eq!(AgentProvider::Codex.aria_command(), Some("codex"));
        assert_eq!(AgentProvider::Gemini.aria_command(), None);
    }

    #[test]
    fn provider_key_debug_never_contains_secret() {
        let key = WriteOnlyProviderKey::new(String::from("secret-value"));

        assert!(
            matches!(key, Some(value) if format!("{value:?}") == "WriteOnlyProviderKey([REDACTED])")
        );
    }

    #[test]
    fn legacy_complete_without_initialized_team_requires_repair() {
        let config = PublicConfig {
            onboarding_complete: true,
            ..PublicConfig::default()
        };

        assert!(config.requires_onboarding());
    }

    #[test]
    fn complete_phase_requires_all_three_role_assignments() {
        let config = PublicConfig {
            onboarding_complete: true,
            onboarding_phase: OnboardingPhase::Complete,
            team_initialized: true,
            onboarding_role_skills: [TeamRole::Aria, TeamRole::Implementer, TeamRole::Verifier]
                .into_iter()
                .map(|role| RoleSkillAssignment {
                    role,
                    skills: Vec::new(),
                })
                .collect(),
            ..PublicConfig::default()
        };

        assert!(config.onboarding_ready());
    }
}
