mod activity_panel;
mod base_skills_panel;
mod history_panel;
mod knowledge_panel;
mod memory_panel;
mod skills_panel;
mod telemetry_panel;
mod workspace;

#[allow(
    unused_imports,
    reason = "mounted by the shared onboarding route after domain integration"
)]
pub(crate) use base_skills_panel::BaseSkillsOnboardingPanel;
pub(crate) use workspace::MemorySkillsWorkspace;
