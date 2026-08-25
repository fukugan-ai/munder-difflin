//! Server-side behavior for configuration and browser lifecycle.

mod config_repository;
mod lifecycle;
mod onboarding_paths;
mod release;
mod reset;
mod team;
mod tool_probe;

pub use config_repository::{
    ConfigRepository, ConfigRepositoryError, ConfigServiceError, change_home,
    confirm_team_initialized, connect_from_environment, finish_onboarding, load_config,
    patch_config, set_agent_token_cap,
};
pub use lifecycle::{FloorRegistry, LifecycleError, app_info, shutdown_decision, web_capabilities};
pub use onboarding_paths::{OnboardingPathError, validate_onboarding_paths};
pub use release::{
    GitHubReleaseClient, ReleaseCheckError, ReleaseLookup, ReleaseMetadata, ReleaseSourceError,
    check_for_update, latest_release_url, resolve_release_repository,
};
pub use reset::{reset_namespace, validate_reset_request};
pub use team::{TeamSkillError, resolve_minimal_team};
pub use tool_probe::{host_platform, probe_host_tools, resolve_on_path};
