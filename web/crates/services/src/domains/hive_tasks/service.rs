use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use md_web_contracts::domains::hive_tasks::{
    HiveHookDecision, HiveSnapshot, WorkerTeardownReceipt,
};
use md_web_contracts::domains::pty_agents::AgentHookEvent;
use md_web_contracts::{
    AgentControlSnapshot, HiveDomainEvent, HiveMessage, HiveTask, MessageAct,
    PreservedWorktreeSnapshot, TaskStatus, WorkerSnapshot,
};
use serde_json::{Map, Value};

use super::{
    ControlError, ControlRegistry, EventHub, EventHubError, HiveRouter, HiveStore, HiveStoreError,
    ReplayBatch, RouteError, RouteOutcome, WorkerRegistry, WorkerRegistryError,
};

/// Failure from the process-lifetime Hive application adapter.
#[derive(Debug)]
pub enum HiveServiceError {
    InvalidInput,
    StateUnavailable,
    Store(HiveStoreError),
    Event(EventHubError),
    Route(RouteError),
    Control(ControlError),
    Worker(WorkerRegistryError),
    Json(serde_json::Error),
}

impl From<HiveStoreError> for HiveServiceError {
    fn from(error: HiveStoreError) -> Self {
        Self::Store(error)
    }
}

impl From<EventHubError> for HiveServiceError {
    fn from(error: EventHubError) -> Self {
        Self::Event(error)
    }
}

impl From<RouteError> for HiveServiceError {
    fn from(error: RouteError) -> Self {
        Self::Route(error)
    }
}

impl From<ControlError> for HiveServiceError {
    fn from(error: ControlError) -> Self {
        Self::Control(error)
    }
}

impl From<WorkerRegistryError> for HiveServiceError {
    fn from(error: WorkerRegistryError) -> Self {
        Self::Worker(error)
    }
}

impl From<serde_json::Error> for HiveServiceError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Callable boundary shared by Dioxus server functions and hook adapters.
pub struct HiveTasksService {
    store: Arc<HiveStore>,
    events: Arc<EventHub>,
    router: HiveRouter,
    controls: ControlRegistry,
    workers: WorkerRegistry,
    next_message_id: AtomicU64,
    next_task_id: AtomicU64,
    hook_replay: Mutex<HookReplay>,
}

struct HookReplay {
    capacity: usize,
    order: VecDeque<String>,
    decisions: BTreeMap<String, HiveHookDecision>,
}

impl HiveTasksService {
    /// Creates one process-lifetime service over the server-resolved Hive root.
    pub fn new(
        root: PathBuf,
        event_capacity: usize,
        max_workers: usize,
    ) -> Result<Self, HiveServiceError> {
        let store = Arc::new(HiveStore::new(root)?);
        let events = Arc::new(EventHub::new(event_capacity)?);
        Ok(Self {
            router: HiveRouter::new(Arc::clone(&store), Arc::clone(&events)),
            store,
            events,
            controls: ControlRegistry::default(),
            workers: WorkerRegistry::new(max_workers)?,
            next_message_id: AtomicU64::new(1),
            next_task_id: AtomicU64::new(1),
            hook_replay: Mutex::new(HookReplay {
                capacity: event_capacity,
                order: VecDeque::with_capacity(event_capacity),
                decisions: BTreeMap::new(),
            }),
        })
    }

    /// Returns every renderer-safe surface in a single consistent refresh call.
    pub fn snapshot(
        &self,
        selected_agent_id: Option<&str>,
        message_limit: usize,
    ) -> Result<HiveSnapshot, HiveServiceError> {
        let registry = self.store.registry()?;
        let mut agents: Vec<_> = registry.agents.into_values().collect();
        for agent in &mut agents {
            agent.inbox_backlog = self.store.inbox(&agent.id)?.len();
        }
        let selected_agent_id = selected_agent_id
            .filter(|id| {
                agents
                    .iter()
                    .any(|agent| agent.id == *id && !agent.archived)
            })
            .map(String::from)
            .or_else(|| {
                agents
                    .iter()
                    .find(|agent| !agent.archived)
                    .map(|agent| agent.id.clone())
            });
        let selected_control = selected_agent_id
            .as_deref()
            .map(|id| self.controls.snapshot(id))
            .transpose()?
            .unwrap_or_default();
        let selected_memory = selected_agent_id
            .as_deref()
            .map(|id| self.store.memory(id))
            .transpose()?;
        let (workers, preserved_worktrees, max_workers) = self.workers.snapshot()?;
        Ok(HiveSnapshot {
            tasks: self.store.tasks()?.tasks,
            agents,
            messages: self.store.messages(message_limit)?,
            selected_agent_id,
            selected_control,
            workers,
            preserved_worktrees,
            max_workers,
            board: self.store.board()?,
            log_tail: self.store.log_tail(200)?,
            selected_memory,
        })
    }

    /// Creates an operator-authored task without requiring the browser to mint identity fields.
    pub fn create_task(
        &self,
        title: &str,
        description: Option<String>,
        assignee: Option<String>,
        priority: i32,
        created_at: &str,
        ts_ms: i64,
    ) -> Result<HiveTask, HiveServiceError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        let sequence = self
            .next_task_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| HiveServiceError::InvalidInput)?;
        self.add_task(
            HiveTask {
                id: format!("web-task-{ts_ms}-{sequence}"),
                title: String::from(title),
                description: description.filter(|value| !value.trim().is_empty()),
                assignee: assignee.filter(|value| !value.trim().is_empty()),
                status: TaskStatus::Todo,
                depends_on: Vec::new(),
                priority,
                created_at: String::from(created_at),
                human_qa: Vec::new(),
                result: None,
                extra: Default::default(),
            },
            ts_ms,
        )
    }

    pub fn inbox(&self, agent_id: &str) -> Result<Vec<HiveMessage>, HiveServiceError> {
        self.store.inbox(agent_id).map_err(Into::into)
    }

    pub fn add_task(&self, task: HiveTask, ts_ms: i64) -> Result<HiveTask, HiveServiceError> {
        if task.id.trim().is_empty() || task.title.trim().is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        self.store.add_task(&task)?;
        self.events
            .publish(ts_ms, HiveDomainEvent::TaskAdded(task.clone()))?;
        Ok(task)
    }

    pub fn patch_task(
        &self,
        task_id: &str,
        patch: &Map<String, Value>,
        ts_ms: i64,
    ) -> Result<HiveTask, HiveServiceError> {
        if task_id.trim().is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        let task = self.store.patch_task(task_id, patch)?;
        self.events
            .publish(ts_ms, HiveDomainEvent::TaskPatched(task.clone()))?;
        Ok(task)
    }

    pub fn move_task(
        &self,
        task_id: &str,
        status: TaskStatus,
        ts_ms: i64,
    ) -> Result<HiveTask, HiveServiceError> {
        let patch = Map::from_iter([(String::from("status"), serde_json::to_value(status)?)]);
        self.patch_task(task_id, &patch, ts_ms)
    }

    pub fn delete_task(&self, task_id: &str, ts_ms: i64) -> Result<(), HiveServiceError> {
        if task_id.trim().is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        self.store.delete_task(task_id)?;
        self.events.publish(
            ts_ms,
            HiveDomainEvent::TaskDeleted {
                task_id: String::from(task_id),
            },
        )?;
        Ok(())
    }

    /// Records the newest open answer losslessly, then informs the god inbox.
    pub fn answer_question(
        &self,
        task_id: &str,
        answer: &str,
        answered_at: &str,
        ts_ms: i64,
    ) -> Result<HiveTask, HiveServiceError> {
        let answer = answer.trim();
        if answer.is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        let mut task = self.task(task_id)?;
        let question = task
            .human_qa
            .iter_mut()
            .rev()
            .find(|entry| entry.is_open())
            .ok_or(HiveServiceError::InvalidInput)?;
        question.a = Some(String::from(answer));
        question.answered_at = Some(String::from(answered_at));
        let patch = Map::from_iter([(
            String::from("humanQA"),
            serde_json::to_value(&task.human_qa)?,
        )]);
        let updated = self.patch_task(task_id, &patch, ts_ms)?;
        let message = self.operator_message(
            task_id,
            &format!("回答: {}", task.title),
            &format!(
                "人間から回答が届きました: {answer}\n\n回答はタスクのhumanQAにも記録済みです。カードを解除して作業を続けてください。"
            ),
            answered_at,
            ts_ms,
        )?;
        self.router.route(&message, ts_ms)?;
        Ok(updated)
    }

    /// Closes only the newest open question and deliberately leaves the card blocked.
    pub fn dismiss_question(
        &self,
        task_id: &str,
        dismissed_at: &str,
        ts_ms: i64,
    ) -> Result<HiveTask, HiveServiceError> {
        let mut task = self.task(task_id)?;
        let question = task
            .human_qa
            .iter_mut()
            .rev()
            .find(|entry| entry.is_open())
            .ok_or(HiveServiceError::InvalidInput)?;
        question.dismissed_at = Some(String::from(dismissed_at));
        let patch = Map::from_iter([(
            String::from("humanQA"),
            serde_json::to_value(&task.human_qa)?,
        )]);
        self.patch_task(task_id, &patch, ts_ms)
    }

    pub fn send(
        &self,
        message: &HiveMessage,
        ts_ms: i64,
    ) -> Result<RouteOutcome, HiveServiceError> {
        self.router.route(message, ts_ms).map_err(Into::into)
    }

    /// Sends an operator reply into the god inbox without exposing filesystem paths.
    pub fn reply_to_god(
        &self,
        conversation: &str,
        body: &str,
        created_at: &str,
        ts_ms: i64,
    ) -> Result<RouteOutcome, HiveServiceError> {
        if conversation.trim().is_empty() || body.trim().is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        let message = self.operator_message(
            conversation,
            "人間からの返信",
            body.trim(),
            created_at,
            ts_ms,
        )?;
        self.send(&message, ts_ms)
    }

    pub fn new_thread(
        &self,
        subject: &str,
        body: &str,
        created_at: &str,
        ts_ms: i64,
    ) -> Result<RouteOutcome, HiveServiceError> {
        let subject = subject.trim();
        if subject.is_empty() || body.trim().is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        let message = self.operator_message(
            &format!("web-thread-{ts_ms}"),
            subject,
            body.trim(),
            created_at,
            ts_ms,
        )?;
        self.send(&message, ts_ms)
    }

    pub fn patch_role(&self, agent_id: &str, role: &str) -> Result<(), HiveServiceError> {
        let role = role.trim();
        if role.is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        self.store.patch_registry_agent(
            agent_id,
            &Map::from_iter([(String::from("role"), Value::String(String::from(role)))]),
        )?;
        Ok(())
    }

    pub fn set_hold(&self, agent_id: &str, on: bool) -> Result<(), HiveServiceError> {
        self.store.patch_registry_agent(
            agent_id,
            &Map::from_iter([(String::from("onHold"), Value::Bool(on))]),
        )?;
        Ok(())
    }

    pub fn pause(
        &self,
        agent_id: &str,
        on: bool,
        ts_ms: i64,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        let snapshot = self.controls.pause(agent_id, on)?;
        self.publish_control(agent_id, &snapshot, ts_ms)?;
        Ok(snapshot)
    }

    pub fn pause_auto_delivery(
        &self,
        agent_id: &str,
        on: bool,
        ts_ms: i64,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        let snapshot = self.controls.pause_auto_delivery(agent_id, on)?;
        self.publish_control(agent_id, &snapshot, ts_ms)?;
        Ok(snapshot)
    }

    /// Enables or disables one hook-consumed tool gate on the shared registry.
    pub fn gate_tool(
        &self,
        agent_id: &str,
        tool: &str,
        on: bool,
        ts_ms: i64,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        if agent_id.trim().is_empty() || tool.trim().is_empty() {
            return Err(HiveServiceError::InvalidInput);
        }
        let snapshot = self.controls.gate_tool(agent_id, tool, on)?;
        self.publish_control(agent_id, &snapshot, ts_ms)?;
        Ok(snapshot)
    }

    pub fn resume(
        &self,
        agent_id: &str,
        ts_ms: i64,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        let snapshot = self.controls.resume(agent_id)?;
        self.publish_control(agent_id, &snapshot, ts_ms)?;
        Ok(snapshot)
    }

    pub fn steer(
        &self,
        agent_id: &str,
        text: &str,
        ts_ms: i64,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        let snapshot = self.controls.steer(agent_id, text)?;
        self.publish_control(agent_id, &snapshot, ts_ms)?;
        Ok(snapshot)
    }

    pub fn halt(
        &self,
        agent_id: &str,
        ts_ms: i64,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        let snapshot = self.controls.halt(agent_id)?;
        self.publish_control(agent_id, &snapshot, ts_ms)?;
        Ok(snapshot)
    }

    pub fn control_snapshot(
        &self,
        agent_id: &str,
    ) -> Result<AgentControlSnapshot, HiveServiceError> {
        self.controls.snapshot(agent_id).map_err(Into::into)
    }

    pub fn take_steer(&self, agent_id: &str) -> Result<Option<String>, HiveServiceError> {
        self.controls.take_steer(agent_id).map_err(Into::into)
    }

    /// Consumes the shared control state at a real PTY/tool hook boundary.
    pub fn hook_decision(
        &self,
        agent_id: &str,
        tool: Option<&str>,
    ) -> Result<HiveHookDecision, HiveServiceError> {
        let snapshot = self.controls.snapshot(agent_id)?;
        let tool_gated =
            tool.is_some_and(|tool| snapshot.gated_tools.iter().any(|item| item == tool));
        let steer = if snapshot.paused || snapshot.halted || tool_gated {
            None
        } else {
            self.controls.take_steer(agent_id)?
        };
        Ok(HiveHookDecision {
            paused: snapshot.paused,
            halted: snapshot.halted,
            auto_delivery_paused: snapshot.auto_delivery_paused,
            tool_gated,
            steer,
        })
    }

    /// Records a verified typed hook once and replays the same one-shot decision on retry.
    /// Capability and payload are deliberately absent from this boundary.
    pub fn process_agent_hook(
        &self,
        agent_id: &str,
        event_id: &str,
        event: AgentHookEvent,
        tool_name: Option<&str>,
        ts_ms: i64,
    ) -> Result<HiveHookDecision, HiveServiceError> {
        let agent_id = agent_id.trim();
        let event_id = event_id.trim();
        let tool_name = tool_name.map(str::trim).filter(|tool| !tool.is_empty());
        if agent_id.is_empty()
            || agent_id.len() > 128
            || event_id.is_empty()
            || event_id.len() > 128
            || tool_name.is_some_and(|tool| tool.len() > 128)
        {
            return Err(HiveServiceError::InvalidInput);
        }
        let key = format!("{agent_id}\0{event_id}");
        let mut replay = self
            .hook_replay
            .lock()
            .map_err(|_| HiveServiceError::StateUnavailable)?;
        if let Some(decision) = replay.decisions.get(&key) {
            return Ok(decision.clone());
        }

        let snapshot = self.controls.snapshot(agent_id)?;
        let tool_gated =
            tool_name.is_some_and(|tool| snapshot.gated_tools.iter().any(|item| item == tool));
        self.events.publish(
            ts_ms,
            HiveDomainEvent::AgentHookObserved {
                agent_id: String::from(agent_id),
                event_id: String::from(event_id),
                event,
                tool_name: tool_name.map(String::from),
            },
        )?;
        let steer = if snapshot.paused || snapshot.halted || tool_gated {
            None
        } else {
            self.controls.take_steer(agent_id)?
        };
        let decision = HiveHookDecision {
            paused: snapshot.paused,
            halted: snapshot.halted,
            auto_delivery_paused: snapshot.auto_delivery_paused,
            tool_gated,
            steer,
        };
        let evicted = if replay.order.len() == replay.capacity {
            replay.order.pop_front()
        } else {
            None
        };
        if let Some(evicted) = evicted {
            replay.decisions.remove(&evicted);
        }
        replay.order.push_back(key.clone());
        replay.decisions.insert(key, decision.clone());
        Ok(decision)
    }

    pub fn register_worker(
        &self,
        worker: WorkerSnapshot,
        ts_ms: i64,
    ) -> Result<(), HiveServiceError> {
        self.workers.insert(worker.clone())?;
        self.events
            .publish(ts_ms, HiveDomainEvent::WorkerChanged(worker))?;
        Ok(())
    }

    pub fn workers_snapshot(
        &self,
    ) -> Result<(Vec<WorkerSnapshot>, Vec<PreservedWorktreeSnapshot>, usize), HiveServiceError>
    {
        self.workers.snapshot().map_err(Into::into)
    }

    pub fn stop_worker(
        &self,
        worker_id: &str,
        ts_ms: i64,
    ) -> Result<WorkerSnapshot, HiveServiceError> {
        let worker = self.workers.request_stop(worker_id)?;
        self.events
            .publish(ts_ms, HiveDomainEvent::WorkerChanged(worker.clone()))?;
        Ok(worker)
    }

    pub fn complete_worker_stop(
        &self,
        worker_id: &str,
        worktree_path: Option<String>,
        completed_at: i64,
    ) -> Result<WorkerTeardownReceipt, HiveServiceError> {
        let receipt = self
            .workers
            .complete_stop(worker_id, worktree_path, completed_at)
            .map_err(HiveServiceError::from)?;
        self.events.publish(
            completed_at,
            HiveDomainEvent::WorkerTeardown(receipt.clone()),
        )?;
        Ok(receipt)
    }

    pub fn replay_after(&self, after: u64) -> Result<ReplayBatch, HiveServiceError> {
        self.events.replay_after(after).map_err(Into::into)
    }

    fn task(&self, task_id: &str) -> Result<HiveTask, HiveServiceError> {
        self.store
            .tasks()?
            .tasks
            .into_iter()
            .find(|task| task.id == task_id)
            .ok_or(HiveStoreError::TaskNotFound.into())
    }

    fn operator_message(
        &self,
        conversation: &str,
        subject: &str,
        body: &str,
        created_at: &str,
        ts_ms: i64,
    ) -> Result<HiveMessage, HiveServiceError> {
        let sequence = self
            .next_message_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| HiveServiceError::InvalidInput)?;
        Ok(HiveMessage {
            id: format!("web-{ts_ms}-{sequence}"),
            conversation: String::from(conversation),
            in_reply_to: None,
            from: String::from("human"),
            to: String::from("god"),
            act: MessageAct::Inform,
            subject: String::from(subject),
            body: String::from(body),
            hops: 0,
            requires_reply: false,
            needs_human: false,
            created_at: String::from(created_at),
        })
    }

    fn publish_control(
        &self,
        agent_id: &str,
        snapshot: &AgentControlSnapshot,
        ts_ms: i64,
    ) -> Result<(), HiveServiceError> {
        self.events.publish(
            ts_ms,
            HiveDomainEvent::ControlChanged {
                agent_id: String::from(agent_id),
                snapshot: snapshot.clone(),
            },
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use md_web_contracts::domains::hive_tasks::{HiveDomainEvent, HumanQa};
    use md_web_contracts::domains::pty_agents::AgentHookEvent;
    use md_web_contracts::{HiveTask, TaskStatus, WorkerSnapshot, WorkerStatus};

    use super::{HiveServiceError, HiveStoreError, HiveTasksService};

    fn root(name: &str) -> Result<PathBuf, HiveStoreError> {
        let root = std::env::current_dir()?
            .join("target")
            .join("hive-service-tests")
            .join(name);
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join("agents/god/inbox"))?;
        fs::create_dir_all(root.join("agents/god/outbox"))?;
        fs::write(
            root.join("registry.json"),
            serde_json::to_vec(&serde_json::json!({
                "godId": "god",
                "agents": {
                    "god": {
                        "id": "god", "name": "Michael", "status": "idle",
                        "role": "orchestrator", "provider": "codex"
                    }
                }
            }))?,
        )?;
        Ok(root)
    }

    fn blocked_task() -> HiveTask {
        HiveTask {
            id: String::from("t-1"),
            title: String::from("Decision"),
            description: None,
            assignee: Some(String::from("god")),
            status: TaskStatus::Blocked,
            depends_on: Vec::new(),
            priority: 1,
            created_at: String::from("1"),
            human_qa: vec![HumanQa {
                q: String::from("Proceed?"),
                a: None,
                asked_at: Some(String::from("1")),
                answered_at: None,
                dismissed_at: None,
            }],
            result: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn answer_patches_latest_question_and_notifies_god() -> Result<(), HiveServiceError> {
        let service =
            HiveTasksService::new(root("answer").map_err(HiveServiceError::Store)?, 8, 1)?;
        service.add_task(blocked_task(), 1)?;

        let task = service.answer_question("t-1", "Yes", "2", 2)?;

        assert_eq!(task.human_qa[0].a.as_deref(), Some("Yes"));
        assert_eq!(service.inbox("god")?.len(), 1);
        assert_eq!(service.replay_after(0)?.events.len(), 3);
        Ok(())
    }

    #[test]
    fn dismissal_leaves_blocked_status_and_sends_no_message() -> Result<(), HiveServiceError> {
        let service =
            HiveTasksService::new(root("dismiss").map_err(HiveServiceError::Store)?, 8, 1)?;
        service.add_task(blocked_task(), 1)?;

        let task = service.dismiss_question("t-1", "2", 2)?;

        assert_eq!(task.status, TaskStatus::Blocked);
        assert_eq!(task.human_qa[0].dismissed_at.as_deref(), Some("2"));
        assert!(service.inbox("god")?.is_empty());
        Ok(())
    }

    #[test]
    fn snapshot_reads_legacy_registry_defaults() -> Result<(), HiveServiceError> {
        let service =
            HiveTasksService::new(root("snapshot").map_err(HiveServiceError::Store)?, 8, 2)?;

        let snapshot = service.snapshot(None, 20)?;

        assert_eq!(snapshot.selected_agent_id.as_deref(), Some("god"));
        assert_eq!(snapshot.max_workers, 2);
        assert_eq!(snapshot.agents[0].inbox_backlog, 0);
        Ok(())
    }

    #[test]
    fn gate_tool_uses_shared_control_state_and_survives_resume() -> Result<(), HiveServiceError> {
        let service =
            HiveTasksService::new(root("gate-tool").map_err(HiveServiceError::Store)?, 8, 1)?;

        let gated = service.gate_tool("god", "Bash", true, 1)?;
        service.pause("god", true, 2)?;
        let resumed = service.resume("god", 3)?;

        assert_eq!(gated.gated_tools, [String::from("Bash")]);
        assert_eq!(resumed.gated_tools, [String::from("Bash")]);
        assert_eq!(service.control_snapshot("god")?, resumed);
        assert_eq!(service.replay_after(0)?.events.len(), 3);
        Ok(())
    }

    #[test]
    fn hook_decision_enforces_pause_gate_halt_and_consumes_one_steer()
    -> Result<(), HiveServiceError> {
        let service = HiveTasksService::new(
            root("hook-decision").map_err(HiveServiceError::Store)?,
            16,
            1,
        )?;
        service.steer("god", "use the smaller patch", 1)?;
        let first = service.hook_decision("god", Some("Read"))?;
        let second = service.hook_decision("god", Some("Read"))?;
        service.gate_tool("god", "Bash", true, 2)?;
        let gated = service.hook_decision("god", Some("Bash"))?;
        service.halt("god", 3)?;
        let halted = service.hook_decision("god", None)?;

        assert_eq!(first.steer.as_deref(), Some("use the smaller patch"));
        assert!(second.steer.is_none());
        assert!(gated.tool_gated);
        assert!(halted.halted);
        Ok(())
    }

    #[test]
    fn worker_teardown_publishes_archive_receipt() -> Result<(), HiveServiceError> {
        let service = HiveTasksService::new(
            root("worker-teardown").map_err(HiveServiceError::Store)?,
            8,
            1,
        )?;
        service.register_worker(
            WorkerSnapshot {
                worker_id: String::from("worker-1"),
                request_id: String::from("request-1"),
                name: String::from("Worker 1"),
                base_branch: String::from("main"),
                spawned_at: 1,
                age_ms: 0,
                idle_ms: None,
                tokens_used: 0,
                token_cap: None,
                has_slack: false,
                status: WorkerStatus::Working,
            },
            1,
        )?;
        service.stop_worker("worker-1", 2)?;

        let receipt = service.complete_worker_stop(
            "worker-1",
            Some(String::from("/worktrees/worker-1")),
            3,
        )?;
        let replay = service.replay_after(1)?;

        assert!(receipt.pty_stopped);
        assert_eq!(replay.events.len(), 2);
        assert!(matches!(
            &replay.events[1].event,
            HiveDomainEvent::WorkerTeardown(event) if event == &receipt
        ));
        Ok(())
    }

    #[test]
    fn agent_hook_retry_is_idempotent_and_replays_one_shot_steer() -> Result<(), HiveServiceError> {
        let service = HiveTasksService::new(
            root("agent-hook-retry").map_err(HiveServiceError::Store)?,
            8,
            1,
        )?;
        service.steer("god", "keep this private", 1)?;

        let first = service.process_agent_hook(
            "god",
            "event-1",
            AgentHookEvent::PreToolUse,
            Some("Bash"),
            2,
        )?;
        let retry = service.process_agent_hook(
            "god",
            "event-1",
            AgentHookEvent::PreToolUse,
            Some("Bash"),
            3,
        )?;
        let replay = service.replay_after(1)?;

        assert_eq!(first, retry);
        assert_eq!(first.steer.as_deref(), Some("keep this private"));
        assert_eq!(replay.events.len(), 1);
        assert!(matches!(
            &replay.events[0].event,
            HiveDomainEvent::AgentHookObserved { event_id, .. } if event_id == "event-1"
        ));
        Ok(())
    }
}
