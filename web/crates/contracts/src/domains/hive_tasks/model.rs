use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The intent carried by one hive message.
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

impl MessageAct {
    /// Whether the protocol expects a response for this act by default.
    #[must_use]
    pub const fn requires_reply(self) -> bool {
        matches!(self, Self::Request | Self::Propose | Self::Query)
    }
}

/// A normalized message routed through the local hive.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HiveMessage {
    pub id: String,
    pub conversation: String,
    pub in_reply_to: Option<String>,
    pub from: String,
    pub to: String,
    pub act: MessageAct,
    pub subject: String,
    pub body: String,
    pub hops: u8,
    pub requires_reply: bool,
    pub needs_human: bool,
    pub created_at: String,
}

/// One human question and its durable decision trail.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HumanQa {
    pub q: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asked_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dismissed_at: Option<String>,
}

impl HumanQa {
    /// True while the question has neither an answer nor a dismissal marker.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.a.as_ref().is_none_or(String::is_empty) && self.dismissed_at.is_none()
    }
}

/// Kanban column persisted by the hive task ledger.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Todo,
    Doing,
    Blocked,
    Done,
}

/// A lossless task card. Unknown hand-written fields remain in `extra`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiveTask {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub status: TaskStatus,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub priority: i32,
    pub created_at: String,
    #[serde(
        default,
        rename = "humanQA",
        alias = "humanQa",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub human_qa: Vec<HumanQa>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl HiveTask {
    /// Returns the newest unresolved human question.
    #[must_use]
    pub fn open_question(&self) -> Option<&HumanQa> {
        self.human_qa.iter().rev().find(|entry| entry.is_open())
    }

    /// True only for blocked work with an unresolved human question.
    #[must_use]
    pub fn waits_on_human(&self) -> bool {
        self.status == TaskStatus::Blocked && self.open_question().is_some()
    }
}

/// Top-level shape of `hive/tasks.json`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TaskLedger {
    #[serde(default)]
    pub tasks: Vec<HiveTask>,
}

/// Renderer-safe agent metadata used by coordination views.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiveAgent {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub on_hold: bool,
    #[serde(default)]
    pub inbox_backlog: usize,
}

/// Current hive roster.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiveRegistry {
    pub god_id: Option<String>,
    #[serde(default)]
    pub agents: BTreeMap<String, HiveAgent>,
}

/// Per-agent operator controls consumed at hook boundaries.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentControlSnapshot {
    pub paused: bool,
    pub halted: bool,
    pub auto_delivery_paused: bool,
    pub gated_tools: Vec<String>,
    pub pending_steers: usize,
}

/// Circuit-breaker level shown on the floor.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakerLevel {
    Healthy,
    Steering,
    Constrained,
    Stopped,
}

/// Latest breaker decision for one agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakerState {
    pub agent_id: String,
    pub level: BreakerLevel,
    pub reason: String,
    pub ts_ms: i64,
}

/// Lifecycle state of an ephemeral worker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Working,
    Releasing,
}

/// One live ephemeral worker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSnapshot {
    pub worker_id: String,
    pub request_id: String,
    pub name: String,
    pub base_branch: String,
    pub spawned_at: i64,
    pub age_ms: u64,
    pub idle_ms: Option<u64>,
    pub tokens_used: u64,
    pub token_cap: Option<u64>,
    pub has_slack: bool,
    pub status: WorkerStatus,
}

/// Worktree kept after worker exit because integration is not yet proven.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreservedWorktreeSnapshot {
    pub worker_id: String,
    pub worktree_path: String,
    pub base_branch: String,
    pub preserved_at: i64,
}

/// One renderer-safe snapshot for every Hive coordination tab.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiveSnapshot {
    pub tasks: Vec<HiveTask>,
    pub agents: Vec<HiveAgent>,
    pub messages: Vec<HiveMessage>,
    pub selected_agent_id: Option<String>,
    pub selected_control: AgentControlSnapshot,
    pub workers: Vec<WorkerSnapshot>,
    pub preserved_worktrees: Vec<PreservedWorktreeSnapshot>,
    pub max_workers: usize,
    pub board: String,
    pub log_tail: Vec<Value>,
    pub selected_memory: Option<String>,
}

/// Control decision consumed at the actual PTY/tool hook boundary.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HiveHookDecision {
    pub paused: bool,
    pub halted: bool,
    pub auto_delivery_paused: bool,
    pub tool_gated: bool,
    pub steer: Option<String>,
}

/// Durable projection receipt returned after a worker PTY has stopped.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerTeardownReceipt {
    pub worker_id: String,
    pub pty_stopped: bool,
    pub worktree_preserved: bool,
    pub preserved_path: Option<String>,
    pub completed_at: i64,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::Value;

    use super::{HiveTask, HumanQa, MessageAct, TaskStatus};

    fn task(status: TaskStatus, human_qa: Vec<HumanQa>) -> HiveTask {
        HiveTask {
            id: String::from("t-1"),
            title: String::from("test"),
            description: None,
            assignee: None,
            status,
            depends_on: Vec::new(),
            priority: 1,
            created_at: String::from("2026-08-25T00:00:00Z"),
            human_qa,
            result: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn query_requires_reply() {
        assert!(MessageAct::Query.requires_reply());
    }

    #[test]
    fn inform_does_not_require_reply() {
        assert!(!MessageAct::Inform.requires_reply());
    }

    #[test]
    fn dismissed_question_is_closed() {
        let qa = HumanQa {
            q: String::from("continue?"),
            a: None,
            asked_at: None,
            answered_at: None,
            dismissed_at: Some(String::from("2026-08-25T01:00:00Z")),
        };

        assert!(!qa.is_open());
    }

    #[test]
    fn blocked_task_with_open_question_waits_on_human() {
        let qa = HumanQa {
            q: String::from("continue?"),
            a: None,
            asked_at: None,
            answered_at: None,
            dismissed_at: None,
        };

        assert!(task(TaskStatus::Blocked, vec![qa]).waits_on_human());
    }

    #[test]
    fn todo_task_does_not_wait_on_human() {
        let qa = HumanQa {
            q: String::from("continue?"),
            a: None,
            asked_at: None,
            answered_at: None,
            dismissed_at: None,
        };

        assert!(!task(TaskStatus::Todo, vec![qa]).waits_on_human());
    }

    #[test]
    fn task_uses_protocol_human_qa_key() -> Result<(), serde_json::Error> {
        let qa = HumanQa {
            q: String::from("continue?"),
            a: None,
            asked_at: None,
            answered_at: None,
            dismissed_at: None,
        };
        let value = serde_json::to_value(task(TaskStatus::Blocked, vec![qa]))?;
        let object = value.as_object().cloned().unwrap_or_default();

        assert!(object.contains_key("humanQA"));
        assert!(!object.contains_key("humanQa"));
        let mut legacy = object;
        let entries = legacy.remove("humanQA").unwrap_or(Value::Array(Vec::new()));
        legacy.insert(String::from("humanQa"), entries);
        let decoded: HiveTask = serde_json::from_value(Value::Object(legacy))?;
        assert_eq!(decoded.human_qa.len(), 1);
        Ok(())
    }
}
