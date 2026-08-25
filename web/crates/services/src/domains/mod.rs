use md_web_contracts::DomainId;

pub mod config_onboarding;
pub mod connections;
pub mod fs_git_ide;
pub mod hive_tasks;
pub mod memory_skills;
pub mod office_ui;
pub mod persistence;
pub mod pty_agents;
pub mod voice_realtime;

const REGISTERED_DOMAINS: &[DomainId] = &[
    DomainId::ConfigOnboarding,
    DomainId::Connections,
    DomainId::FsGitIde,
    DomainId::HiveTasks,
    DomainId::MemorySkills,
    DomainId::OfficeUi,
    DomainId::PtyAgents,
    DomainId::VoiceRealtime,
];

/// Compile-time registry populated as domain services are integrated.
#[derive(Clone, Copy, Debug)]
pub struct DomainRegistry {
    registered: &'static [DomainId],
}

impl DomainRegistry {
    pub(crate) const fn integrated() -> Self {
        Self {
            registered: REGISTERED_DOMAINS,
        }
    }

    /// Returns the domains whose service modules are part of this build.
    pub fn registered(&self) -> &'static [DomainId] {
        self.registered
    }
}

#[cfg(test)]
mod tests {
    use super::DomainRegistry;

    #[test]
    fn registry_contains_integrated_domains() {
        assert_eq!(DomainRegistry::integrated().registered().len(), 8);
    }
}
