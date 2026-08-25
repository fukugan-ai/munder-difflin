use serde::{Deserialize, Serialize};

use crate::domains::pty_agents::AgentProvider;
use crate::domains::pty_agents::SpawnAgentRequest;

/// Color identity shared by the roster card and floor avatar.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    Coral,
    Mint,
    Sky,
    #[default]
    Lemon,
    Lilac,
    Peach,
}

/// Visual lifecycle state of an agent.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    #[default]
    Idle,
    Thinking,
    Working,
    Waiting,
    Blocked,
    Success,
    Ghost,
    Compacting,
    Looping,
}

/// Licensed pixel-character identities supported by the existing office art.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficeCharacter {
    Michael,
    Dwight,
    Pam,
    #[default]
    Jim,
    Stanley,
    Phyllis,
    Angela,
    Kevin,
    Oscar,
    Meredith,
    Creed,
    Andy,
    Ryan,
    Kelly,
    Toby,
    Darryl,
}

/// Swappable floor-map identifier.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OfficeTheme {
    #[default]
    Office,
    Brooklyn99,
    Friends,
    SiliconValley,
    Got,
    Hogwarts,
}

/// User-facing theme preference. System resolves in the browser shell.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Dark,
    Light,
}

/// Minimal avatar/card projection. Secrets and filesystem data never enter it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeAgent {
    pub id: String,
    pub name: String,
    pub character: OfficeCharacter,
    pub accent: Accent,
    pub status: AgentStatus,
    pub project: String,
    pub action: String,
    pub note: String,
    pub last_prompt: String,
    pub carrying: Option<String>,
    pub progress_eighths: u8,
    pub context_tokens: Option<u64>,
    pub context_limit: Option<u64>,
    pub has_terminal_draft: bool,
    pub is_god: bool,
}

/// Task state required by the wall boards and roster sticky notes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Todo,
    Doing,
    Blocked,
    Done,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeTask {
    pub id: String,
    pub status: TaskStatus,
    pub assignee: Option<String>,
    pub has_unanswered_human_qa: bool,
}

/// Complete live agent state supplied by the PTY/event adapter. Complete fields
/// avoid ambiguous nested-option patch semantics when a value must be cleared.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeAgentLiveState {
    pub agent_id: String,
    pub status: AgentStatus,
    pub action: String,
    pub last_prompt: String,
    pub progress_eighths: u8,
    pub context_tokens: Option<u64>,
    pub context_limit: Option<u64>,
    pub carrying: Option<String>,
    pub has_terminal_draft: bool,
}

/// Integer-valued office telemetry projection. The durable cost ledger remains
/// authoritative; this DTO is safe for deterministic UI equality and polling.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeAgentTelemetry {
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd_micros: u64,
    pub last_tool: Option<String>,
    pub last_tool_duration_ms: Option<u64>,
    pub observed_at_ms: i64,
}

/// Restorable roster entry; process recipes remain server-side.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestorableAgent {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Complete DTO snapshot consumed by the Dioxus UI and Pixi island.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeSnapshot {
    pub revision: u64,
    pub theme: OfficeTheme,
    pub theme_preference: ThemePreference,
    pub selected_agent_id: Option<String>,
    pub paused: bool,
    pub agents: Vec<OfficeAgent>,
    pub tasks: Vec<OfficeTask>,
    pub restorable_agents: Vec<RestorableAgent>,
    #[serde(default)]
    pub handoffs: Vec<HiveHandoff>,
    #[serde(default)]
    pub telemetry: Vec<OfficeAgentTelemetry>,
}

/// Lossless add-agent dialog payload: process authority stays in the PTY
/// request while office-only visual and briefing fields remain explicit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeAgentSpawnRequest {
    pub process: SpawnAgentRequest,
    pub character: OfficeCharacter,
    pub accent: Accent,
    pub project: String,
    pub goal: String,
}

/// Browser-imported, review-only hire manifest. It cannot carry an executable;
/// the modal resolves the provider to a local command before the human spawns.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OfficeHireManifest {
    pub spec: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    #[serde(default)]
    pub character: Option<String>,
    #[serde(default)]
    pub accent: Option<String>,
    #[serde(default)]
    pub provider: Option<AgentProvider>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub command_flags: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub isolate: bool,
    #[serde(default)]
    pub token_cap: Option<u64>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
}

/// Complete response consumed by the office route; it is produced from the
/// live agent registry plus the server-owned office projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OfficeUiState {
    pub snapshot: OfficeSnapshot,
    pub notices: Vec<CompletionNotice>,
    pub focus_mode: bool,
    pub auto_mode: bool,
    #[serde(default)]
    pub dismissed_restorable_agent_ids: Vec<String>,
}

/// Hive speech act rendered by an animated envelope.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageAct {
    Request,
    Inform,
    Propose,
    Query,
    Agree,
    Refuse,
    Done,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HiveHandoff {
    pub event_id: String,
    pub sequence: u64,
    pub from: String,
    pub targets: Vec<String>,
    pub act: MessageAct,
    pub needs_human: bool,
}

/// Typed fan-in seam for PTY, Hive, memory/cost and selection event owners.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum OfficeLiveUpdate {
    AgentState(OfficeAgentLiveState),
    ReplaceTasks { tasks: Vec<OfficeTask> },
    Handoff(HiveHandoff),
    Telemetry(OfficeAgentTelemetry),
    SelectAgent { agent_id: Option<String> },
}

/// Completion notice rendered in the global toast stack.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompletionNotice {
    pub correlation_id: String,
    pub kind: String,
    pub target_agent_id: String,
    pub task_id: Option<String>,
    pub summary: String,
    pub completed_at_ms: i64,
    pub objective: Option<String>,
}

/// Actions emitted from the browser-only floor island.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum OfficeUiAction {
    SelectAgent { agent_id: String },
    OpenTasks,
    OpenHumanQuestions,
    RequestClose,
}

#[cfg(test)]
mod tests {
    use super::{
        Accent, AgentStatus, OfficeAgent, OfficeCharacter, OfficeSnapshot, OfficeTheme,
        OfficeUiAction, ThemePreference,
    };

    fn sample_agent(progress_eighths: u8) -> OfficeAgent {
        OfficeAgent {
            id: String::from("god"),
            name: String::from("Aria"),
            character: OfficeCharacter::Michael,
            accent: Accent::Lemon,
            status: AgentStatus::Working,
            project: String::from("munder-difflin"),
            action: String::from("coordinating"),
            note: String::new(),
            last_prompt: String::new(),
            carrying: None,
            progress_eighths,
            context_tokens: Some(10),
            context_limit: Some(100),
            has_terminal_draft: false,
            is_god: true,
        }
    }

    #[test]
    fn empty_snapshot_has_office_defaults() {
        let snapshot = OfficeSnapshot::default();

        assert_eq!(snapshot.theme, OfficeTheme::Office);
    }

    #[test]
    fn snapshot_accepts_maximum_revision() {
        let snapshot = OfficeSnapshot {
            revision: u64::MAX,
            theme_preference: ThemePreference::Dark,
            agents: vec![sample_agent(8)],
            ..OfficeSnapshot::default()
        };

        assert_eq!(snapshot.revision, u64::MAX);
    }

    #[test]
    fn agent_progress_preserves_boundary_for_projection_validation() {
        let agent = sample_agent(u8::MAX);

        assert_eq!(agent.progress_eighths, u8::MAX);
    }

    #[test]
    fn island_actions_deserialize_from_custom_event_dtos() {
        let Ok(select) = serde_json::from_str::<OfficeUiAction>(
            r#"{"type":"select_agent","data":{"agent_id":"andy"}}"#,
        ) else {
            panic!("select action should deserialize");
        };
        let Ok(tasks) = serde_json::from_str::<OfficeUiAction>(r#"{"type":"open_tasks"}"#) else {
            panic!("task action should deserialize");
        };
        let Ok(human) =
            serde_json::from_str::<OfficeUiAction>(r#"{"type":"open_human_questions"}"#)
        else {
            panic!("human action should deserialize");
        };
        let Ok(close) = serde_json::from_str::<OfficeUiAction>(r#"{"type":"request_close"}"#)
        else {
            panic!("close action should deserialize");
        };

        assert_eq!(
            select,
            OfficeUiAction::SelectAgent {
                agent_id: String::from("andy")
            }
        );
        assert_eq!(tasks, OfficeUiAction::OpenTasks);
        assert_eq!(human, OfficeUiAction::OpenHumanQuestions);
        assert_eq!(close, OfficeUiAction::RequestClose);
    }
}
