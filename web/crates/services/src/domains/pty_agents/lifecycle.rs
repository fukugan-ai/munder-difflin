use std::collections::BTreeMap;

use md_web_contracts::domains::pty_agents::{
    AgentRecord, AgentStatus, PtyDimensions, RestartAgentRequest, RestoreAgentRequest,
    SpawnAgentRequest,
};

use super::error::PtyServiceError;

/// In-memory active/archive/restore indexes. Durable persistence remains the database domain's owner.
#[derive(Default)]
pub struct AgentLifecycle {
    active: BTreeMap<String, AgentRecord>,
    archived: BTreeMap<String, AgentRecord>,
    restorable: BTreeMap<String, AgentRecord>,
}

impl AgentLifecycle {
    /// Creates empty lifecycle indexes.
    pub fn new() -> Self {
        Self::default()
    }

    /// Activates one record and removes stale archive/restore copies with the same id.
    pub fn activate(&mut self, mut agent: AgentRecord) {
        agent.archived = false;
        self.archived.remove(&agent.id);
        self.restorable.remove(&agent.id);
        self.active.insert(agent.id.clone(), agent);
    }

    /// Archives an active agent while retaining its process recipe and durable identity.
    pub fn archive(&mut self, agent_id: &str) -> Result<(), PtyServiceError> {
        let mut agent = self
            .active
            .remove(agent_id)
            .ok_or(PtyServiceError::NotFound)?;
        agent.status = AgentStatus::Archived;
        agent.action_ja = String::from("アーカイブ済み");
        agent.pty_id = None;
        agent.archived = true;
        self.archived.insert(String::from(agent_id), agent);
        Ok(())
    }

    /// Marks a previously active non-orchestrator agent as eligible for one-click restore.
    pub fn mark_restorable(&mut self, agent_id: &str) -> Result<(), PtyServiceError> {
        let role = self
            .active
            .get(agent_id)
            .ok_or(PtyServiceError::NotFound)?
            .role;
        if role.orchestrator || role.assistant {
            return Err(PtyServiceError::InvalidRequest(
                "オーケストレーターとアシスタントは自動復旧されます。",
            ));
        }
        let mut agent = self
            .active
            .remove(agent_id)
            .ok_or(PtyServiceError::NotFound)?;
        agent.status = AgentStatus::Restorable;
        agent.action_ja = String::from("復元可能");
        self.restorable.insert(String::from(agent_id), agent);
        Ok(())
    }

    /// Reads one active agent without copying its process recipe.
    pub fn active(&self, agent_id: &str) -> Option<&AgentRecord> {
        self.active.get(agent_id)
    }

    /// Reads one archived agent without copying its process recipe.
    pub fn archived(&self, agent_id: &str) -> Option<&AgentRecord> {
        self.archived.get(agent_id)
    }

    /// Reads one restorable agent without copying its process recipe.
    pub fn restorable(&self, agent_id: &str) -> Option<&AgentRecord> {
        self.restorable.get(agent_id)
    }
}

/// Builds the replacement spawn before the caller stops the current process.
pub fn restart_spawn_request(
    agent: &AgentRecord,
    restart: &RestartAgentRequest,
    dimensions: PtyDimensions,
) -> Result<SpawnAgentRequest, PtyServiceError> {
    if agent.id != restart.agent_id {
        return Err(PtyServiceError::InvalidRequest(
            "再起動するエージェントIDが一致しません。",
        ));
    }
    if restart.resume && restart.provider != agent.provider {
        return Err(PtyServiceError::ResumeUnavailable);
    }
    if restart.require_resume && agent.session_id.as_deref().is_none_or(str::is_empty) {
        return Err(PtyServiceError::ResumeUnavailable);
    }
    Ok(SpawnAgentRequest {
        id: agent
            .pty_id
            .clone()
            .unwrap_or_else(|| format!("pty-{}", agent.id)),
        name: agent.name.clone(),
        provider: restart.provider,
        role: agent.role,
        description: agent.description.clone(),
        cwd: agent.cwd.clone(),
        command: agent.command.clone(),
        args: agent.args.clone(),
        model: restart.model.clone(),
        cols: dimensions.cols,
        rows: dimensions.rows,
        isolate: false,
        resume: restart.resume,
        require_resume: restart.require_resume,
        resume_session_id: if restart.resume {
            agent.session_id.clone()
        } else {
            None
        },
    })
}

/// Builds a restore spawn, re-entering the existing worktree only when the caller proved it exists.
pub fn restore_spawn_request(
    restore: &RestoreAgentRequest,
    dimensions: PtyDimensions,
    worktree_available: bool,
) -> Result<SpawnAgentRequest, PtyServiceError> {
    if restore.agent.command.trim().is_empty() || restore.agent.cwd.trim().is_empty() {
        return Err(PtyServiceError::InvalidRequest(
            "保存された起動情報が不足しています。",
        ));
    }
    let cwd = if restore.prefer_worktree && worktree_available {
        restore
            .agent
            .worktree_path
            .clone()
            .unwrap_or_else(|| restore.agent.cwd.clone())
    } else {
        restore.agent.cwd.clone()
    };
    Ok(SpawnAgentRequest {
        id: restore
            .agent
            .pty_id
            .clone()
            .unwrap_or_else(|| format!("pty-{}", restore.agent.id)),
        name: restore.agent.name.clone(),
        provider: restore.agent.provider,
        role: restore.agent.role,
        description: restore.agent.description.clone(),
        cwd,
        command: restore.agent.command.clone(),
        args: restore.agent.args.clone(),
        model: restore.agent.model.clone(),
        cols: dimensions.cols,
        rows: dimensions.rows,
        isolate: false,
        resume: true,
        require_resume: false,
        resume_session_id: restore.agent.session_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{
        AgentProvider, AgentRecord, AgentRole, AgentStatus, PtyDimensions, RestartAgentRequest,
        RestoreAgentRequest,
    };

    use super::{AgentLifecycle, restart_spawn_request, restore_spawn_request};
    use crate::domains::pty_agents::PtyServiceError;

    fn agent(id: &str) -> AgentRecord {
        AgentRecord {
            id: String::from(id),
            name: String::from("Dev"),
            provider: AgentProvider::Codex,
            role: AgentRole::default(),
            description: String::from("developer"),
            cwd: String::from("/repo"),
            command: String::from("codex"),
            args: Vec::new(),
            model: None,
            status: AgentStatus::Idle,
            action_ja: String::from("待機中"),
            pty_id: Some(format!("pty-{id}")),
            worktree_path: Some(String::from("/repo-worktree")),
            session_id: Some(String::from("session-1")),
            archived: false,
        }
    }

    #[test]
    fn new_lifecycle_has_no_active_agent() {
        let lifecycle = AgentLifecycle::new();
        assert!(lifecycle.active("missing").is_none());
    }

    #[test]
    fn activate_removes_archived_copy() {
        let mut lifecycle = AgentLifecycle::new();
        lifecycle.activate(agent("dev-1"));
        let archived = lifecycle.archive("dev-1");
        assert!(archived.is_ok());
        lifecycle.activate(agent("dev-1"));
        assert!(lifecycle.archived("dev-1").is_none());
    }

    #[test]
    fn archive_clears_live_terminal() {
        let mut lifecycle = AgentLifecycle::new();
        lifecycle.activate(agent("dev-1"));
        assert!(lifecycle.archive("dev-1").is_ok());
        assert!(matches!(
            lifecycle.archived("dev-1"),
            Some(record) if record.pty_id.is_none()
        ));
    }

    #[test]
    fn mark_restorable_moves_worker_out_of_active() {
        let mut lifecycle = AgentLifecycle::new();
        lifecycle.activate(agent("dev-1"));
        assert!(lifecycle.mark_restorable("dev-1").is_ok());
        assert!(lifecycle.active("dev-1").is_none());
        assert!(lifecycle.restorable("dev-1").is_some());
    }

    #[test]
    fn refused_orchestrator_restore_keeps_agent_active() {
        let mut lifecycle = AgentLifecycle::new();
        let mut orchestrator = agent("god");
        orchestrator.role.orchestrator = true;
        lifecycle.activate(orchestrator);
        assert!(lifecycle.mark_restorable("god").is_err());
        assert!(lifecycle.active("god").is_some());
    }

    #[test]
    fn restart_refuses_cross_provider_resume() {
        let request = RestartAgentRequest {
            agent_id: String::from("dev-1"),
            provider: AgentProvider::Claude,
            model: None,
            resume: true,
            require_resume: true,
        };
        assert!(matches!(
            restart_spawn_request(
                &agent("dev-1"),
                &request,
                PtyDimensions { cols: 80, rows: 24 }
            ),
            Err(PtyServiceError::ResumeUnavailable)
        ));
    }

    #[test]
    fn restore_reenters_available_worktree() {
        let request = RestoreAgentRequest {
            agent: agent("dev-1"),
            prefer_worktree: true,
        };
        assert!(matches!(
            restore_spawn_request(&request, PtyDimensions { cols: 80, rows: 24 }, true),
            Ok(spawn) if spawn.cwd == "/repo-worktree"
        ));
    }
}
