//! Browser-safe contracts for configuration, onboarding, prerequisites, updates,
//! and Web application lifecycle.

mod config;
mod lifecycle;
mod onboarding;
mod prerequisite;
mod update;

pub use config::{
    AgentProvider, Audience, ChangeHomeRequest, ConfigPatch, ConfigRuntimeReceipt, EmbeddingModel,
    MAX_AGENT_TOKEN_CAP, ProviderKeyId, ProviderKeyWrite, PublicConfig, RuntimeReinitialize,
    SecretPresence, SetAgentTokenCapRequest, TerminalTheme, WriteOnlyProviderKey,
};
pub use lifecycle::{
    AppCapability, AppInfo, CapabilityAvailability, CapabilitySupport, CreateFloorRequest,
    CreateFloorResponse, FloorId, ResetNamespaceRequest, ResetResult, ShutdownRequest,
    ShutdownResult,
};
pub use onboarding::{
    AriaSpawnRecipe, ConfirmTeamInitializedRequest, ConfirmTeamInitializedResult,
    FinishOnboardingRequest, FinishOnboardingResult, OnboardingNavigation,
    OnboardingPathProbeRequest, OnboardingPhase, OnboardingRepairReceipt, OnboardingStep,
    OnboardingValidationError, RoleSkillAssignment, TeamRole, ValidatedOnboardingPaths,
};
pub use prerequisite::{HostPlatform, ToolKind, ToolStatus};
pub use update::{ReleaseRepository, UpdateAction, UpdateStatus};
