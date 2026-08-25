use std::fmt;

use md_web_contracts::domains::office_ui::{
    HiveHandoff, OfficeAgent, OfficeAgentLiveState, OfficeAgentTelemetry, OfficeLiveUpdate,
    OfficeSnapshot, OfficeTask, OfficeTheme, RestorableAgent, ThemePreference,
};

const MAX_NOTE_CHARS: usize = 2_000;
const MAX_NAME_CHARS: usize = 80;

/// Mutations accepted by the office projection. Process lifecycle stays in its
/// own service domain and feeds snapshots back here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OfficeCommand {
    ReplaceAgents(Vec<OfficeAgent>),
    ReplaceRestorable(Vec<RestorableAgent>),
    ReplaceTasks(Vec<OfficeTask>),
    AgentState(OfficeAgentLiveState),
    Handoff(HiveHandoff),
    Telemetry(OfficeAgentTelemetry),
    Select(Option<String>),
    Reorder { from_id: String, to_id: String },
    Rename { agent_id: String, name: String },
    SetNote { agent_id: String, note: String },
    SetTheme(OfficeTheme),
    SetThemePreference(ThemePreference),
    SetPaused(bool),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfficeUiError {
    AgentNotFound,
    DuplicateAgentId,
    InvalidName,
    NoteTooLong,
    InvalidProgress,
    SameReorderTarget,
}

impl fmt::Display for OfficeUiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::AgentNotFound => "agent was not found",
            Self::DuplicateAgentId => "agent identifiers must be unique",
            Self::InvalidName => "agent name must contain 1 to 80 characters",
            Self::NoteTooLong => "agent note exceeds 2000 characters",
            Self::InvalidProgress => "agent progress must be between 0 and 8",
            Self::SameReorderTarget => "source and target agents must differ",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for OfficeUiError {}

/// Compact, deterministic projection used by server functions and event fanout.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OfficeProjection {
    snapshot: OfficeSnapshot,
}

impl OfficeProjection {
    #[must_use]
    pub fn snapshot(&self) -> &OfficeSnapshot {
        &self.snapshot
    }

    pub fn apply(&mut self, command: OfficeCommand) -> Result<&OfficeSnapshot, OfficeUiError> {
        match command {
            OfficeCommand::ReplaceAgents(agents) => self.replace_agents(agents)?,
            OfficeCommand::ReplaceRestorable(restorable) => {
                self.snapshot.restorable_agents = restorable;
            }
            OfficeCommand::ReplaceTasks(tasks) => self.snapshot.tasks = tasks,
            OfficeCommand::AgentState(state) => self.set_agent_state(state)?,
            OfficeCommand::Handoff(handoff) => self.push_handoff(handoff),
            OfficeCommand::Telemetry(telemetry) => self.set_telemetry(telemetry)?,
            OfficeCommand::Select(selected) => self.select(selected)?,
            OfficeCommand::Reorder { from_id, to_id } => self.reorder(&from_id, &to_id)?,
            OfficeCommand::Rename { agent_id, name } => self.rename(&agent_id, name)?,
            OfficeCommand::SetNote { agent_id, note } => self.set_note(&agent_id, note)?,
            OfficeCommand::SetTheme(theme) => self.snapshot.theme = theme,
            OfficeCommand::SetThemePreference(preference) => {
                self.snapshot.theme_preference = preference;
            }
            OfficeCommand::SetPaused(paused) => self.snapshot.paused = paused,
        }
        self.snapshot.revision = self.snapshot.revision.saturating_add(1);
        Ok(&self.snapshot)
    }

    pub fn apply_live(
        &mut self,
        update: OfficeLiveUpdate,
    ) -> Result<&OfficeSnapshot, OfficeUiError> {
        match update {
            OfficeLiveUpdate::AgentState(state) => self.apply(OfficeCommand::AgentState(state)),
            OfficeLiveUpdate::ReplaceTasks { tasks } => {
                self.apply(OfficeCommand::ReplaceTasks(tasks))
            }
            OfficeLiveUpdate::Handoff(handoff) => self.apply(OfficeCommand::Handoff(handoff)),
            OfficeLiveUpdate::Telemetry(telemetry) => {
                self.apply(OfficeCommand::Telemetry(telemetry))
            }
            OfficeLiveUpdate::SelectAgent { agent_id } => {
                self.apply(OfficeCommand::Select(agent_id))
            }
        }
    }

    fn set_agent_state(&mut self, state: OfficeAgentLiveState) -> Result<(), OfficeUiError> {
        if state.progress_eighths > 8 {
            return Err(OfficeUiError::InvalidProgress);
        }
        let agent = self
            .snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.id == state.agent_id)
            .ok_or(OfficeUiError::AgentNotFound)?;
        agent.status = state.status;
        agent.action = state.action;
        agent.last_prompt = state.last_prompt;
        agent.progress_eighths = state.progress_eighths;
        agent.context_tokens = state.context_tokens;
        agent.context_limit = state.context_limit;
        agent.carrying = state.carrying;
        agent.has_terminal_draft = state.has_terminal_draft;
        Ok(())
    }

    fn push_handoff(&mut self, handoff: HiveHandoff) {
        const MAX_HANDOFFS: usize = 32;
        self.snapshot.handoffs.push(handoff);
        let overflow = self.snapshot.handoffs.len().saturating_sub(MAX_HANDOFFS);
        if overflow > 0 {
            self.snapshot.handoffs.drain(..overflow);
        }
    }

    fn set_telemetry(&mut self, telemetry: OfficeAgentTelemetry) -> Result<(), OfficeUiError> {
        if !self
            .snapshot
            .agents
            .iter()
            .any(|agent| agent.id == telemetry.agent_id)
        {
            return Err(OfficeUiError::AgentNotFound);
        }
        if let Some(existing) = self
            .snapshot
            .telemetry
            .iter_mut()
            .find(|existing| existing.agent_id == telemetry.agent_id)
        {
            *existing = telemetry;
        } else {
            self.snapshot.telemetry.push(telemetry);
        }
        Ok(())
    }

    fn replace_agents(&mut self, agents: Vec<OfficeAgent>) -> Result<(), OfficeUiError> {
        if agents.iter().any(|agent| agent.progress_eighths > 8) {
            return Err(OfficeUiError::InvalidProgress);
        }
        for (index, agent) in agents.iter().enumerate() {
            if agents[index + 1..]
                .iter()
                .any(|candidate| candidate.id == agent.id)
            {
                return Err(OfficeUiError::DuplicateAgentId);
            }
        }
        self.snapshot.agents = agents;
        self.snapshot.telemetry.retain(|telemetry| {
            self.snapshot
                .agents
                .iter()
                .any(|agent| agent.id == telemetry.agent_id)
        });
        if self
            .snapshot
            .selected_agent_id
            .as_ref()
            .is_some_and(|selected| {
                !self
                    .snapshot
                    .agents
                    .iter()
                    .any(|agent| &agent.id == selected)
            })
        {
            self.snapshot.selected_agent_id = None;
        }
        Ok(())
    }

    fn select(&mut self, selected: Option<String>) -> Result<(), OfficeUiError> {
        if selected
            .as_ref()
            .is_some_and(|id| !self.snapshot.agents.iter().any(|agent| &agent.id == id))
        {
            return Err(OfficeUiError::AgentNotFound);
        }
        self.snapshot.selected_agent_id = selected;
        Ok(())
    }

    fn reorder(&mut self, from_id: &str, to_id: &str) -> Result<(), OfficeUiError> {
        if from_id == to_id {
            return Err(OfficeUiError::SameReorderTarget);
        }
        let from = self
            .snapshot
            .agents
            .iter()
            .position(|agent| agent.id == from_id)
            .ok_or(OfficeUiError::AgentNotFound)?;
        let to = self
            .snapshot
            .agents
            .iter()
            .position(|agent| agent.id == to_id)
            .ok_or(OfficeUiError::AgentNotFound)?;
        let agent = self.snapshot.agents.remove(from);
        self.snapshot.agents.insert(to, agent);
        Ok(())
    }

    fn rename(&mut self, agent_id: &str, name: String) -> Result<(), OfficeUiError> {
        let trimmed = name.trim();
        let len = trimmed.chars().count();
        if len == 0 || len > MAX_NAME_CHARS {
            return Err(OfficeUiError::InvalidName);
        }
        let agent = self
            .snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
            .ok_or(OfficeUiError::AgentNotFound)?;
        trimmed.clone_into(&mut agent.name);
        Ok(())
    }

    fn set_note(&mut self, agent_id: &str, note: String) -> Result<(), OfficeUiError> {
        if note.chars().count() > MAX_NOTE_CHARS {
            return Err(OfficeUiError::NoteTooLong);
        }
        let agent = self
            .snapshot
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
            .ok_or(OfficeUiError::AgentNotFound)?;
        agent.note = note;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::office_ui::{
        Accent, AgentStatus, HiveHandoff, MessageAct, OfficeAgent, OfficeAgentLiveState,
        OfficeAgentTelemetry, OfficeCharacter, OfficeLiveUpdate, OfficeTask, TaskStatus,
    };

    use super::{OfficeCommand, OfficeProjection, OfficeUiError};

    fn agent(id: &str) -> OfficeAgent {
        OfficeAgent {
            id: String::from(id),
            name: String::from(id),
            character: OfficeCharacter::Jim,
            accent: Accent::Sky,
            status: AgentStatus::Idle,
            project: String::new(),
            action: String::new(),
            note: String::new(),
            last_prompt: String::new(),
            carrying: None,
            progress_eighths: 0,
            context_tokens: None,
            context_limit: None,
            has_terminal_draft: false,
            is_god: false,
        }
    }

    #[test]
    fn snapshot_starts_empty() {
        let projection = OfficeProjection::default();

        assert!(projection.snapshot().agents.is_empty());
    }

    #[test]
    fn replacing_agents_rejects_duplicate_ids() {
        let mut projection = OfficeProjection::default();
        let result = projection.apply(OfficeCommand::ReplaceAgents(vec![
            agent("jim"),
            agent("jim"),
        ]));

        assert!(matches!(result, Err(OfficeUiError::DuplicateAgentId)));
    }

    #[test]
    fn replacing_agents_rejects_progress_above_eight() {
        let mut invalid = agent("jim");
        invalid.progress_eighths = 9;
        let mut projection = OfficeProjection::default();
        let result = projection.apply(OfficeCommand::ReplaceAgents(vec![invalid]));

        assert!(matches!(result, Err(OfficeUiError::InvalidProgress)));
    }

    #[test]
    fn selecting_unknown_agent_fails() {
        let mut projection = OfficeProjection::default();
        let result = projection.apply(OfficeCommand::Select(Some(String::from("missing"))));

        assert!(matches!(result, Err(OfficeUiError::AgentNotFound)));
    }

    #[test]
    fn error_display_is_operator_readable() {
        assert_eq!(
            OfficeUiError::AgentNotFound.to_string(),
            "agent was not found"
        );
    }

    #[test]
    fn reorder_moves_source_into_target_slot() -> Result<(), OfficeUiError> {
        let mut projection = OfficeProjection::default();
        projection.apply(OfficeCommand::ReplaceAgents(vec![
            agent("a"),
            agent("b"),
            agent("c"),
        ]))?;
        projection.apply(OfficeCommand::Reorder {
            from_id: String::from("a"),
            to_id: String::from("c"),
        })?;

        assert_eq!(projection.snapshot().agents[2].id, "a");
        Ok(())
    }

    #[test]
    fn rename_rejects_empty_value() -> Result<(), OfficeUiError> {
        let mut projection = OfficeProjection::default();
        projection.apply(OfficeCommand::ReplaceAgents(vec![agent("a")]))?;
        let result = projection.apply(OfficeCommand::Rename {
            agent_id: String::from("a"),
            name: String::from("   "),
        });

        assert!(matches!(result, Err(OfficeUiError::InvalidName)));
        Ok(())
    }

    #[test]
    fn note_rejects_more_than_two_thousand_characters() -> Result<(), OfficeUiError> {
        let mut projection = OfficeProjection::default();
        projection.apply(OfficeCommand::ReplaceAgents(vec![agent("a")]))?;
        let result = projection.apply(OfficeCommand::SetNote {
            agent_id: String::from("a"),
            note: "a".repeat(2_001),
        });

        assert!(matches!(result, Err(OfficeUiError::NoteTooLong)));
        Ok(())
    }

    #[test]
    fn revision_saturates_at_maximum() -> Result<(), OfficeUiError> {
        let mut projection = OfficeProjection::default();
        projection.snapshot.revision = u64::MAX;
        projection.apply(OfficeCommand::SetPaused(true))?;

        assert_eq!(projection.snapshot().revision, u64::MAX);
        Ok(())
    }

    #[test]
    fn live_fan_in_updates_agent_tasks_handoff_telemetry_and_selection() -> Result<(), OfficeUiError>
    {
        let mut projection = OfficeProjection::default();
        projection.apply(OfficeCommand::ReplaceAgents(vec![agent("a"), agent("b")]))?;
        projection.apply_live(OfficeLiveUpdate::AgentState(OfficeAgentLiveState {
            agent_id: String::from("b"),
            status: AgentStatus::Working,
            action: String::from("reviewing"),
            last_prompt: String::from("inspect the patch"),
            progress_eighths: 5,
            context_tokens: Some(4_000),
            context_limit: Some(8_000),
            carrying: Some(String::from("handoff-1")),
            has_terminal_draft: true,
        }))?;
        projection.apply_live(OfficeLiveUpdate::ReplaceTasks {
            tasks: vec![OfficeTask {
                id: String::from("task-1"),
                status: TaskStatus::Doing,
                assignee: Some(String::from("b")),
                has_unanswered_human_qa: false,
            }],
        })?;
        projection.apply_live(OfficeLiveUpdate::Handoff(HiveHandoff {
            event_id: String::from("message-1"),
            sequence: 1,
            from: String::from("a"),
            targets: vec![String::from("b")],
            act: MessageAct::Inform,
            needs_human: false,
        }))?;
        projection.apply_live(OfficeLiveUpdate::Telemetry(OfficeAgentTelemetry {
            agent_id: String::from("b"),
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 80,
            cache_creation_tokens: 10,
            cost_usd_micros: 42_000,
            last_tool: Some(String::from("cargo test")),
            last_tool_duration_ms: Some(900),
            observed_at_ms: 123,
        }))?;
        projection.apply_live(OfficeLiveUpdate::SelectAgent {
            agent_id: Some(String::from("b")),
        })?;

        let snapshot = projection.snapshot();
        let b = snapshot
            .agents
            .iter()
            .find(|agent| agent.id == "b")
            .ok_or(OfficeUiError::AgentNotFound)?;
        assert_eq!(b.status, AgentStatus::Working);
        assert_eq!(b.progress_eighths, 5);
        assert_eq!(b.context_tokens, Some(4_000));
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.handoffs.len(), 1);
        assert_eq!(snapshot.telemetry[0].cost_usd_micros, 42_000);
        assert_eq!(snapshot.selected_agent_id.as_deref(), Some("b"));
        Ok(())
    }

    #[test]
    fn live_agent_state_rejects_invalid_progress_without_partial_mutation()
    -> Result<(), OfficeUiError> {
        let mut projection = OfficeProjection::default();
        projection.apply(OfficeCommand::ReplaceAgents(vec![agent("a")]))?;
        let before = projection.snapshot().clone();
        let result = projection.apply_live(OfficeLiveUpdate::AgentState(OfficeAgentLiveState {
            agent_id: String::from("a"),
            status: AgentStatus::Working,
            action: String::new(),
            last_prompt: String::new(),
            progress_eighths: 9,
            context_tokens: None,
            context_limit: None,
            carrying: None,
            has_terminal_draft: false,
        }));
        assert!(matches!(result, Err(OfficeUiError::InvalidProgress)));
        assert_eq!(projection.snapshot(), &before);
        Ok(())
    }
}
