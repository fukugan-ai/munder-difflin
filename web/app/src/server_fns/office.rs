use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::{
    CompletionNotice, OfficeAgentSpawnRequest, OfficeLiveUpdate, OfficeSnapshot, OfficeTheme,
    OfficeUiState, ThemePreference,
};
use md_web_contracts::domains::pty_agents::SpawnAgentResult;

#[cfg(feature = "server")]
mod server {
    use std::collections::{BTreeMap, BTreeSet, HashMap};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use md_web_contracts::domains::office_ui::{
        Accent, AgentStatus, CompletionNotice, OfficeAgent, OfficeAgentLiveState,
        OfficeAgentSpawnRequest, OfficeCharacter, OfficeLiveUpdate, OfficeSnapshot, OfficeTheme,
        OfficeUiState, RestorableAgent, ThemePreference,
    };
    use md_web_contracts::domains::pty_agents::{
        AgentRecord, AgentStatus as ProcessStatus, SpawnAgentResult,
    };
    use md_web_services::domains::office_ui::{
        CompletionToastStack, OfficeCommand, OfficeProjection,
    };
    #[derive(Clone)]
    struct AgentVisual {
        character: OfficeCharacter,
        accent: Accent,
        project: String,
        goal: String,
        name: Option<String>,
    }

    struct OfficeRuntime {
        projection: OfficeProjection,
        toasts: CompletionToastStack,
        focus_mode: bool,
        auto_mode: bool,
        visuals: BTreeMap<String, AgentVisual>,
        persistence_revision: i64,
        hydrated: bool,
        dismissed_restorable_agent_ids: BTreeSet<String>,
    }

    impl OfficeRuntime {
        fn new() -> Self {
            Self {
                projection: OfficeProjection::default(),
                toasts: CompletionToastStack::default(),
                focus_mode: false,
                auto_mode: true,
                visuals: BTreeMap::new(),
                persistence_revision: 0,
                hydrated: false,
                dismissed_restorable_agent_ids: BTreeSet::new(),
            }
        }
    }

    static RUNTIME: OnceLock<Mutex<OfficeRuntime>> = OnceLock::new();

    fn runtime() -> &'static Mutex<OfficeRuntime> {
        RUNTIME.get_or_init(|| Mutex::new(OfficeRuntime::new()))
    }

    pub(super) fn reset_runtime_projection() -> Result<(), ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        reset_runtime_state(&mut state);
        Ok(())
    }

    fn reset_runtime_state(state: &mut OfficeRuntime) {
        *state = OfficeRuntime::new();
    }

    pub(super) fn snapshot(
        mut active: Vec<AgentRecord>,
        restorable: Vec<AgentRecord>,
    ) -> Result<OfficeUiState, ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        let existing = state.projection.snapshot().clone();
        let order: HashMap<&str, usize> = existing
            .agents
            .iter()
            .enumerate()
            .map(|(index, agent)| (agent.id.as_str(), index))
            .collect();
        active.sort_by_key(|record| order.get(record.id.as_str()).copied().unwrap_or(usize::MAX));

        let agents = active
            .iter()
            .enumerate()
            .map(|(index, record)| {
                let previous = existing.agents.iter().find(|agent| agent.id == record.id);
                to_office_agent(record, index, previous, state.visuals.get(&record.id))
            })
            .collect();
        state
            .projection
            .apply(OfficeCommand::ReplaceAgents(agents))
            .map_err(|_| ())?;
        let visible_restorable = restorable
            .iter()
            .filter(|record| !state.dismissed_restorable_agent_ids.contains(&record.id))
            .map(to_restorable)
            .collect();
        state
            .projection
            .apply(OfficeCommand::ReplaceRestorable(visible_restorable))
            .map_err(|_| ())?;

        if state.projection.snapshot().selected_agent_id.is_none()
            && let Some(first) = state
                .projection
                .snapshot()
                .agents
                .iter()
                .find(|agent| agent.is_god)
                .or_else(|| state.projection.snapshot().agents.first())
        {
            let id = first.id.clone();
            state
                .projection
                .apply(OfficeCommand::Select(Some(id)))
                .map_err(|_| ())?;
        }

        state.toasts.expire(now_ms());
        Ok(OfficeUiState {
            snapshot: state.projection.snapshot().clone(),
            notices: state.toasts.notices().to_vec(),
            focus_mode: state.focus_mode,
            auto_mode: state.auto_mode,
            dismissed_restorable_agent_ids: state
                .dismissed_restorable_agent_ids
                .iter()
                .cloned()
                .collect(),
        })
    }

    pub(super) fn hydrate(payload: Option<(&str, i64)>) -> Result<(), ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        if state.hydrated {
            return Ok(());
        }
        if let Some((payload, revision)) = payload {
            let mut persisted: OfficeUiState = serde_json::from_str(payload).map_err(|_| ())?;
            clear_ephemeral_projection(&mut persisted.snapshot);
            state
                .projection
                .apply(OfficeCommand::ReplaceAgents(persisted.snapshot.agents))
                .map_err(|_| ())?;
            state
                .projection
                .apply(OfficeCommand::ReplaceRestorable(
                    persisted.snapshot.restorable_agents,
                ))
                .map_err(|_| ())?;
            state
                .projection
                .apply(OfficeCommand::SetTheme(persisted.snapshot.theme))
                .map_err(|_| ())?;
            state
                .projection
                .apply(OfficeCommand::SetThemePreference(
                    persisted.snapshot.theme_preference,
                ))
                .map_err(|_| ())?;
            state
                .projection
                .apply(OfficeCommand::SetPaused(persisted.snapshot.paused))
                .map_err(|_| ())?;
            state
                .projection
                .apply(OfficeCommand::Select(persisted.snapshot.selected_agent_id))
                .map_err(|_| ())?;
            state.focus_mode = persisted.focus_mode;
            state.auto_mode = persisted.auto_mode;
            state.dismissed_restorable_agent_ids = persisted
                .dismissed_restorable_agent_ids
                .into_iter()
                .collect();
            state.persistence_revision = revision;
        }
        state.hydrated = true;
        Ok(())
    }

    pub(super) fn persistence_payload() -> Result<(String, i64), ()> {
        let state = runtime().lock().map_err(|_| ())?;
        let mut snapshot = state.projection.snapshot().clone();
        clear_ephemeral_projection(&mut snapshot);
        serde_json::to_string(&OfficeUiState {
            snapshot,
            notices: Vec::new(),
            focus_mode: state.focus_mode,
            auto_mode: state.auto_mode,
            dismissed_restorable_agent_ids: state
                .dismissed_restorable_agent_ids
                .iter()
                .cloned()
                .collect(),
        })
        .map(|payload| (payload, state.persistence_revision))
        .map_err(|_| ())
    }

    fn clear_ephemeral_projection(snapshot: &mut OfficeSnapshot) {
        snapshot.tasks.clear();
        snapshot.handoffs.clear();
        snapshot.telemetry.clear();
        for agent in &mut snapshot.agents {
            agent.status = AgentStatus::Idle;
            agent.action.clear();
            agent.last_prompt.clear();
            agent.carrying = None;
            agent.progress_eighths = 0;
            agent.context_tokens = None;
            agent.context_limit = None;
            agent.has_terminal_draft = false;
        }
    }

    pub(super) fn set_persistence_revision(revision: i64) -> Result<(), ()> {
        runtime().lock().map_err(|_| ())?.persistence_revision = revision;
        Ok(())
    }

    pub(super) fn record_spawn(
        request: &OfficeAgentSpawnRequest,
        result: SpawnAgentResult,
    ) -> Result<SpawnAgentResult, ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        state.visuals.insert(
            request.process.id.clone(),
            AgentVisual {
                character: request.character,
                accent: request.accent,
                project: if request.project.is_empty() {
                    project_name(&request.process.cwd)
                } else {
                    request.project.clone()
                },
                goal: request.goal.clone(),
                name: Some(request.process.name.clone()),
            },
        );
        Ok(result)
    }

    pub(super) fn prepare_spawn(request: &mut OfficeAgentSpawnRequest) {
        if request.process.id.is_empty() {
            let slug: String = request
                .process
                .name
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .map(|character| character.to_ascii_lowercase())
                .collect();
            let stem = if slug.is_empty() { "agent" } else { &slug };
            request.process.id = format!("{stem}-{}", now_ms());
        }
    }

    pub(super) fn select(agent_id: Option<String>) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .projection
            .apply(OfficeCommand::Select(agent_id))
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(super) fn live_update(update: OfficeLiveUpdate) -> Result<OfficeSnapshot, ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        let current = state.projection.snapshot();
        let unchanged = match &update {
            OfficeLiveUpdate::ReplaceTasks { tasks } => current.tasks == *tasks,
            OfficeLiveUpdate::SelectAgent { agent_id } => current.selected_agent_id == *agent_id,
            OfficeLiveUpdate::Handoff(handoff) => current.handoffs.iter().any(|existing| {
                existing.event_id == handoff.event_id && existing.sequence == handoff.sequence
            }),
            OfficeLiveUpdate::Telemetry(telemetry) => current
                .telemetry
                .iter()
                .any(|existing| existing == telemetry),
            OfficeLiveUpdate::AgentState(live) => current.agents.iter().any(|agent| {
                agent.id == live.agent_id
                    && agent.status == live.status
                    && agent.action == live.action
                    && agent.last_prompt == live.last_prompt
                    && agent.progress_eighths == live.progress_eighths
                    && agent.context_tokens == live.context_tokens
                    && agent.context_limit == live.context_limit
                    && agent.carrying == live.carrying
                    && agent.has_terminal_draft == live.has_terminal_draft
            }),
        };
        if unchanged {
            return Ok(current.clone());
        }
        state.projection.apply_live(update).map_err(|_| ())?;
        Ok(state.projection.snapshot().clone())
    }

    pub(super) fn agent_activity(
        agent_id: &str,
        status: AgentStatus,
        action: &str,
        last_prompt: Option<&str>,
        has_terminal_draft: bool,
    ) -> Result<(), ()> {
        let state = runtime().lock().map_err(|_| ())?;
        let agent = state
            .projection
            .snapshot()
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .cloned()
            .ok_or(())?;
        drop(state);
        live_update(OfficeLiveUpdate::AgentState(OfficeAgentLiveState {
            agent_id: String::from(agent_id),
            status,
            action: String::from(action),
            last_prompt: last_prompt
                .map(|prompt| prompt.chars().take(256).collect())
                .unwrap_or(agent.last_prompt),
            progress_eighths: agent.progress_eighths,
            context_tokens: agent.context_tokens,
            context_limit: agent.context_limit,
            carrying: agent.carrying,
            has_terminal_draft,
        }))
        .map(|_| ())
    }

    pub(super) fn live_poll(since_revision: Option<u64>) -> Result<Option<OfficeSnapshot>, ()> {
        let state = runtime().lock().map_err(|_| ())?;
        Ok(poll_runtime_state(&state, since_revision))
    }

    fn poll_runtime_state(
        state: &OfficeRuntime,
        since_revision: Option<u64>,
    ) -> Option<OfficeSnapshot> {
        let snapshot = state.projection.snapshot();
        (since_revision != Some(snapshot.revision)).then(|| snapshot.clone())
    }

    pub(super) fn reorder(from_id: String, to_id: String) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .projection
            .apply(OfficeCommand::Reorder { from_id, to_id })
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(super) fn rename(agent_id: String, name: String) -> Result<(), ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        state
            .projection
            .apply(OfficeCommand::Rename {
                agent_id: agent_id.clone(),
                name: name.clone(),
            })
            .map_err(|_| ())?;
        if let Some(visual) = state.visuals.get_mut(&agent_id) {
            visual.name = Some(name);
        } else if let Some(agent) = state
            .projection
            .snapshot()
            .agents
            .iter()
            .find(|agent| agent.id == agent_id)
            .cloned()
        {
            state.visuals.insert(
                agent_id,
                AgentVisual {
                    character: agent.character,
                    accent: agent.accent,
                    project: agent.project,
                    goal: String::new(),
                    name: Some(name),
                },
            );
        }
        Ok(())
    }

    pub(super) fn note(agent_id: String, note: String) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .projection
            .apply(OfficeCommand::SetNote { agent_id, note })
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(super) fn theme(theme: OfficeTheme) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .projection
            .apply(OfficeCommand::SetTheme(theme))
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(super) fn theme_preference(preference: ThemePreference) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .projection
            .apply(OfficeCommand::SetThemePreference(preference))
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(super) fn pause(paused: bool) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .projection
            .apply(OfficeCommand::SetPaused(paused))
            .map(|_| ())
            .map_err(|_| ())
    }

    pub(super) fn focus(focused: bool) -> Result<(), ()> {
        runtime().lock().map_err(|_| ())?.focus_mode = focused;
        Ok(())
    }

    pub(super) fn auto_mode(enabled: bool) -> Result<(), ()> {
        runtime().lock().map_err(|_| ())?.auto_mode = enabled;
        Ok(())
    }

    pub(super) fn push_notice(notice: CompletionNotice) -> Result<(), ()> {
        runtime().lock().map_err(|_| ())?.toasts.push(notice);
        Ok(())
    }

    pub(super) fn dismiss_notice(correlation_id: &str, completed_at_ms: i64) -> Result<(), ()> {
        runtime()
            .lock()
            .map_err(|_| ())?
            .toasts
            .dismiss(correlation_id, completed_at_ms);
        Ok(())
    }

    pub(super) fn dismiss_restorable(agent_id: String) -> Result<(), ()> {
        let mut state = runtime().lock().map_err(|_| ())?;
        apply_dismiss_restorable(&mut state, agent_id)
    }

    fn apply_dismiss_restorable(state: &mut OfficeRuntime, agent_id: String) -> Result<(), ()> {
        state
            .dismissed_restorable_agent_ids
            .insert(agent_id.clone());
        let remaining = state
            .projection
            .snapshot()
            .restorable_agents
            .iter()
            .filter(|agent| agent.id != agent_id)
            .cloned()
            .collect();
        state
            .projection
            .apply(OfficeCommand::ReplaceRestorable(remaining))
            .map(|_| ())
            .map_err(|_| ())
    }

    fn to_office_agent(
        record: &AgentRecord,
        index: usize,
        previous: Option<&OfficeAgent>,
        visual: Option<&AgentVisual>,
    ) -> OfficeAgent {
        let (character, accent) = visual
            .map(|value| (value.character, value.accent))
            .or_else(|| previous.map(|value| (value.character, value.accent)))
            .unwrap_or_else(|| roster_visual(record, index));
        let project = visual
            .map(|value| value.project.clone())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| project_name(&record.cwd));
        let action = if let Some(visual) = visual
            && !visual.goal.is_empty()
            && record.action_ja.is_empty()
        {
            visual.goal.clone()
        } else if record.action_ja.is_empty() {
            previous.map_or_else(String::new, |value| value.action.clone())
        } else {
            record.action_ja.clone()
        };
        OfficeAgent {
            id: record.id.clone(),
            name: visual
                .and_then(|value| value.name.clone())
                .unwrap_or_else(|| record.name.clone()),
            character,
            accent,
            status: map_status(record.status),
            project,
            action,
            note: previous.map_or_else(String::new, |value| value.note.clone()),
            last_prompt: previous.map_or_else(String::new, |value| value.last_prompt.clone()),
            carrying: previous.and_then(|value| value.carrying.clone()),
            progress_eighths: previous.map_or(0, |value| value.progress_eighths),
            context_tokens: previous.and_then(|value| value.context_tokens),
            context_limit: previous.and_then(|value| value.context_limit),
            has_terminal_draft: previous.is_some_and(|value| value.has_terminal_draft),
            is_god: record.role.orchestrator || record.id == "god",
        }
    }

    fn to_restorable(record: &AgentRecord) -> RestorableAgent {
        RestorableAgent {
            id: record.id.clone(),
            name: record.name.clone(),
            description: record.description.clone(),
        }
    }

    fn map_status(status: ProcessStatus) -> AgentStatus {
        match status {
            ProcessStatus::Starting => AgentStatus::Thinking,
            ProcessStatus::Idle => AgentStatus::Idle,
            ProcessStatus::Working => AgentStatus::Working,
            ProcessStatus::Waiting => AgentStatus::Waiting,
            ProcessStatus::Blocked => AgentStatus::Blocked,
            ProcessStatus::Looping => AgentStatus::Looping,
            ProcessStatus::Exited | ProcessStatus::Archived | ProcessStatus::Restorable => {
                AgentStatus::Ghost
            }
        }
    }

    fn roster_visual(record: &AgentRecord, index: usize) -> (OfficeCharacter, Accent) {
        let named = match record.name.to_ascii_lowercase().as_str() {
            "aria" => Some(OfficeCharacter::Michael),
            "michael" => Some(OfficeCharacter::Michael),
            "jim" => Some(OfficeCharacter::Jim),
            "pam" => Some(OfficeCharacter::Pam),
            "dwight" => Some(OfficeCharacter::Dwight),
            "kevin" => Some(OfficeCharacter::Kevin),
            "andy" => Some(OfficeCharacter::Andy),
            "ryan" => Some(OfficeCharacter::Ryan),
            "stanley" => Some(OfficeCharacter::Stanley),
            "meredith" => Some(OfficeCharacter::Meredith),
            "toby" => Some(OfficeCharacter::Toby),
            _ => None,
        };
        let characters = [
            OfficeCharacter::Michael,
            OfficeCharacter::Jim,
            OfficeCharacter::Pam,
            OfficeCharacter::Kevin,
            OfficeCharacter::Ryan,
            OfficeCharacter::Stanley,
            OfficeCharacter::Meredith,
            OfficeCharacter::Toby,
            OfficeCharacter::Andy,
            OfficeCharacter::Dwight,
        ];
        let accents = [
            Accent::Lemon,
            Accent::Sky,
            Accent::Lilac,
            Accent::Mint,
            Accent::Coral,
            Accent::Peach,
        ];
        (
            named.unwrap_or(characters[index % characters.len()]),
            accents[index % accents.len()],
        )
    }

    fn project_name(cwd: &str) -> String {
        cwd.rsplit(['/', '\\'])
            .find(|part| !part.is_empty())
            .map_or_else(|| String::from("workspace"), String::from)
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }

    #[cfg(test)]
    mod tests {
        use md_web_contracts::domains::office_ui::{
            Accent, AgentStatus, HiveHandoff, MessageAct, OfficeAgent, OfficeAgentLiveState,
            OfficeAgentTelemetry, OfficeCharacter, OfficeLiveUpdate, OfficeTask, OfficeTheme,
            RestorableAgent, TaskStatus, ThemePreference,
        };
        use md_web_services::domains::office_ui::OfficeCommand;

        use super::{
            OfficeRuntime, apply_dismiss_restorable, clear_ephemeral_projection,
            poll_runtime_state, reset_runtime_state,
        };

        #[test]
        fn dismiss_restore_removes_card_and_records_durable_filter() {
            let mut state = OfficeRuntime::new();
            state
                .projection
                .apply(OfficeCommand::ReplaceRestorable(vec![RestorableAgent {
                    id: String::from("darryl"),
                    name: String::from("Darryl"),
                    description: String::from("operations"),
                }]))
                .unwrap_or_else(|error| panic!("test setup failed: {error}"));

            assert!(apply_dismiss_restorable(&mut state, String::from("darryl")).is_ok());
            assert!(state.projection.snapshot().restorable_agents.is_empty());
            assert!(state.dismissed_restorable_agent_ids.contains("darryl"));
        }

        #[test]
        fn namespace_reset_clears_stale_projection_and_runtime_metadata() {
            let mut state = OfficeRuntime::new();
            let setup = state
                .projection
                .apply(OfficeCommand::ReplaceAgents(vec![OfficeAgent {
                    id: String::from("darryl"),
                    name: String::from("Darryl"),
                    character: OfficeCharacter::Darryl,
                    accent: Accent::Mint,
                    status: AgentStatus::Idle,
                    project: String::from("warehouse"),
                    action: String::new(),
                    note: String::from("stale note"),
                    last_prompt: String::new(),
                    carrying: None,
                    progress_eighths: 0,
                    context_tokens: None,
                    context_limit: None,
                    has_terminal_draft: false,
                    is_god: false,
                }]));
            assert!(setup.is_ok());
            assert!(
                state
                    .projection
                    .apply(OfficeCommand::SetTheme(OfficeTheme::Brooklyn99))
                    .is_ok()
            );
            assert!(
                state
                    .projection
                    .apply(OfficeCommand::SetThemePreference(ThemePreference::Dark))
                    .is_ok()
            );
            state
                .dismissed_restorable_agent_ids
                .insert(String::from("darryl"));
            state.focus_mode = true;
            state.auto_mode = false;
            state.hydrated = true;
            state.persistence_revision = 42;

            reset_runtime_state(&mut state);

            let snapshot = state.projection.snapshot();
            assert!(snapshot.agents.is_empty());
            assert_eq!(snapshot.theme, OfficeTheme::Office);
            assert_eq!(snapshot.theme_preference, ThemePreference::System);
            assert!(state.dismissed_restorable_agent_ids.is_empty());
            assert!(!state.focus_mode);
            assert!(state.auto_mode);
            assert!(!state.hydrated);
            assert_eq!(state.persistence_revision, 0);
        }

        #[test]
        fn live_poll_cursor_observes_a_to_b_selection_once() {
            let mut state = OfficeRuntime::new();
            assert!(
                state
                    .projection
                    .apply(OfficeCommand::ReplaceAgents(vec![
                        OfficeAgent {
                            id: String::from("a"),
                            name: String::from("A"),
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
                        },
                        OfficeAgent {
                            id: String::from("b"),
                            name: String::from("B"),
                            character: OfficeCharacter::Pam,
                            accent: Accent::Mint,
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
                        },
                    ]))
                    .is_ok()
            );
            assert!(
                state
                    .projection
                    .apply(OfficeCommand::Select(Some(String::from("a"))))
                    .is_ok()
            );
            let revision_a = state.projection.snapshot().revision;
            assert!(poll_runtime_state(&state, Some(revision_a)).is_none());
            assert!(
                state
                    .projection
                    .apply(OfficeCommand::Select(Some(String::from("b"))))
                    .is_ok()
            );
            let Some(snapshot_b) = poll_runtime_state(&state, Some(revision_a)) else {
                panic!("selection revision should be observable");
            };
            assert_eq!(snapshot_b.selected_agent_id.as_deref(), Some("b"));
            assert!(poll_runtime_state(&state, Some(snapshot_b.revision)).is_none());
        }

        #[test]
        fn persistence_projection_excludes_live_source_owned_fields() {
            let mut state = OfficeRuntime::new();
            let durable_agent = OfficeAgent {
                id: String::from("a"),
                name: String::from("A"),
                character: OfficeCharacter::Jim,
                accent: Accent::Sky,
                status: AgentStatus::Idle,
                project: String::new(),
                action: String::new(),
                note: String::from("durable note"),
                last_prompt: String::new(),
                carrying: None,
                progress_eighths: 0,
                context_tokens: None,
                context_limit: None,
                has_terminal_draft: false,
                is_god: false,
            };
            assert!(
                state
                    .projection
                    .apply(OfficeCommand::ReplaceAgents(vec![durable_agent]))
                    .is_ok()
            );
            assert!(
                state
                    .projection
                    .apply_live(OfficeLiveUpdate::AgentState(OfficeAgentLiveState {
                        agent_id: String::from("a"),
                        status: AgentStatus::Working,
                        action: String::from("live action"),
                        last_prompt: String::from("live prompt"),
                        progress_eighths: 7,
                        context_tokens: Some(700),
                        context_limit: Some(1_000),
                        carrying: Some(String::from("message-1")),
                        has_terminal_draft: true,
                    }))
                    .is_ok()
            );
            assert!(
                state
                    .projection
                    .apply_live(OfficeLiveUpdate::ReplaceTasks {
                        tasks: vec![OfficeTask {
                            id: String::from("task-1"),
                            status: TaskStatus::Doing,
                            assignee: Some(String::from("a")),
                            has_unanswered_human_qa: false,
                        }],
                    })
                    .is_ok()
            );
            assert!(
                state
                    .projection
                    .apply_live(OfficeLiveUpdate::Handoff(HiveHandoff {
                        event_id: String::from("message-1"),
                        sequence: 1,
                        from: String::from("a"),
                        targets: vec![String::from("b")],
                        act: MessageAct::Inform,
                        needs_human: false,
                    }))
                    .is_ok()
            );
            assert!(
                state
                    .projection
                    .apply_live(OfficeLiveUpdate::Telemetry(OfficeAgentTelemetry {
                        agent_id: String::from("a"),
                        input_tokens: 10,
                        output_tokens: 2,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                        cost_usd_micros: 50,
                        last_tool: None,
                        last_tool_duration_ms: None,
                        observed_at_ms: 1,
                    }))
                    .is_ok()
            );

            let mut persisted = state.projection.snapshot().clone();
            clear_ephemeral_projection(&mut persisted);
            assert_eq!(persisted.agents[0].note, "durable note");
            assert_eq!(persisted.agents[0].status, AgentStatus::Idle);
            assert!(persisted.agents[0].last_prompt.is_empty());
            assert!(persisted.tasks.is_empty());
            assert!(persisted.handoffs.is_empty());
            assert!(persisted.telemetry.is_empty());
        }
    }
}

#[cfg_attr(
    not(feature = "server"),
    expect(dead_code, reason = "Dioxus replaces web server-function bodies")
)]
fn safe_error() -> ServerFnError {
    ServerFnError::new("オフィス状態の操作に失敗しました")
}

/// Clear only this process's Office projection after the owning PostgreSQL
/// namespace-reset transaction has committed. This performs no persistence
/// write; the next snapshot hydrates from the now-reset namespace.
#[cfg(feature = "server")]
pub(crate) fn reset_office_runtime_projection() -> Result<(), ServerFnError> {
    server::reset_runtime_projection().map_err(|_| safe_error())
}

#[cfg(feature = "server")]
static PERSISTENCE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(feature = "server")]
fn persistence_key() -> md_web_contracts::domains::persistence::RecordKey {
    use md_web_contracts::domains::persistence::{RecordDomain, RecordKey};
    RecordKey {
        domain: RecordDomain::Floors,
        kind: String::from("office_projection"),
        record_id: String::from("local"),
    }
}

#[cfg(feature = "server")]
async fn ensure_hydrated(
    repository: &md_web_services::domains::persistence::PgPersistenceRepository,
) -> Result<(), ServerFnError> {
    let record = repository
        .get_record(&persistence_key())
        .await
        .map_err(|_| safe_error())?;
    server::hydrate(
        record
            .as_ref()
            .map(|record| (record.payload_json.as_str(), record.revision)),
    )
    .map_err(|_| safe_error())
}

#[cfg(feature = "server")]
async fn persist_projection(
    repository: &md_web_services::domains::persistence::PgPersistenceRepository,
) -> Result<(), ServerFnError> {
    use md_web_contracts::domains::persistence::RecordWrite;
    let (payload_json, expected_revision) =
        server::persistence_payload().map_err(|_| safe_error())?;
    let record = repository
        .write_record(&RecordWrite {
            key: persistence_key(),
            expected_revision,
            payload_json,
        })
        .await
        .map_err(|_| safe_error())?;
    server::set_persistence_revision(record.revision).map_err(|_| safe_error())
}

#[get("/api/office/snapshot")]
pub(crate) async fn office_snapshot() -> Result<OfficeUiState, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _serial = PERSISTENCE_SERIAL.lock().await;
        let repository = super::persistence_repository().await?;
        ensure_hydrated(&repository).await?;
        let (active, restorable) = super::pty::list_agents().await?;
        server::snapshot(active, restorable).map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

/// Ingest one typed, replayable update from the local PTY/Hive/telemetry fan-in.
/// The source domain remains the durable owner; this updates only the live Office
/// projection and intentionally does not write PostgreSQL on every sample.
#[post("/api/office/live/update")]
pub(crate) async fn office_live_update(
    update: OfficeLiveUpdate,
) -> Result<OfficeSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _serial = PERSISTENCE_SERIAL.lock().await;
        let repository = super::persistence_repository().await?;
        ensure_hydrated(&repository).await?;
        server::live_update(update).map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = update;
        Err(safe_error())
    }
}

#[cfg(feature = "server")]
pub(crate) fn office_agent_activity(
    agent_id: &str,
    status: md_web_contracts::domains::pty_agents::AgentStatus,
    action: &str,
    last_prompt: Option<&str>,
    has_terminal_draft: bool,
) -> Result<(), ServerFnError> {
    use md_web_contracts::domains::office_ui::AgentStatus as OfficeStatus;
    use md_web_contracts::domains::pty_agents::AgentStatus as ProcessStatus;
    let status = match status {
        ProcessStatus::Starting => OfficeStatus::Thinking,
        ProcessStatus::Idle => OfficeStatus::Idle,
        ProcessStatus::Working => OfficeStatus::Working,
        ProcessStatus::Waiting => OfficeStatus::Waiting,
        ProcessStatus::Blocked => OfficeStatus::Blocked,
        ProcessStatus::Looping => OfficeStatus::Looping,
        ProcessStatus::Exited | ProcessStatus::Archived | ProcessStatus::Restorable => {
            OfficeStatus::Ghost
        }
    };
    server::agent_activity(agent_id, status, action, last_prompt, has_terminal_draft)
        .map_err(|_| safe_error())
}

/// Revision cursor poll used when the shared shell has no streaming transport.
/// `None` means unchanged; any selection event increments the same revision.
#[get("/api/office/live/poll")]
pub(crate) async fn office_live_poll(
    since_revision: Option<u64>,
) -> Result<Option<OfficeSnapshot>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _serial = PERSISTENCE_SERIAL.lock().await;
        let repository = super::persistence_repository().await?;
        ensure_hydrated(&repository).await?;
        server::live_poll(since_revision).map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = since_revision;
        Err(safe_error())
    }
}

#[post("/api/office/spawn")]
pub(crate) async fn office_spawn(
    request: OfficeAgentSpawnRequest,
) -> Result<SpawnAgentResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _serial = PERSISTENCE_SERIAL.lock().await;
        let repository = super::persistence_repository().await?;
        ensure_hydrated(&repository).await?;
        let mut request = request;
        server::prepare_spawn(&mut request);
        let result = super::pty::pty_spawn(request.process.clone()).await?;
        let result = server::record_spawn(&request, result).map_err(|_| safe_error())?;
        persist_projection(&repository).await?;
        Ok(result)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/office/agents/close")]
pub(crate) async fn office_close_agent(agent_id: String) -> Result<(), ServerFnError> {
    let (active, _) = super::pty::list_agents().await?;
    let pty_id = active
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .and_then(|agent| agent.pty_id)
        .ok_or_else(safe_error)?;
    super::pty::pty_kill(pty_id).await
}

#[post("/api/office/agents/restore-all")]
pub(crate) async fn office_restore_all() -> Result<(), ServerFnError> {
    let (_, restorable) = super::pty::list_agents().await?;
    for agent in restorable {
        super::pty::pty_restore(md_web_contracts::domains::pty_agents::RestoreAgentRequest {
            agent,
            prefer_worktree: true,
        })
        .await?;
    }
    Ok(())
}

#[post("/api/office/agents/dismiss-restore")]
pub(crate) async fn office_dismiss_restore(agent_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::dismiss_restorable(agent_id)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = agent_id;
        Err(safe_error())
    }
}

#[post("/api/office/select")]
pub(crate) async fn office_select(agent_id: Option<String>) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::select(agent_id)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = agent_id;
        Err(safe_error())
    }
}

#[post("/api/office/reorder")]
pub(crate) async fn office_reorder(from_id: String, to_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::reorder(from_id, to_id)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (from_id, to_id);
        Err(safe_error())
    }
}

#[post("/api/office/rename")]
pub(crate) async fn office_rename(agent_id: String, name: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::rename(agent_id, name)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (agent_id, name);
        Err(safe_error())
    }
}

#[post("/api/office/note")]
pub(crate) async fn office_note(agent_id: String, note: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::note(agent_id, note)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (agent_id, note);
        Err(safe_error())
    }
}

#[post("/api/office/theme")]
pub(crate) async fn office_theme(theme: OfficeTheme) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::theme(theme)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = theme;
        Err(safe_error())
    }
}

#[post("/api/office/theme-preference")]
pub(crate) async fn office_theme_preference(
    preference: ThemePreference,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::theme_preference(preference)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = preference;
        Err(safe_error())
    }
}

#[post("/api/office/pause")]
pub(crate) async fn office_pause(paused: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::pause(paused)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = paused;
        Err(safe_error())
    }
}

#[post("/api/office/focus")]
pub(crate) async fn office_focus(focused: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::focus(focused)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = focused;
        Err(safe_error())
    }
}

#[post("/api/office/auto-mode")]
pub(crate) async fn office_auto_mode(enabled: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::auto_mode(enabled)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = enabled;
        Err(safe_error())
    }
}

#[post("/api/office/toast")]
pub(crate) async fn office_toast(notice: CompletionNotice) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::push_notice(notice)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = notice;
        Err(safe_error())
    }
}

#[post("/api/office/toast/dismiss")]
pub(crate) async fn office_dismiss_toast(
    correlation_id: String,
    completed_at_ms: i64,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        mutate(move || server::dismiss_notice(&correlation_id, completed_at_ms)).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (correlation_id, completed_at_ms);
        Err(safe_error())
    }
}

#[cfg(feature = "server")]
async fn mutate(operation: impl FnOnce() -> Result<(), ()>) -> Result<(), ServerFnError> {
    let _serial = PERSISTENCE_SERIAL.lock().await;
    let repository = super::persistence_repository().await?;
    ensure_hydrated(&repository).await?;
    operation().map_err(|_| safe_error())?;
    persist_projection(&repository).await
}
