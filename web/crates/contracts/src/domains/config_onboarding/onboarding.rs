use serde::{Deserialize, Serialize};

use super::{AgentProvider, Audience, PublicConfig};
use crate::domains::memory_skills::LocalSkill;

/// Ordered first-run screens in the browser experience.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    #[default]
    Persona,
    Welcome,
    Home,
    Orchestrator,
    Repositories,
    Team,
    Reliability,
    Done,
}

/// Durable first-run saga phase. A configured team is not complete until all
/// three canonical roles have been confirmed by the runtime integration.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingPhase {
    #[default]
    Draft,
    TeamStarting,
    Complete,
    RepairRequired,
}

impl OnboardingStep {
    /// Advances one screen and remains at the terminal screen once complete.
    pub const fn next(self) -> Self {
        match self {
            Self::Persona => Self::Welcome,
            Self::Welcome => Self::Home,
            Self::Home => Self::Orchestrator,
            Self::Orchestrator => Self::Repositories,
            Self::Repositories => Self::Team,
            Self::Team => Self::Reliability,
            Self::Reliability | Self::Done => Self::Done,
        }
    }

    /// Moves one screen back and remains at the first screen at the boundary.
    pub const fn previous(self) -> Self {
        match self {
            Self::Persona | Self::Welcome => Self::Persona,
            Self::Home => Self::Welcome,
            Self::Orchestrator => Self::Home,
            Self::Repositories => Self::Orchestrator,
            Self::Team => Self::Repositories,
            Self::Reliability => Self::Team,
            Self::Done => Self::Reliability,
        }
    }
}

/// Atomic request that finishes onboarding after server-side validation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FinishOnboardingRequest {
    pub expected_revision: i64,
    pub audience: Audience,
    pub harness_home: String,
    pub registered_repos: Vec<String>,
    pub workspace_cwd: String,
    pub auto_mode: bool,
    pub god_provider: AgentProvider,
    pub god_model: Option<String>,
    pub base_skill_managed_id: String,
    pub telemetry_enabled: bool,
}

/// Server-side path validation request used before the final save.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OnboardingPathProbeRequest {
    pub harness_home: String,
    pub registered_repos: Vec<String>,
    /// Optional for home-only settings changes; required for final onboarding.
    #[serde(default)]
    pub workspace_cwd: Option<String>,
}

/// Canonical paths proven to exist inside the configured server roots.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ValidatedOnboardingPaths {
    pub harness_home: String,
    pub registered_repos: Vec<String>,
    pub workspace_cwd: Option<String>,
}

/// Typed Aria recipe which the shared route may pass to the existing Office/PTy adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AriaSpawnRecipe {
    pub id: String,
    pub name: String,
    pub provider: AgentProvider,
    pub model: String,
    pub command: String,
    pub cwd: String,
    pub orchestrator: bool,
}

/// Stable role identifiers for the default three-person software team.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    Aria,
    Implementer,
    Verifier,
}

/// Resolved local skills assigned to one default team role.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RoleSkillAssignment {
    pub role: TeamRole,
    pub skills: Vec<LocalSkill>,
}

/// Navigation requested only after both persistence and Aria spawn succeed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingNavigation {
    Office,
}

/// Durable completion receipt. It does not claim that Aria was already spawned.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FinishOnboardingResult {
    pub config: PublicConfig,
    #[serde(alias = "michael")]
    pub aria: AriaSpawnRecipe,
    pub role_skill_assignments: Vec<RoleSkillAssignment>,
    pub repair: Option<OnboardingRepairReceipt>,
}

/// Records that an inconsistent legacy or interrupted saga was reconfigured.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OnboardingRepairReceipt {
    pub previous_phase: OnboardingPhase,
    pub configuration_repaired: bool,
}

/// Runtime acknowledgement emitted only after all canonical team roles exist.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfirmTeamInitializedRequest {
    pub expected_revision: i64,
    pub initialized_roles: Vec<TeamRole>,
}

/// Final saga receipt. Only this receipt may unlock Office navigation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfirmTeamInitializedResult {
    pub config: PublicConfig,
    pub navigation: OnboardingNavigation,
    pub repair: Option<OnboardingRepairReceipt>,
}

/// Stable validation failure for the first-run form.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingValidationError {
    HomeRequired,
    HomeContainsControlCharacter,
    HomeMustBeAbsolute,
    RepositoryRequired,
    RepositoryPathInvalid,
    WorkspaceRequired,
    WorkspaceMustBeRegistered,
    ProviderRequired,
    ProviderUnsupported,
    ModelRequired,
    ModelContainsControlCharacter,
    BaseSkillRequired,
}

impl FinishOnboardingRequest {
    /// Rejects values that cannot be safely handed to the server-side home service.
    pub fn validate(&self) -> Result<(), OnboardingValidationError> {
        let home = self.harness_home.trim();
        if home.is_empty() {
            return Err(OnboardingValidationError::HomeRequired);
        }
        if home.chars().any(char::is_control) {
            return Err(OnboardingValidationError::HomeContainsControlCharacter);
        }
        if !std::path::Path::new(home).is_absolute() {
            return Err(OnboardingValidationError::HomeMustBeAbsolute);
        }
        if self.registered_repos.is_empty() {
            return Err(OnboardingValidationError::RepositoryRequired);
        }
        if self.registered_repos.iter().any(|repository| {
            repository.trim().is_empty()
                || repository.chars().any(char::is_control)
                || !std::path::Path::new(repository.trim()).is_absolute()
        }) {
            return Err(OnboardingValidationError::RepositoryPathInvalid);
        }
        let workspace = self.workspace_cwd.trim();
        if workspace.is_empty() {
            return Err(OnboardingValidationError::WorkspaceRequired);
        }
        if !self
            .registered_repos
            .iter()
            .any(|repository| repository.trim() == workspace)
        {
            return Err(OnboardingValidationError::WorkspaceMustBeRegistered);
        }
        if self.god_provider.as_str().trim().is_empty() {
            return Err(OnboardingValidationError::ProviderRequired);
        }
        if self.god_provider.aria_command().is_none() {
            return Err(OnboardingValidationError::ProviderUnsupported);
        }
        let model = self.god_model.as_deref().unwrap_or_default().trim();
        if model.is_empty() {
            return Err(OnboardingValidationError::ModelRequired);
        }
        if model.chars().any(char::is_control) {
            return Err(OnboardingValidationError::ModelContainsControlCharacter);
        }
        if self.base_skill_managed_id.trim().is_empty() {
            return Err(OnboardingValidationError::BaseSkillRequired);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{FinishOnboardingRequest, OnboardingStep, OnboardingValidationError};
    use crate::domains::config_onboarding::{AgentProvider, Audience};

    fn request(home: &str) -> FinishOnboardingRequest {
        FinishOnboardingRequest {
            expected_revision: 0,
            audience: Audience::Technical,
            harness_home: String::from(home),
            registered_repos: Vec::new(),
            workspace_cwd: String::new(),
            auto_mode: true,
            god_provider: AgentProvider::Claude,
            god_model: Some(String::from("claude-opus-4-8")),
            base_skill_managed_id: String::from("0:software-base"),
            telemetry_enabled: true,
        }
    }

    #[test]
    fn onboarding_step_stops_at_done() {
        assert_eq!(OnboardingStep::Done.next(), OnboardingStep::Done);
    }

    #[test]
    fn onboarding_step_stops_at_persona_when_moving_back() {
        assert_eq!(OnboardingStep::Persona.previous(), OnboardingStep::Persona);
    }

    #[test]
    fn whitespace_home_is_rejected() {
        assert_eq!(
            request("  ").validate(),
            Err(OnboardingValidationError::HomeRequired)
        );
    }

    #[test]
    fn absolute_home_is_accepted() {
        let mut value = request("/srv/munder-difflin");
        value.registered_repos = vec![String::from("/srv/project")];
        value.workspace_cwd = String::from("/srv/project");

        assert_eq!(value.validate(), Ok(()));
    }

    #[test]
    fn relative_home_is_rejected() {
        assert_eq!(
            request("HarnessAgents").validate(),
            Err(OnboardingValidationError::HomeMustBeAbsolute)
        );
    }

    #[test]
    fn supported_provider_requires_a_model() {
        let mut value = request("/srv/munder-difflin");
        value.registered_repos = vec![String::from("/srv/project")];
        value.workspace_cwd = String::from("/srv/project");
        value.god_model = None;

        assert_eq!(
            value.validate(),
            Err(OnboardingValidationError::ModelRequired)
        );
    }

    #[test]
    fn harness_home_and_workspace_are_independent() {
        let mut value = request("/srv/harness");
        value.registered_repos = vec![String::from("/srv/project")];
        value.workspace_cwd = String::from("/srv/project");

        assert_eq!(value.validate(), Ok(()));
        assert_ne!(value.harness_home, value.workspace_cwd);
    }
}
