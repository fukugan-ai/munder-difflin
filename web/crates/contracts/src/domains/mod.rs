use serde::{Deserialize, Serialize};

pub mod config_onboarding;
pub mod connections;
pub mod fs_git_ide;
pub mod hive_tasks;
pub mod memory_skills;
pub mod office_ui;
pub mod persistence;
pub mod pty_agents;
pub mod voice_realtime;

/// Stable identity for each independently migrated Web domain.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainId {
    ConfigOnboarding,
    Connections,
    FsGitIde,
    HiveTasks,
    MemorySkills,
    OfficeUi,
    PtyAgents,
    VoiceRealtime,
}

/// Lightweight notification that tells the client which domain to refresh.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DomainInvalidated {
    pub domain: DomainId,
    pub revision: u64,
}

#[cfg(test)]
mod tests {
    use super::{DomainId, DomainInvalidated};

    #[test]
    fn invalidation_accepts_maximum_revision() {
        let invalidated = DomainInvalidated {
            domain: DomainId::HiveTasks,
            revision: u64::MAX,
        };

        assert_eq!(invalidated.revision, u64::MAX);
    }
}
