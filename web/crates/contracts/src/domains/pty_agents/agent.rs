use serde::{Deserialize, Serialize};

/// Supported local coding-agent command providers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    Claude,
    Codex,
    Grok,
    Kimi,
    Gemini,
    Antigravity,
    Qwen,
    OpenCode,
    Crush,
    Pi,
    Copilot,
    Cursor,
    Custom,
}

/// Durable role flags attached to an agent.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentRole {
    pub orchestrator: bool,
    pub assistant: bool,
}

/// User-visible lifecycle state for an agent.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Idle,
    Working,
    Waiting,
    Blocked,
    Looping,
    Exited,
    Archived,
    Restorable,
}

/// Browser-safe agent record. Secrets and child-process environment are never included.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentRecord {
    pub id: String,
    pub name: String,
    pub provider: AgentProvider,
    pub role: AgentRole,
    pub description: String,
    pub cwd: String,
    pub command: String,
    pub args: Vec<String>,
    pub model: Option<String>,
    pub status: AgentStatus,
    pub action_ja: String,
    pub pty_id: Option<String>,
    pub worktree_path: Option<String>,
    pub session_id: Option<String>,
    pub archived: bool,
}

/// Request to create a local process and optionally provision it as a hive agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpawnAgentRequest {
    pub id: String,
    pub name: String,
    pub provider: AgentProvider,
    pub role: AgentRole,
    pub description: String,
    pub cwd: String,
    pub command: String,
    pub args: Vec<String>,
    pub model: Option<String>,
    pub cols: u16,
    pub rows: u16,
    pub isolate: bool,
    pub resume: bool,
    pub require_resume: bool,
    pub resume_session_id: Option<String>,
}

/// Successful spawn receipt returned to the browser.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpawnAgentResult {
    pub pty_id: String,
    pub cwd: String,
    pub worktree_path: Option<String>,
    pub resumed: bool,
    pub resume_not_found: bool,
    pub seed_prompt: Option<String>,
    /// True only when this process generation has an active tool-boundary hook bridge.
    #[serde(default)]
    pub hook_supported: bool,
}

/// Kill-and-spawn request that preserves the durable agent identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestartAgentRequest {
    pub agent_id: String,
    pub provider: AgentProvider,
    pub model: Option<String>,
    pub resume: bool,
    pub require_resume: bool,
}

/// Request to re-enter the saved process recipe after a server restart.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RestoreAgentRequest {
    pub agent: AgentRecord,
    pub prefer_worktree: bool,
}

#[cfg(test)]
mod tests {
    use super::{AgentProvider, AgentRole, AgentStatus, SpawnAgentRequest};

    #[test]
    fn spawn_request_preserves_maximum_terminal_dimensions() {
        let request = SpawnAgentRequest {
            id: String::from("dev-1"),
            name: String::from("Dev 1"),
            provider: AgentProvider::Codex,
            role: AgentRole::default(),
            description: String::new(),
            cwd: String::from("/workspace"),
            command: String::from("codex"),
            args: Vec::new(),
            model: None,
            cols: u16::MAX,
            rows: u16::MAX,
            isolate: false,
            resume: false,
            require_resume: false,
            resume_session_id: None,
        };

        assert_eq!((request.cols, request.rows), (u16::MAX, u16::MAX));
    }

    #[test]
    fn archived_status_is_distinct_from_exited() {
        assert_ne!(AgentStatus::Archived, AgentStatus::Exited);
    }

    #[test]
    fn browser_spawn_contract_has_no_hook_or_server_secret_fields() -> Result<(), serde_json::Error>
    {
        let request = SpawnAgentRequest {
            id: String::from("dev-1"),
            name: String::from("Dev 1"),
            provider: AgentProvider::Codex,
            role: AgentRole::default(),
            description: String::new(),
            cwd: String::from("/workspace"),
            command: String::from("codex"),
            args: Vec::new(),
            model: None,
            cols: 80,
            rows: 24,
            isolate: false,
            resume: false,
            require_resume: false,
            resume_session_id: None,
        };

        let json = serde_json::to_string(&request)?;
        assert!(!json.contains("capability"));
        assert!(!json.contains("hook_url"));
        assert!(!json.contains("MD_PG_"));
        Ok(())
    }
}
