use dioxus::prelude::*;
use md_web_contracts::domains::pty_agents::{
    AgentRecord, PtyDimensions, RestartAgentRequest, RestoreAgentRequest, SpawnAgentRequest,
    SpawnAgentResult,
};

#[cfg(feature = "server")]
mod server {
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::Duration;

    use dioxus::server::axum::body::Bytes;
    use dioxus::server::axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use dioxus::server::axum::extract::{Json, Path as RoutePath};
    use dioxus::server::axum::http::{HeaderMap, StatusCode};
    use dioxus::server::axum::response::Response;
    use futures_util::{SinkExt, StreamExt};
    use md_web_contracts::domains::fs_git_ide::{
        PrivateWorkspaceCapability, ProvisionWorktreeRequest, WorkspaceCapability, WorkspaceId,
    };
    use md_web_contracts::domains::persistence::{
        FloorAgentWrite, NaturalExitDisposition, NaturalExitWrite, TerminalQueueEnqueue,
        TerminalQueueHeadMutation,
    };
    use md_web_contracts::domains::pty_agents::{
        AgentHookDecision, AgentHookEvent, AgentHookRequest, AgentProvider, AgentRecord,
        AgentStatus, PtyDimensions, PtyExitEvent, QueuedTerminalMessage, RestartAgentRequest,
        RestoreAgentRequest, SpawnAgentRequest, SpawnAgentResult, TerminalClientFrame,
        TerminalServerFrame,
    };
    use md_web_services::domains::persistence::PgPersistenceRepository;
    use md_web_services::domains::pty_agents::{
        AgentHookLaunch, DeliveryDecision, DeliveryGate, PtyRegistry, TerminalFrameRouter,
        TerminalQueue, evaluate_terminal_readiness, render_claude_hook_response,
        render_gemini_hook_response, restart_spawn_request, restore_spawn_request,
    };
    use md_web_services::{PrivateWorkspaceRoot, WorkspaceRegistry, WorktreeProvisioner};
    use tokio::sync::OnceCell;
    use uuid::Uuid;

    static REGISTRY: OnceLock<Arc<PtyRegistry>> = OnceLock::new();
    static AGENTS: OnceLock<Mutex<BTreeMap<String, DurableAgent>>> = OnceLock::new();
    static HYDRATED: AtomicBool = AtomicBool::new(false);
    static RESETTING: AtomicBool = AtomicBool::new(false);
    static HYDRATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static WORKTREES: OnceLock<Mutex<BTreeMap<PathBuf, Arc<WorktreeProvisioner>>>> =
        OnceLock::new();
    static WORKTREE_IDS: OnceLock<Mutex<BTreeMap<String, (PathBuf, String)>>> = OnceLock::new();
    static SPAWNED_AT: OnceLock<Mutex<BTreeMap<String, i64>>> = OnceLock::new();
    static INPUT_DRAFTS: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    static PENDING_EXITS: OnceLock<Mutex<VecDeque<PendingExit>>> = OnceLock::new();
    static LIFECYCLE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    static RUNTIME_STARTED: OnceCell<()> = OnceCell::const_new();
    static DELIVERY_NOTIFY: tokio::sync::Notify = tokio::sync::Notify::const_new();
    const FLOOR_ID: &str = "local";

    #[derive(Clone)]
    struct DurableAgent {
        record: AgentRecord,
        revision: i64,
    }

    #[derive(Clone)]
    struct PendingExit {
        agent_id: String,
        pty_id: String,
        generation: u64,
        disposition: NaturalExitDisposition,
        exit_code: Option<i32>,
        event_id: String,
    }

    pub(super) fn registry() -> &'static Arc<PtyRegistry> {
        REGISTRY.get_or_init(|| Arc::new(PtyRegistry::new()))
    }

    fn agents() -> &'static Mutex<BTreeMap<String, DurableAgent>> {
        AGENTS.get_or_init(|| Mutex::new(BTreeMap::new()))
    }

    async fn repository() -> Result<PgPersistenceRepository, ()> {
        super::super::persistence_repository().await.map_err(|_| ())
    }

    async fn ensure_hydrated() -> Result<(), ()> {
        if !HYDRATED.load(Ordering::Acquire) {
            let _guard = HYDRATION_LOCK.lock().await;
            if !HYDRATED.load(Ordering::Acquire) {
                let rows = repository()
                    .await?
                    .list_floor_agents(FLOOR_ID, 1_000)
                    .await
                    .map_err(|_| ())?;
                let mut state = agents().lock().map_err(|_| ())?;
                state.clear();
                for row in rows {
                    state.insert(
                        row.agent.id.clone(),
                        DurableAgent {
                            record: row.agent,
                            revision: row.revision,
                        },
                    );
                }
                HYDRATED.store(true, Ordering::Release);
            }
        }
        ensure_runtime_started().await
    }

    async fn ensure_runtime_started() -> Result<(), ()> {
        RUNTIME_STARTED
            .get_or_try_init(|| async {
                let (exit_tx, exit_rx) = tokio::sync::mpsc::unbounded_channel();
                registry()
                    .start_exit_monitor(move |event| {
                        let _ = exit_tx.send(event);
                    })
                    .map_err(|_| ())?;
                tokio::spawn(exit_monitor_loop(exit_rx));
                tokio::spawn(delivery_loop());
                Ok::<(), ()>(())
            })
            .await
            .map(|_| ())
    }

    async fn exit_monitor_loop(mut receiver: tokio::sync::mpsc::UnboundedReceiver<PtyExitEvent>) {
        let mut retry = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                event = receiver.recv() => {
                    let Some(event) = event else { return };
                    let _ = mark_natural_exit(&event.pty_id, event.generation, event.exit.exit_code).await;
                }
                _ = retry.tick() => flush_pending_exits().await,
            }
        }
    }

    async fn flush_pending_exits() {
        let pending = PENDING_EXITS
            .get_or_init(|| Mutex::new(VecDeque::new()))
            .lock()
            .map(|mut queue| queue.drain(..).collect::<Vec<_>>())
            .unwrap_or_default();
        for exit in pending {
            // A deliberate kill removes the registry entry before PostgreSQL is updated.
            // Only a known replacement generation makes this exit stale; NotFound remains
            // retryable so a transient database failure cannot lose the terminal state.
            if matches!(
                registry().is_current_generation(&exit.pty_id, exit.generation),
                Ok(false)
            ) {
                continue;
            }
            if persist_exit(
                &exit.pty_id,
                exit.disposition,
                exit.exit_code,
                &exit.event_id,
            )
            .await
            .is_ok()
            {
                record_exit_activity(&exit.agent_id);
            } else if let Ok(mut queue) = PENDING_EXITS
                .get_or_init(|| Mutex::new(VecDeque::new()))
                .lock()
            {
                queue.push_back(exit);
            }
        }
    }

    async fn delivery_loop() {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        let mut shutdown = super::super::shutdown_receiver();
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = DELIVERY_NOTIFY.notified() => {}
                _ = shutdown.changed() => return,
            }
            if RESETTING.load(Ordering::Acquire) {
                continue;
            }
            let agent_ids = agents()
                .lock()
                .map(|rows| rows.keys().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for agent_id in agent_ids {
                let _ = deliver_next(&agent_id).await;
            }
        }
    }

    pub(super) async fn list() -> Result<(Vec<AgentRecord>, Vec<AgentRecord>), ()> {
        ensure_hydrated().await?;
        let rows = agents().lock().map_err(|_| ())?;
        let active = rows
            .values()
            .map(|row| &row.record)
            .filter(|row| !matches!(row.status, AgentStatus::Archived | AgentStatus::Restorable))
            .cloned()
            .collect();
        let restorable = rows
            .values()
            .map(|row| &row.record)
            .filter(|row| row.status == AgentStatus::Restorable)
            .cloned()
            .collect();
        Ok((active, restorable))
    }

    pub(super) async fn workspace_private_capabilities()
    -> Result<Vec<PrivateWorkspaceCapability>, ()> {
        ensure_hydrated().await?;
        Ok(agents()
            .lock()
            .map_err(|_| ())?
            .values()
            .filter_map(|state| state.record.workspace_capability.clone())
            .collect())
    }

    pub(super) async fn unarchive(agent_id: &str) -> Result<AgentRecord, ()> {
        ensure_hydrated().await?;
        let agent = agents()
            .lock()
            .map_err(|_| ())?
            .get(agent_id)
            .map(|state| state.record.clone())
            .filter(|record| record.status == AgentStatus::Archived)
            .ok_or(())?;
        restore(RestoreAgentRequest {
            agent,
            prefer_worktree: true,
        })
        .await?;
        agents()
            .lock()
            .map_err(|_| ())?
            .get(agent_id)
            .map(|state| state.record.clone())
            .ok_or(())
    }

    pub(super) async fn spawn(mut request: SpawnAgentRequest) -> Result<SpawnAgentResult, ()> {
        ensure_hydrated()
            .await
            .map_err(|_| spawn_failure("hydrate"))?;
        if request.id.trim().is_empty() {
            request.id = format!("agent-{}", Uuid::new_v4());
        }
        let config = persisted_config()
            .await
            .map_err(|_| spawn_failure("config"))?;
        let workspaces = configured_workspaces(&config);
        validate_authority(&request, &workspaces).map_err(|_| spawn_failure("authority"))?;
        let worktree = provision_worktree(&request, &workspaces, &config)
            .map_err(|_| spawn_failure("worktree"))?;
        if let Some((worktree_root, capability)) = &worktree {
            request.cwd.clone_from(&capability.path);
            request.isolate = false;
            WORKTREE_IDS
                .get_or_init(|| Mutex::new(BTreeMap::new()))
                .lock()
                .map_err(|_| ())?
                .insert(
                    request.id.clone(),
                    (worktree_root.clone(), capability.id.clone()),
                );
        }
        let mut record = record_from_request(&request, None);
        record.worktree_path = worktree
            .as_ref()
            .map(|(_, capability)| capability.path.clone());
        record.workspace_capability = worktree.map(|(_, capability)| capability);
        let hook = agent_hook_launch(request.provider, &request.id, &config)
            .map_err(|_| spawn_failure("hook"))?;
        let result = match registry().spawn_with_hook(request, hook) {
            Ok(result) => result,
            Err(_) => {
                rollback_worktree(&record.id, &workspaces);
                spawn_failure("process");
                return Err(());
            }
        };
        record.pty_id = Some(result.pty_id.clone());
        record.cwd = result.cwd.clone();
        record.status = AgentStatus::Idle;
        record.action_ja = String::from("待機中");
        let persisted = match repository()
            .await?
            .upsert_floor_agent(&FloorAgentWrite {
                floor_id: String::from(FLOOR_ID),
                expected_revision: 0,
                agent: record.clone(),
            })
            .await
        {
            Ok(persisted) => persisted,
            Err(_) => {
                let _ = registry().kill(&result.pty_id);
                rollback_worktree(&record.id, &workspaces);
                spawn_failure("persist");
                return Err(());
            }
        };
        agents().lock().map_err(|_| ())?.insert(
            record.id.clone(),
            DurableAgent {
                record,
                revision: persisted.revision,
            },
        );
        SPAWNED_AT
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| ())?
            .insert(persisted.agent.id.clone(), now_ms());
        if !persisted.agent.role.orchestrator {
            let config = persisted_config().await?;
            if super::super::hive_register_worker_projection(
                md_web_contracts::domains::hive_tasks::WorkerSnapshot {
                    worker_id: persisted.agent.id.clone(),
                    request_id: Uuid::new_v4().to_string(),
                    name: persisted.agent.name.clone(),
                    base_branch: String::from("HEAD"),
                    spawned_at: now_ms(),
                    age_ms: 0,
                    idle_ms: Some(0),
                    tokens_used: 0,
                    token_cap: config.agent_token_caps.get(&persisted.agent.id).copied(),
                    has_slack: false,
                    status: md_web_contracts::domains::hive_tasks::WorkerStatus::Working,
                },
            )
            .await
            .is_err()
            {
                let _ = registry().kill(&result.pty_id);
                let event_id = Uuid::new_v4().to_string();
                let _ = persist_exit(
                    &result.pty_id,
                    NaturalExitDisposition::Archived,
                    None,
                    &event_id,
                )
                .await;
                spawn_failure("worker_projection");
                return Err(());
            }
        }
        super::super::record_activity_event(
            md_web_contracts::domains::memory_skills::ActivityEntry {
                timestamp_ms: now_ms(),
                kind: String::from("agent_spawned"),
                summary: format!("{}を起動", persisted.agent.name),
                details: BTreeMap::new(),
            },
        );
        Ok(SpawnAgentResult {
            worktree_path: persisted.agent.worktree_path,
            ..result
        })
    }

    pub(super) async fn kill(pty_id: &str) -> Result<(), ()> {
        let _lifecycle = LIFECYCLE_SERIAL.lock().await;
        ensure_hydrated().await?;
        let generation = registry().current_generation(pty_id).map_err(|_| ())?;
        let agent_id = agents()
            .lock()
            .map_err(|_| ())?
            .values()
            .find(|state| state.record.pty_id.as_deref() == Some(pty_id))
            .map(|state| state.record.id.clone())
            .ok_or(())?;
        registry().kill(pty_id).map_err(|_| ())?;
        let event_id = Uuid::new_v4().to_string();
        if persist_exit(pty_id, NaturalExitDisposition::Archived, None, &event_id)
            .await
            .is_err()
        {
            PENDING_EXITS
                .get_or_init(|| Mutex::new(VecDeque::new()))
                .lock()
                .map_err(|_| ())?
                .push_back(PendingExit {
                    agent_id,
                    pty_id: String::from(pty_id),
                    generation,
                    disposition: NaturalExitDisposition::Archived,
                    exit_code: None,
                    event_id,
                });
        }
        Ok(())
    }

    pub(super) async fn shutdown_all() -> Result<(), ()> {
        let _lifecycle = LIFECYCLE_SERIAL.lock().await;
        let pty_ids = registry()
            .list()
            .map_err(|_| ())?
            .into_iter()
            .map(|pty| pty.id)
            .collect::<Vec<_>>();
        if pty_ids.is_empty() {
            return Ok(());
        }
        ensure_hydrated().await?;
        for pty_id in pty_ids {
            let generation = registry().current_generation(&pty_id).map_err(|_| ())?;
            let agent_id = agents()
                .lock()
                .map_err(|_| ())?
                .values()
                .find(|state| state.record.pty_id.as_deref() == Some(pty_id.as_str()))
                .map(|state| state.record.id.clone())
                .ok_or(())?;
            registry().kill(&pty_id).map_err(|_| ())?;
            let event_id = Uuid::new_v4().to_string();
            if persist_exit(&pty_id, NaturalExitDisposition::Archived, None, &event_id)
                .await
                .is_err()
            {
                PENDING_EXITS
                    .get_or_init(|| Mutex::new(VecDeque::new()))
                    .lock()
                    .map_err(|_| ())?
                    .push_back(PendingExit {
                        agent_id,
                        pty_id,
                        generation,
                        disposition: NaturalExitDisposition::Archived,
                        exit_code: None,
                        event_id,
                    });
            }
        }
        for attempt in 0..3 {
            flush_pending_exits().await;
            let pending = PENDING_EXITS
                .get_or_init(|| Mutex::new(VecDeque::new()))
                .lock()
                .map_err(|_| ())?
                .is_empty();
            if pending {
                return Ok(());
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Err(())
    }

    pub(super) async fn prepare_namespace_reset() -> Result<(), ()> {
        if RESETTING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(());
        }
        if shutdown_all().await.is_err() {
            RESETTING.store(false, Ordering::Release);
            return Err(());
        }
        Ok(())
    }

    pub(super) fn finish_namespace_reset(committed: bool) -> Result<(), ()> {
        let result = if committed {
            reset_runtime_projection()
        } else {
            agents().lock().map_err(|_| ())?.clear();
            HYDRATED.store(false, Ordering::Release);
            Ok(())
        };
        RESETTING.store(false, Ordering::Release);
        result
    }

    pub(super) fn reset_runtime_projection() -> Result<(), ()> {
        if !registry().list().map_err(|_| ())?.is_empty() {
            return Err(());
        }
        agents().lock().map_err(|_| ())?.clear();
        if let Some(worktree_ids) = WORKTREE_IDS.get() {
            worktree_ids.lock().map_err(|_| ())?.clear();
        }
        if let Some(spawned_at) = SPAWNED_AT.get() {
            spawned_at.lock().map_err(|_| ())?.clear();
        }
        if let Some(worktrees) = WORKTREES.get() {
            worktrees.lock().map_err(|_| ())?.clear();
        }
        HYDRATED.store(false, Ordering::Release);
        Ok(())
    }

    async fn mark_natural_exit(
        pty_id: &str,
        generation: u64,
        exit_code: Option<i32>,
    ) -> Result<(), ()> {
        let _lifecycle = LIFECYCLE_SERIAL.lock().await;
        if !matches!(
            registry().is_current_generation(pty_id, generation),
            Ok(true)
        ) {
            return Ok(());
        }
        let (agent_id, disposition) = {
            let rows = agents().lock().map_err(|_| ())?;
            let Some(state) = rows
                .values()
                .find(|row| row.record.pty_id.as_deref() == Some(pty_id))
            else {
                return Ok(());
            };
            let disposition = if state.record.role.orchestrator || state.record.role.assistant {
                NaturalExitDisposition::Archived
            } else {
                NaturalExitDisposition::Restorable
            };
            (state.record.id.clone(), disposition)
        };
        let event_id = Uuid::new_v4().to_string();
        if persist_exit(pty_id, disposition, exit_code, &event_id)
            .await
            .is_err()
        {
            PENDING_EXITS
                .get_or_init(|| Mutex::new(VecDeque::new()))
                .lock()
                .map_err(|_| ())?
                .push_back(PendingExit {
                    agent_id,
                    pty_id: String::from(pty_id),
                    generation,
                    disposition,
                    exit_code,
                    event_id,
                });
            return Ok(());
        }
        record_exit_activity(&agent_id);
        Ok(())
    }

    fn record_exit_activity(agent_id: &str) {
        super::super::record_activity_event(
            md_web_contracts::domains::memory_skills::ActivityEntry {
                timestamp_ms: now_ms(),
                kind: String::from("terminal_exit"),
                summary: format!("{agent_id}のターミナルが終了"),
                details: BTreeMap::new(),
            },
        );
    }

    pub(super) async fn queue(agent_id: &str, text: &str) -> Result<(), ()> {
        queue_inner(agent_id, text, true).await
    }

    pub(super) async fn queue_system(agent_id: &str, text: &str) -> Result<(), ()> {
        queue_inner(agent_id, text, false).await
    }

    async fn queue_inner(agent_id: &str, text: &str, record_history: bool) -> Result<(), ()> {
        ensure_hydrated().await?;
        if text.trim().is_empty() {
            return Err(());
        }
        let record = agents()
            .lock()
            .map_err(|_| ())?
            .get(agent_id)
            .map(|state| state.record.clone())
            .ok_or(())?;
        if record.pty_id.is_none() {
            return Err(());
        }
        let message_id = Uuid::new_v4().to_string();
        let message = QueuedTerminalMessage {
            id: message_id.clone(),
            agent_id: String::from(agent_id),
            text: String::from(text),
            instruction: None,
            queued_at_ms: now_ms(),
            manual: true,
            precondition: None,
            failed_attempts: 0,
        };
        let repository = repository().await?;
        let mut enqueued = false;
        for _ in 0..3 {
            let current = repository
                .load_terminal_queue(FLOOR_ID, agent_id)
                .await
                .map_err(|_| ())?;
            if repository
                .enqueue_terminal_message(&TerminalQueueEnqueue {
                    floor_id: String::from(FLOOR_ID),
                    expected_revision: current.as_ref().map_or(0, |queue| queue.revision),
                    message: message.clone(),
                })
                .await
                .is_ok()
            {
                enqueued = true;
                break;
            }
        }
        if !enqueued {
            return Err(());
        }
        if record_history
            && super::super::record_prompt_accepted(
                md_web_contracts::domains::persistence::HistoryAppend {
                    event_id: message_id,
                    agent_id: String::from(agent_id),
                    cwd: Some(record.cwd),
                    text: String::from(text),
                    occurred_at_ms: now_ms(),
                },
            )
            .await
            .is_err()
        {
            eprintln!("PTY_HISTORY_FAILED code=write_failed");
        }
        DELIVERY_NOTIFY.notify_one();
        Ok(())
    }

    async fn deliver_next(agent_id: &str) -> Result<(), ()> {
        let (record, spawned_at) = {
            let rows = agents().lock().map_err(|_| ())?;
            let state = rows.get(agent_id).ok_or(())?;
            let spawned_at = SPAWNED_AT
                .get()
                .and_then(|times| times.lock().ok()?.get(agent_id).copied())
                .unwrap_or(0);
            (state.record.clone(), spawned_at)
        };
        let pty_id = record.pty_id.clone().ok_or(())?;
        let summary = registry()
            .list()
            .map_err(|_| ())?
            .into_iter()
            .find(|pty| pty.id == pty_id)
            .ok_or(())?;
        let now = now_ms();
        let quiet_ms = if summary.last_output_at_ms <= 0 {
            Some(u64::MAX)
        } else {
            u64::try_from(now.saturating_sub(summary.last_output_at_ms)).ok()
        };
        let boot_elapsed_ms = u64::try_from(now.saturating_sub(spawned_at)).unwrap_or(0);
        let quiet_elapsed_ms = quiet_ms.unwrap_or(0);
        let presence = registry().presence(&pty_id).map_err(|_| ())?;
        let readiness = evaluate_terminal_readiness(
            summary.has_output,
            boot_elapsed_ms,
            quiet_elapsed_ms,
            quiet_elapsed_ms,
            presence,
        );
        let repository = repository().await?;
        let persisted = repository
            .load_terminal_queue(FLOOR_ID, agent_id)
            .await
            .map_err(|_| ())?
            .ok_or(())?;
        let queue_revision = persisted.revision;
        let mut queue = TerminalQueue::new();
        for message in persisted.messages {
            queue.enqueue(message).map_err(|_| ())?;
        }
        if queue.selected_head_id(agent_id).is_none() {
            return Ok(());
        }
        let decision = super::super::hive_control_hook_decision(agent_id, None)
            .await
            .map_err(|_| ())?;
        match queue.decide(
            agent_id,
            DeliveryGate {
                status: record.status,
                quiet_ms,
                has_initial_output: readiness.has_initial_output,
                presence: readiness.presence,
                automation_safe: !(decision.paused || decision.halted || decision.tool_gated),
                auto_delivery_paused: decision.auto_delivery_paused,
                boot_grace_remaining_ms: readiness.boot_grace_remaining_ms,
                cooldown_remaining_ms: readiness.cooldown_remaining_ms,
                inbox_nonempty: None,
            },
        ) {
            DeliveryDecision::Empty | DeliveryDecision::Wait => Ok(()),
            DeliveryDecision::Drop { message_id } => repository
                .acknowledge_terminal_message(&TerminalQueueHeadMutation {
                    floor_id: String::from(FLOOR_ID),
                    agent_id: String::from(agent_id),
                    message_id,
                    expected_revision: queue_revision,
                })
                .await
                .map(|_| ())
                .map_err(|_| ()),
            DeliveryDecision::Send { message_id, text } => {
                let registry = Arc::clone(registry());
                let target = pty_id.clone();
                let steer = decision.steer;
                let delivered = tokio::task::spawn_blocking(move || {
                    if let Some(steer) = steer {
                        registry.deliver_queued_message(&target, &steer)?;
                    }
                    registry.deliver_queued_message(&target, &text)
                })
                .await
                .map_err(|_| ())?
                .is_ok();
                let mutation = TerminalQueueHeadMutation {
                    floor_id: String::from(FLOOR_ID),
                    agent_id: String::from(agent_id),
                    message_id,
                    expected_revision: queue_revision,
                };
                if delivered {
                    repository
                        .acknowledge_terminal_message(&mutation)
                        .await
                        .map(|_| ())
                        .map_err(|_| ())
                } else {
                    repository
                        .record_terminal_failure(&mutation)
                        .await
                        .map(|_| ())
                        .map_err(|_| ())
                }
            }
        }
    }

    pub(super) async fn restart(request: RestartAgentRequest) -> Result<SpawnAgentResult, ()> {
        let _lifecycle = LIFECYCLE_SERIAL.lock().await;
        ensure_hydrated().await?;
        let current = agents()
            .lock()
            .map_err(|_| ())?
            .get(&request.agent_id)
            .cloned()
            .ok_or(())?;
        let pty_id = current.record.pty_id.clone().ok_or(())?;
        let dimensions = registry()
            .list()
            .map_err(|_| ())?
            .into_iter()
            .find(|pty| pty.id == pty_id)
            .map(|pty| pty.dimensions)
            .ok_or(())?;
        let spawn_request =
            restart_spawn_request(&current.record, &request, dimensions).map_err(|_| ())?;
        let config = persisted_config()
            .await
            .map_err(|_| restore_failure("config"))?;
        let workspaces = configured_workspaces(&config);
        validate_authority(&spawn_request, &workspaces)?;
        registry().mark_relaunching(&pty_id).map_err(|_| ())?;
        registry().kill(&pty_id).map_err(|_| ())?;
        let hook = agent_hook_launch(request.provider, &request.agent_id, &config)?;
        match registry().spawn_with_hook(spawn_request, hook) {
            Ok(result) => {
                let replacement_generation = registry()
                    .current_generation(&result.pty_id)
                    .map_err(|_| ())?;
                let mut record = current.record.clone();
                record.provider = request.provider;
                record.model = request.model;
                record.pty_id = Some(result.pty_id.clone());
                record.status = AgentStatus::Idle;
                record.action_ja = String::from("再起動済み");
                let repository = repository().await?;
                let persisted = match repository
                    .upsert_floor_agent(&FloorAgentWrite {
                        floor_id: String::from(FLOOR_ID),
                        expected_revision: current.revision,
                        agent: record.clone(),
                    })
                    .await
                {
                    Ok(persisted) => persisted,
                    Err(_) => {
                        let _ = registry().kill_generation(&result.pty_id, replacement_generation);
                        if let Ok(Some(canonical)) = repository
                            .get_floor_agent(FLOOR_ID, &request.agent_id)
                            .await
                        {
                            agents().lock().map_err(|_| ())?.insert(
                                request.agent_id.clone(),
                                DurableAgent {
                                    record: canonical.agent,
                                    revision: canonical.revision,
                                },
                            );
                        }
                        return Err(());
                    }
                };
                let agent_id = request.agent_id.clone();
                agents().lock().map_err(|_| ())?.insert(
                    request.agent_id.clone(),
                    DurableAgent {
                        record,
                        revision: persisted.revision,
                    },
                );
                SPAWNED_AT
                    .get_or_init(|| Mutex::new(BTreeMap::new()))
                    .lock()
                    .map_err(|_| ())?
                    .insert(agent_id, now_ms());
                Ok(result)
            }
            Err(_) => {
                if let Some(state) = agents().lock().map_err(|_| ())?.get_mut(&request.agent_id) {
                    state.record.pty_id = None;
                    state.record.status = AgentStatus::Exited;
                    state.record.action_ja = String::from("再起動に失敗");
                }
                Err(())
            }
        }
    }

    pub(super) async fn restore(request: RestoreAgentRequest) -> Result<SpawnAgentResult, ()> {
        let _lifecycle = LIFECYCLE_SERIAL.lock().await;
        ensure_hydrated()
            .await
            .map_err(|_| restore_failure("hydrate"))?;
        let repository = repository()
            .await
            .map_err(|_| restore_failure("repository"))?;
        let persisted_agent = repository
            .get_floor_agent(FLOOR_ID, &request.agent.id)
            .await
            .map_err(|_| restore_failure("load_agent"))?
            .filter(|persisted| {
                matches!(
                    persisted.agent.status,
                    AgentStatus::Restorable | AgentStatus::Archived
                )
            })
            .ok_or_else(|| restore_failure("missing_agent"))?;
        let expected_revision = persisted_agent.revision;
        let request = RestoreAgentRequest {
            agent: persisted_agent.agent,
            prefer_worktree: request.prefer_worktree,
        };
        let worktree_available = request
            .agent
            .worktree_path
            .as_deref()
            .and_then(|path| Path::new(path).canonicalize().ok())
            .is_some();
        let spawn_request = restore_spawn_request(
            &request,
            PtyDimensions {
                cols: 100,
                rows: 30,
            },
            worktree_available,
        )
        .map_err(|_| restore_failure("recipe"))?;
        let config = persisted_config()
            .await
            .map_err(|_| restore_failure("config"))?;
        let workspaces = configured_workspaces(&config);
        validate_authority(&spawn_request, &workspaces)
            .map_err(|_| restore_failure("authority"))?;
        let hook = agent_hook_launch(request.agent.provider, &request.agent.id, &config)
            .map_err(|_| restore_failure("hook"))?;
        let result = registry()
            .spawn_with_hook(spawn_request, hook)
            .map_err(|_| restore_failure("spawn"))?;
        let mut record = request.agent;
        record.pty_id = Some(result.pty_id.clone());
        record.status = AgentStatus::Idle;
        record.action_ja = String::from("復元済み");
        record.archived = false;
        let persisted = match repository
            .upsert_floor_agent(&FloorAgentWrite {
                floor_id: String::from(FLOOR_ID),
                expected_revision,
                agent: record.clone(),
            })
            .await
        {
            Ok(persisted) => persisted,
            Err(_) => {
                let _ = registry().kill(&result.pty_id);
                restore_failure("persist");
                return Err(());
            }
        };
        let agent_id = record.id.clone();
        agents()
            .lock()
            .map_err(|_| restore_failure("state_commit"))?
            .insert(
                record.id.clone(),
                DurableAgent {
                    record,
                    revision: persisted.revision,
                },
            );
        SPAWNED_AT
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| restore_failure("spawn_time"))?
            .insert(agent_id, now_ms());
        Ok(result)
    }

    pub(super) async fn input(pty_id: &str, data: &str) -> Result<(), ()> {
        ensure_hydrated().await?;
        let record = agents()
            .lock()
            .map_err(|_| ())?
            .values()
            .find(|state| state.record.pty_id.as_deref() == Some(pty_id))
            .map(|state| state.record.clone())
            .ok_or(())?;
        registry().write(pty_id, data).map_err(|_| ())?;
        for prompt in accepted_prompts(pty_id, data)? {
            let _ = super::super::office_agent_activity(
                &record.id,
                AgentStatus::Working,
                "指示を処理中",
                Some(&prompt),
                false,
            );
            if super::super::record_prompt_accepted(
                md_web_contracts::domains::persistence::HistoryAppend {
                    event_id: Uuid::new_v4().to_string(),
                    agent_id: record.id.clone(),
                    cwd: Some(record.cwd.clone()),
                    text: prompt,
                    occurred_at_ms: now_ms(),
                },
            )
            .await
            .is_err()
            {
                eprintln!("PTY_HISTORY_FAILED code=write_failed");
            }
        }
        super::super::record_activity_event(
            md_web_contracts::domains::memory_skills::ActivityEntry {
                timestamp_ms: now_ms(),
                kind: String::from("terminal_input"),
                summary: format!("{}へターミナル入力", record.id),
                details: BTreeMap::new(),
            },
        );
        Ok(())
    }

    fn accepted_prompts(pty_id: &str, data: &str) -> Result<Vec<String>, ()> {
        const MAX_DRAFT_BYTES: usize = 256 * 1024;
        let mut drafts = INPUT_DRAFTS
            .get_or_init(|| Mutex::new(BTreeMap::new()))
            .lock()
            .map_err(|_| ())?;
        let draft = drafts.entry(String::from(pty_id)).or_default();
        let mut accepted = Vec::new();
        for character in data.chars() {
            match character {
                '\r' | '\n' => {
                    let prompt = std::mem::take(draft);
                    if !prompt.trim().is_empty() {
                        accepted.push(prompt);
                    }
                }
                '\u{8}' | '\u{7f}' => {
                    draft.pop();
                }
                character if !character.is_control() && draft.len() < MAX_DRAFT_BYTES => {
                    draft.push(character);
                }
                _ => {}
            }
        }
        Ok(accepted)
    }

    fn restore_failure(stage: &'static str) {
        eprintln!("PTY_RESTORE_FAILED stage={stage}");
    }

    fn spawn_failure(stage: &'static str) {
        eprintln!("PTY_SPAWN_FAILED stage={stage}");
    }

    fn agent_hook_launch(
        provider: AgentProvider,
        agent_id: &str,
        config: &md_web_contracts::domains::config_onboarding::PublicConfig,
    ) -> Result<Option<AgentHookLaunch>, ()> {
        if !matches!(
            provider,
            AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Gemini
        ) {
            return Ok(None);
        }
        let scheme = if std::env::var("MD_WEB_HTTPS").is_ok_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        }) {
            "https"
        } else {
            "http"
        };
        let port = std::env::var("PORT").unwrap_or_else(|_| String::from("5080"));
        if port.parse::<u16>().is_err() {
            return Err(());
        }
        let runtime_root = config
            .harness_home
            .as_deref()
            .and_then(|path| Path::new(path).canonicalize().ok())
            .filter(|path| path.is_dir())
            .ok_or(())?;
        AgentHookLaunch::new(
            format!("{scheme}://127.0.0.1:{port}/internal/hive-hook"),
            agent_id,
            Uuid::new_v4().to_string(),
            runtime_root,
        )
        .map(Some)
        .map_err(|_| ())
    }

    pub(super) async fn agent_hook(
        Json(request): Json<AgentHookRequest>,
    ) -> Result<Json<AgentHookDecision>, StatusCode> {
        verified_hook_decision(request).await.map(Json)
    }

    pub(super) async fn provider_agent_hook(
        RoutePath(provider): RoutePath<String>,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<Json<serde_json::Value>, StatusCode> {
        if body.len() > 256 * 1024 || !matches!(provider.as_str(), "claude" | "codex" | "gemini") {
            return Err(StatusCode::BAD_REQUEST);
        }
        let payload = serde_json::from_slice::<serde_json::Value>(&body)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let object = payload.as_object().ok_or(StatusCode::BAD_REQUEST)?;
        let event_name = object
            .get("hook_event_name")
            .or_else(|| object.get("hookEventName"))
            .or_else(|| object.get("event"))
            .and_then(serde_json::Value::as_str)
            .map(String::from)
            .ok_or(StatusCode::BAD_REQUEST)?;
        let event = parse_hook_event(&event_name).ok_or(StatusCode::BAD_REQUEST)?;
        let agent_id = hook_header(&headers, "x-md-agent-id")?;
        let capability = hook_header(&headers, "x-md-hook-capability")?;
        let tool_name = object
            .get("tool_name")
            .or_else(|| object.get("toolName"))
            .and_then(serde_json::Value::as_str)
            .map(String::from);
        let event_id = object
            .get("event_id")
            .or_else(|| object.get("hook_event_id"))
            .or_else(|| object.get("tool_use_id"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 256)
            .map(String::from)
            .unwrap_or_else(|| {
                let mut hasher = DefaultHasher::new();
                provider.hash(&mut hasher);
                body.hash(&mut hasher);
                format!("provider-{:016x}", hasher.finish())
            });
        let usage_event = provider_usage_kind(&provider).and_then(|kind| {
            provider_payload_has_usage(&payload).then(|| {
                let session_id = object
                    .get("session_id")
                    .or_else(|| object.get("sessionId"))
                    .or_else(|| object.get("conversation_id"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(String::from)
                    .unwrap_or_else(|| format!("{provider}:{agent_id}"));
                let timestamp_ms = object
                    .get("timestamp_ms")
                    .or_else(|| object.get("timestampMs"))
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_else(now_ms);
                (kind, session_id, timestamp_ms, payload.to_string())
            })
        });
        let decision = verified_hook_decision(AgentHookRequest {
            agent_id: agent_id.clone(),
            capability,
            event_id: event_id.clone(),
            event,
            tool_name,
            payload,
        })
        .await?;
        if let Some((kind, session_id, timestamp_ms, payload_json)) = usage_event {
            super::super::record_provider_transcript(
                kind,
                &event_id,
                &agent_id,
                &session_id,
                timestamp_ms,
                &payload_json,
            )
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        }
        let response = if provider == "gemini" {
            render_gemini_hook_response(&event_name, &decision)
        } else {
            render_claude_hook_response(&event_name, &decision)
        };
        Ok(Json(response))
    }

    fn provider_usage_kind(
        provider: &str,
    ) -> Option<md_web_contracts::domains::memory_skills::ProviderUsageKind> {
        use md_web_contracts::domains::memory_skills::ProviderUsageKind;
        match provider {
            "claude" => Some(ProviderUsageKind::Claude),
            "codex" => Some(ProviderUsageKind::Codex),
            "gemini" => Some(ProviderUsageKind::Gemini),
            _ => None,
        }
    }

    fn provider_payload_has_usage(payload: &serde_json::Value) -> bool {
        payload.as_object().is_some_and(|object| {
            ["usage", "modelUsage", "usageMetadata"]
                .into_iter()
                .any(|key| object.contains_key(key))
        })
    }

    fn hook_header(headers: &HeaderMap, name: &'static str) -> Result<String, StatusCode> {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(String::from)
            .ok_or(StatusCode::UNAUTHORIZED)
    }

    fn parse_hook_event(value: &str) -> Option<AgentHookEvent> {
        match value {
            "PreToolUse" | "pre_tool_use" | "BeforeTool" => Some(AgentHookEvent::PreToolUse),
            "PostToolUse" | "post_tool_use" | "AfterTool" => Some(AgentHookEvent::PostToolUse),
            "PostToolUseFailure" | "post_tool_use_failure" => {
                Some(AgentHookEvent::PostToolUseFailure)
            }
            "UserPromptSubmit" | "user_prompt_submit" | "BeforeAgent" => {
                Some(AgentHookEvent::UserPromptSubmit)
            }
            "Notification" | "notification" => Some(AgentHookEvent::Notification),
            "Stop" | "stop" | "AfterAgent" => Some(AgentHookEvent::Stop),
            "SubagentStop" | "subagent_stop" => Some(AgentHookEvent::SubagentStop),
            "SessionStart" | "session_start" => Some(AgentHookEvent::SessionStart),
            "SessionEnd" | "session_end" => Some(AgentHookEvent::SessionEnd),
            _ => None,
        }
    }

    async fn verified_hook_decision(
        request: AgentHookRequest,
    ) -> Result<AgentHookDecision, StatusCode> {
        match registry().verify_hook_request(&request) {
            Ok(true) => {}
            Ok(false) => return Err(StatusCode::UNAUTHORIZED),
            Err(_) => return Err(StatusCode::BAD_REQUEST),
        }
        let decision = super::super::hive_agent_hook_event(&request)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let allow = !(decision.paused || decision.halted || decision.tool_gated);
        let reason_ja = if decision.halted {
            Some(String::from("このエージェントは停止されています。"))
        } else if decision.paused {
            Some(String::from("このエージェントは一時停止中です。"))
        } else if decision.tool_gated {
            Some(String::from("このツールの実行は現在許可されていません。"))
        } else {
            None
        };
        Ok(AgentHookDecision {
            allow,
            reason_ja,
            steer: decision.steer,
        })
    }

    fn validate_authority(
        request: &SpawnAgentRequest,
        workspaces: &WorkspaceRegistry,
    ) -> Result<(), ()> {
        if request.id.trim().is_empty() || request.name.trim().is_empty() {
            return Err(());
        }
        let expected = match request.provider {
            AgentProvider::Claude => "claude",
            AgentProvider::Codex => "codex",
            AgentProvider::Grok => "grok",
            AgentProvider::Kimi => "kimi",
            AgentProvider::Gemini => "gemini",
            AgentProvider::Antigravity => "agy",
            AgentProvider::Qwen => "qwen",
            AgentProvider::OpenCode => "opencode",
            AgentProvider::Crush => "crush",
            AgentProvider::Pi => "pi",
            AgentProvider::Copilot => "copilot",
            AgentProvider::Cursor => "cursor-agent",
            AgentProvider::Custom => return Err(()),
        };
        if request.command != expected || request.command.contains(['/', '\\']) {
            return Err(());
        }
        let candidate = Path::new(&request.cwd).canonicalize().map_err(|_| ())?;
        let workspace = workspaces
            .list()
            .into_iter()
            .filter_map(|workspace| {
                let root = Path::new(&workspace.display_path).canonicalize().ok()?;
                candidate.starts_with(&root).then_some((
                    root.components().count(),
                    root,
                    workspace.capability,
                ))
            })
            .max_by_key(|(depth, _, _)| *depth)
            .ok_or(())?;
        match workspace {
            (_, root, WorkspaceCapability::SourceReadOnly) => (request.isolate
                && candidate == root)
                .then_some(())
                .ok_or(()),
            (_, _, WorkspaceCapability::PrivateMutable) => {
                (!request.isolate).then_some(()).ok_or(())
            }
        }
    }

    async fn persisted_config()
    -> Result<md_web_contracts::domains::config_onboarding::PublicConfig, ()> {
        md_web_services::domains::config_onboarding::load_config(&repository().await?)
            .await
            .map_err(|_| ())
    }

    fn configured_workspaces(
        config: &md_web_contracts::domains::config_onboarding::PublicConfig,
    ) -> WorkspaceRegistry {
        let mut paths = std::env::var_os("MD_REGISTERED_REPOS")
            .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
            .unwrap_or_default();
        if paths.is_empty() {
            paths.extend(config.registered_repos.iter().map(PathBuf::from));
        }
        let sources = WorkspaceRegistry::from_source_paths(paths);
        let Some(harness_home) = std::env::var_os("MD_HARNESS_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| config.harness_home.as_ref().map(PathBuf::from))
            .filter(|path| path.is_absolute())
        else {
            return sources;
        };
        let Ok(authority) = PrivateWorkspaceRoot::new(harness_home.join("worktrees")) else {
            return sources;
        };
        let private_capabilities = agents()
            .lock()
            .map(|records| {
                records
                    .values()
                    .filter_map(|state| state.record.workspace_capability.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        sources.with_private_workspaces(&authority, private_capabilities)
    }

    fn worktree_provisioner(root: PathBuf) -> Result<Arc<WorktreeProvisioner>, ()> {
        let worktrees = WORKTREES.get_or_init(|| Mutex::new(BTreeMap::new()));
        let mut worktrees = worktrees.lock().map_err(|_| ())?;
        if let Some(provisioner) = worktrees.get(&root) {
            return Ok(Arc::clone(provisioner));
        }
        let provisioner = Arc::new(WorktreeProvisioner::new(root.clone()).map_err(|_| ())?);
        worktrees.insert(root, Arc::clone(&provisioner));
        Ok(provisioner)
    }

    fn provision_worktree(
        request: &SpawnAgentRequest,
        workspaces: &WorkspaceRegistry,
        config: &md_web_contracts::domains::config_onboarding::PublicConfig,
    ) -> Result<Option<(PathBuf, PrivateWorkspaceCapability)>, ()> {
        if !request.isolate {
            return Ok(None);
        }
        let harness_home = std::env::var_os("MD_HARNESS_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| config.harness_home.as_ref().map(PathBuf::from))
            .filter(|path| path.is_absolute())
            .ok_or(())?;
        let candidate = Path::new(&request.cwd).canonicalize().map_err(|_| ())?;
        let workspace = workspaces
            .list()
            .into_iter()
            .find(|workspace| {
                workspace.capability == WorkspaceCapability::SourceReadOnly
                    && Path::new(&workspace.display_path) == candidate
            })
            .ok_or(())?;
        let worktree_root = harness_home.join("worktrees");
        let isolated = worktree_provisioner(worktree_root.clone())?
            .create_isolated_worktree(
                workspaces,
                &ProvisionWorktreeRequest {
                    workspace_id: WorkspaceId(workspace.id.0),
                    name: request.id.clone(),
                    base_reference: String::from("HEAD"),
                },
            )
            .map_err(|_| ())?;
        Ok(Some((worktree_root, isolated.capability)))
    }

    fn rollback_worktree(agent_id: &str, workspaces: &WorkspaceRegistry) {
        let worktree = WORKTREE_IDS
            .get()
            .and_then(|ids| ids.lock().ok()?.remove(agent_id));
        if let Some((root, worktree_id)) = worktree
            && let Some(provisioner) = WORKTREES
                .get()
                .and_then(|worktrees| worktrees.lock().ok()?.get(&root).cloned())
        {
            let _ = provisioner.remove_isolated_worktree(workspaces, &worktree_id);
        }
    }

    async fn persist_exit(
        pty_id: &str,
        disposition: NaturalExitDisposition,
        exit_code: Option<i32>,
        event_id: &str,
    ) -> Result<(), ()> {
        let repository = repository().await?;
        let mut current = {
            let rows = agents().lock().map_err(|_| ())?;
            rows.values()
                .find(|state| state.record.pty_id.as_deref() == Some(pty_id))
                .cloned()
                .ok_or(())?
        };
        let mut receipt_revision = None;
        for _ in 0..3 {
            let request = NaturalExitWrite {
                floor_id: String::from(FLOOR_ID),
                agent_id: current.record.id.clone(),
                expected_agent_revision: current.revision,
                event_id: String::from(event_id),
                occurred_at_ms: now_ms(),
                exit_code,
                disposition,
            };
            match repository.persist_agent_exit(&request).await {
                Ok(receipt) => {
                    receipt_revision = Some(receipt.agent_revision);
                    break;
                }
                Err(_) => {
                    let Some(reloaded) = repository
                        .get_floor_agent(FLOOR_ID, &current.record.id)
                        .await
                        .map_err(|_| ())?
                    else {
                        return Err(());
                    };
                    if reloaded.agent.pty_id.is_none()
                        && matches!(
                            reloaded.agent.status,
                            AgentStatus::Archived | AgentStatus::Restorable | AgentStatus::Exited
                        )
                    {
                        current = DurableAgent {
                            record: reloaded.agent,
                            revision: reloaded.revision,
                        };
                        receipt_revision = Some(current.revision);
                        break;
                    }
                    current = DurableAgent {
                        record: reloaded.agent,
                        revision: reloaded.revision,
                    };
                }
            }
        }
        let receipt_revision = receipt_revision.ok_or(())?;
        let agent_id = current.record.id.clone();
        let mut record = current.record;
        record.pty_id = None;
        record.status = match disposition {
            NaturalExitDisposition::Archived => AgentStatus::Archived,
            NaturalExitDisposition::Restorable => AgentStatus::Restorable,
            NaturalExitDisposition::Exited => AgentStatus::Exited,
        };
        record.action_ja = match disposition {
            NaturalExitDisposition::Archived => String::from("アーカイブ済み"),
            NaturalExitDisposition::Restorable => String::from("復元可能"),
            NaturalExitDisposition::Exited => String::from("終了"),
        };
        record.archived = disposition == NaturalExitDisposition::Archived;
        agents().lock().map_err(|_| ())?.insert(
            record.id.clone(),
            DurableAgent {
                record,
                revision: receipt_revision,
            },
        );
        if let Some((root, worktree_id)) = WORKTREE_IDS
            .get()
            .and_then(|ids| ids.lock().ok()?.get(&agent_id).cloned())
            && let Some(provisioner) = WORKTREES
                .get()
                .and_then(|worktrees| worktrees.lock().ok()?.get(&root).cloned())
        {
            let _ = provisioner.archive(&worktree_id);
        }
        Ok(())
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }

    fn record_from_request(request: &SpawnAgentRequest, pty_id: Option<String>) -> AgentRecord {
        AgentRecord {
            id: request.id.trim_start_matches("pty-").to_string(),
            name: request.name.clone(),
            provider: request.provider,
            role: request.role,
            description: request.description.clone(),
            cwd: request.cwd.clone(),
            command: request.command.clone(),
            args: request.args.clone(),
            model: request.model.clone(),
            status: AgentStatus::Starting,
            action_ja: String::from("起動中"),
            pty_id,
            worktree_path: None,
            workspace_capability: None,
            session_id: request.resume_session_id.clone(),
            archived: false,
        }
    }

    pub(super) async fn socket(ws: WebSocketUpgrade) -> Response {
        ws.on_upgrade(run_socket)
    }

    async fn run_socket(socket: WebSocket) {
        let (mut sender, mut receiver) = socket.split();
        let mut shutdown = super::super::shutdown_receiver();
        let mut stream_reset = super::super::stream_reset_receiver();
        let mut attached = HashMap::<String, u64>::new();
        let mut interval = tokio::time::interval(Duration::from_millis(40));
        loop {
            tokio::select! {
                incoming = receiver.next() => {
                    let Some(Ok(Message::Text(text))) = incoming else { break };
                    let Ok(frame) = serde_json::from_str::<TerminalClientFrame>(&text) else { continue };
                    match &frame {
                        TerminalClientFrame::Attach { pty_id, after_seq } => {
                            attached.insert(pty_id.clone(), *after_seq);
                        }
                        TerminalClientFrame::Detach { pty_id } => {
                            attached.remove(pty_id);
                        }
                        TerminalClientFrame::Input { pty_id, data } => {
                            let _ = input(pty_id, data).await;
                            continue;
                        }
                        TerminalClientFrame::Presence { pty_id, presence } => {
                            if let Ok(rows) = agents().lock()
                                && let Some(record) = rows
                                    .values()
                                    .find(|state| state.record.pty_id.as_deref() == Some(pty_id))
                                    .map(|state| state.record.clone())
                            {
                                let _ = super::super::office_agent_activity(
                                    &record.id,
                                    record.status,
                                    &record.action_ja,
                                    None,
                                    presence.blocks_automation(),
                                );
                            }
                        }
                        _ => {}
                    }
                    let frames = TerminalFrameRouter::new(registry()).route(frame);
                    if send_frames(&mut sender, &mut attached, frames).await.is_err() { break; }
                }
                _ = interval.tick() => {
                    let targets = attached.clone();
                    for (pty_id, after_seq) in targets {
                        let Ok(frames) = registry().drain_frames(&pty_id, after_seq) else { continue };
                        if send_frames(&mut sender, &mut attached, frames).await.is_err() { return; }
                    }
                }
                _ = shutdown.changed() => break,
                _ = stream_reset.changed() => break,
            }
        }
    }

    async fn send_frames(
        sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
        attached: &mut HashMap<String, u64>,
        frames: Vec<TerminalServerFrame>,
    ) -> Result<(), ()> {
        for frame in frames {
            match &frame {
                TerminalServerFrame::Output { pty_id, seq, .. }
                | TerminalServerFrame::Relaunching { pty_id, seq, .. } => {
                    attached.insert(pty_id.clone(), *seq);
                    if let Ok(rows) = agents().lock()
                        && let Some(agent_id) = rows
                            .values()
                            .find(|state| state.record.pty_id.as_deref() == Some(pty_id))
                            .map(|state| state.record.id.clone())
                    {
                        let _ = super::super::office_agent_activity(
                            &agent_id,
                            AgentStatus::Working,
                            "出力中",
                            None,
                            false,
                        );
                    }
                }
                TerminalServerFrame::Exited {
                    seq,
                    pty_id,
                    generation,
                    exit,
                } => {
                    let _ = mark_natural_exit(pty_id, *generation, exit.exit_code).await;
                    attached.insert(pty_id.clone(), *seq);
                }
                _ => {}
            }
            let text = serde_json::to_string(&frame).map_err(|_| ())?;
            sender
                .send(Message::Text(text.into()))
                .await
                .map_err(|_| ())?;
        }
        Ok(())
    }
}

#[cfg_attr(
    not(feature = "server"),
    expect(dead_code, reason = "Dioxus replaces web server-function bodies")
)]
fn safe_error() -> ServerFnError {
    ServerFnError::new("ターミナル操作に失敗しました")
}

#[cfg(feature = "server")]
pub(crate) fn running_terminal_count() -> Result<u32, ServerFnError> {
    let count = server::registry().list().map_err(|_| safe_error())?.len();
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

#[cfg(feature = "server")]
pub(crate) async fn workspace_private_capabilities()
-> Result<Vec<md_web_contracts::domains::fs_git_ide::PrivateWorkspaceCapability>, ServerFnError> {
    server::workspace_private_capabilities()
        .await
        .map_err(|_| safe_error())
}

#[cfg(feature = "server")]
pub(crate) async fn shutdown_all() -> Result<(), ServerFnError> {
    server::shutdown_all().await.map_err(|_| safe_error())
}

#[cfg(feature = "server")]
pub(crate) async fn prepare_namespace_reset() -> Result<(), ServerFnError> {
    server::prepare_namespace_reset()
        .await
        .map_err(|_| safe_error())
}

#[cfg(feature = "server")]
pub(crate) fn finish_namespace_reset(committed: bool) -> Result<(), ServerFnError> {
    server::finish_namespace_reset(committed).map_err(|_| safe_error())
}

#[get("/api/agents")]
pub(crate) async fn list_agents() -> Result<(Vec<AgentRecord>, Vec<AgentRecord>), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::list().await.map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/agents/restore")]
pub(crate) async fn pty_restore(
    request: RestoreAgentRequest,
) -> Result<SpawnAgentResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::restore(request).await.map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/agents/unarchive")]
pub(crate) async fn pty_unarchive(agent_id: String) -> Result<AgentRecord, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::unarchive(&agent_id).await.map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = agent_id;
        Err(safe_error())
    }
}

#[post("/api/agents/spawn")]
pub(crate) async fn pty_spawn(
    request: SpawnAgentRequest,
) -> Result<SpawnAgentResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::spawn(request).await.map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/agents/kill")]
pub(crate) async fn pty_kill(pty_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::kill(&pty_id).await.map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = pty_id;
        Err(safe_error())
    }
}

#[post("/api/agents/restart")]
pub(crate) async fn pty_restart(
    request: RestartAgentRequest,
) -> Result<SpawnAgentResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::restart(request).await.map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/pty/input")]
pub(crate) async fn pty_input(pty_id: String, data: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::input(&pty_id, &data)
            .await
            .map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (pty_id, data);
        Err(safe_error())
    }
}

#[post("/api/pty/queue")]
pub(crate) async fn pty_queue(agent_id: String, text: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::queue(&agent_id, &text)
            .await
            .map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (agent_id, text);
        Err(safe_error())
    }
}

#[cfg(feature = "server")]
pub(crate) async fn pty_queue_system(agent_id: &str, text: &str) -> Result<(), ServerFnError> {
    server::queue_system(agent_id, text)
        .await
        .map_err(|_| safe_error())
}

#[cfg(feature = "server")]
pub(crate) async fn enforce_agent_token_cap(
    sample: &md_web_contracts::domains::memory_skills::AgentUsageSample,
) -> Result<(), ServerFnError> {
    let repository = super::persistence_repository().await?;
    let config = md_web_services::domains::config_onboarding::load_config(&repository)
        .await
        .map_err(|_| safe_error())?;
    let Some(cap) = config.agent_token_caps.get(&sample.agent_id).copied() else {
        return Ok(());
    };
    let consumed = sample
        .input_tokens
        .saturating_add(sample.output_tokens)
        .saturating_add(sample.cache_read_tokens)
        .saturating_add(sample.cache_creation_tokens);
    if consumed < cap {
        return Ok(());
    }
    let pty_id = server::list()
        .await
        .map_err(|_| safe_error())?
        .0
        .into_iter()
        .find(|record| record.id == sample.agent_id)
        .and_then(|record| record.pty_id)
        .ok_or_else(safe_error)?;
    server::kill(&pty_id).await.map_err(|_| safe_error())
}

#[post("/api/pty/resize")]
pub(crate) async fn pty_resize(
    pty_id: String,
    dimensions: PtyDimensions,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::registry()
            .resize(&pty_id, dimensions)
            .map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (pty_id, dimensions);
        Err(safe_error())
    }
}

#[post("/api/pty/redraw")]
pub(crate) async fn pty_redraw(pty_id: String) -> Result<PtyDimensions, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::registry().redraw(&pty_id).map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = pty_id;
        Err(safe_error())
    }
}

#[cfg(feature = "server")]
pub(crate) async fn agent_hook(
    request: dioxus::server::axum::extract::Json<
        md_web_contracts::domains::pty_agents::AgentHookRequest,
    >,
) -> Result<
    dioxus::server::axum::extract::Json<md_web_contracts::domains::pty_agents::AgentHookDecision>,
    dioxus::server::axum::http::StatusCode,
> {
    server::agent_hook(request).await
}

#[cfg(feature = "server")]
pub(crate) async fn provider_agent_hook(
    provider: dioxus::server::axum::extract::Path<String>,
    headers: dioxus::server::axum::http::HeaderMap,
    body: dioxus::server::axum::body::Bytes,
) -> Result<
    dioxus::server::axum::extract::Json<serde_json::Value>,
    dioxus::server::axum::http::StatusCode,
> {
    server::provider_agent_hook(provider, headers, body).await
}

#[cfg(feature = "server")]
pub(crate) async fn terminal_socket(
    ws: dioxus::server::axum::extract::ws::WebSocketUpgrade,
) -> dioxus::server::axum::response::Response {
    server::socket(ws).await
}
