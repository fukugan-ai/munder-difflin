use md_web_contracts::{
    AgentControlSnapshot, HiveAgent, HiveMessage, HiveTask, PreservedWorktreeSnapshot, TaskStatus,
    WorkerSnapshot,
};
use serde_json::Value;

/// Browser projection for all hive coordination tabs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HiveTasksViewModel {
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
    pub loading: bool,
    pub error: Option<String>,
}

/// User intent emitted by the task and ASK ME views.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TaskAction {
    Create {
        title: String,
        description: Option<String>,
        assignee: Option<String>,
        priority: i32,
    },
    Move {
        task_id: String,
        status: TaskStatus,
    },
    Delete {
        task_id: String,
    },
    Answer {
        task_id: String,
        answer: String,
    },
    DismissQuestion {
        task_id: String,
    },
}

/// User intent emitted by the agent control view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlAction {
    PatchRole { agent_id: String, role: String },
    SetHold { agent_id: String, on: bool },
    Pause { agent_id: String, on: bool },
    AutoDelivery { agent_id: String, paused: bool },
    Resume { agent_id: String },
    Steer { agent_id: String, text: String },
    Halt { agent_id: String },
}

/// User intent emitted by the thread surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessageAction {
    Reply { conversation: String, body: String },
    NewThread { subject: String, body: String },
}

#[cfg(test)]
mod tests {
    use md_web_contracts::TaskStatus;

    use super::TaskAction;

    #[test]
    fn move_action_keeps_target_status() {
        let action = TaskAction::Move {
            task_id: String::from("t-1"),
            status: TaskStatus::Done,
        };

        assert!(matches!(
            action,
            TaskAction::Move {
                status: TaskStatus::Done,
                ..
            }
        ));
    }
}
