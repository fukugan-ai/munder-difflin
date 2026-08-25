use dioxus::prelude::*;
#[cfg(feature = "server")]
use md_web_contracts::domains::hive_tasks::HiveHookDecision;
use md_web_contracts::domains::hive_tasks::{
    AgentControlSnapshot, HiveEventEnvelope, HiveMessage, HiveSnapshot, HiveTask,
    PreservedWorktreeSnapshot, TaskStatus, WorkerSnapshot, WorkerTeardownReceipt,
};
#[cfg(feature = "server")]
use md_web_contracts::domains::pty_agents::AgentHookRequest;
use serde_json::{Map, Value};

#[cfg(feature = "server")]
mod server {
    use std::collections::VecDeque;
    use std::convert::Infallible;
    use std::ops::Deref;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, LazyLock};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use dioxus::prelude::ServerFnError;
    use dioxus::server::axum::http::{HeaderMap, StatusCode};
    use dioxus::server::axum::response::sse::{Event, KeepAlive, Sse};
    use dioxus::server::axum::response::{IntoResponse, Response};
    use futures_util::stream;
    use md_web_contracts::domains::hive_tasks::{HiveDomainEvent, HiveEventEnvelope};
    use md_web_contracts::domains::persistence::{ReplayEventWrite, ReplayPage};
    use md_web_services::domains::hive_tasks::{HiveServiceError, HiveTasksService};

    const EVENT_CAPACITY: usize = 512;
    const MAX_WORKERS: usize = 4;
    const MESSAGE_LIMIT: usize = 200;

    pub(super) struct HiveRuntime {
        root: PathBuf,
        service: HiveTasksService,
        local_event_cursor: AtomicU64,
    }

    impl Deref for HiveRuntime {
        type Target = HiveTasksService;

        fn deref(&self) -> &Self::Target {
            &self.service
        }
    }

    static SERVICE: LazyLock<tokio::sync::RwLock<Option<Arc<HiveRuntime>>>> =
        LazyLock::new(|| tokio::sync::RwLock::new(None));
    const EVENT_STREAM: &str = "hive";

    pub(super) async fn service() -> Result<Arc<HiveRuntime>, ServerFnError> {
        let repository = super::super::persistence_repository()
            .await
            .map_err(|_| ServerFnError::new("Hive PostgreSQL runtime is unavailable"))?;
        let config = md_web_services::domains::config_onboarding::load_config(&repository)
            .await
            .map_err(|_| ServerFnError::new("Hive configuration is unavailable"))?;
        let home = PathBuf::from(
            config
                .harness_home
                .ok_or_else(|| ServerFnError::new("Hive harness home is not configured"))?,
        );
        if !home.is_absolute() {
            return Err(ServerFnError::new("Hive harness home is invalid"));
        }
        let root = home.join("hive");
        let mut cached = SERVICE.write().await;
        if let Some(runtime) = cached.as_ref().filter(|runtime| runtime.root == root) {
            return Ok(Arc::clone(runtime));
        }
        let runtime = Arc::new(HiveRuntime {
            root: root.clone(),
            service: HiveTasksService::new(root, EVENT_CAPACITY, MAX_WORKERS)
                .map_err(|_| ServerFnError::new("Hive service is unavailable"))?,
            local_event_cursor: AtomicU64::new(0),
        });
        *cached = Some(Arc::clone(&runtime));
        Ok(runtime)
    }

    pub(super) async fn clear_service() {
        *SERVICE.write().await = None;
    }

    pub(super) fn now() -> Result<(i64, String), ()> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| ())?;
        let milliseconds = i64::try_from(duration.as_millis()).map_err(|_| ())?;
        Ok((milliseconds, format!("{milliseconds}")))
    }

    pub(super) fn map_error<T>(result: Result<T, HiveServiceError>) -> Result<T, ServerFnError> {
        result.map_err(|_| ServerFnError::new("Hive operation failed"))
    }

    pub(super) async fn flush_events(runtime: &HiveRuntime) -> Result<(), ServerFnError> {
        let mut cursor = runtime.local_event_cursor.load(Ordering::Acquire);
        let batch = map_error(runtime.replay_after(cursor))?;
        if batch.gap {
            return Err(ServerFnError::new("Hive event buffer gap requires restart"));
        }
        let repository = super::super::persistence_repository()
            .await
            .map_err(|_| ServerFnError::new("Hive PostgreSQL runtime is unavailable"))?;
        for envelope in batch.events {
            let payload_json = serde_json::to_string(&envelope.event)
                .map_err(|_| ServerFnError::new("Hive event is invalid"))?;
            let event_id = event_uuid(&envelope)?;
            repository
                .append_replay_event(&ReplayEventWrite {
                    stream: String::from(EVENT_STREAM),
                    event_id,
                    occurred_at_ms: envelope.ts_ms,
                    payload_json,
                })
                .await
                .map_err(|_| ServerFnError::new("Hive event persistence failed"))?;
            if let HiveDomainEvent::MessageRouted { message, targets } = &envelope.event {
                let act = match message.act {
                    md_web_contracts::domains::hive_tasks::MessageAct::Request => {
                        md_web_contracts::domains::office_ui::MessageAct::Request
                    }
                    md_web_contracts::domains::hive_tasks::MessageAct::Inform => {
                        md_web_contracts::domains::office_ui::MessageAct::Inform
                    }
                    md_web_contracts::domains::hive_tasks::MessageAct::Propose => {
                        md_web_contracts::domains::office_ui::MessageAct::Propose
                    }
                    md_web_contracts::domains::hive_tasks::MessageAct::Query => {
                        md_web_contracts::domains::office_ui::MessageAct::Query
                    }
                    md_web_contracts::domains::hive_tasks::MessageAct::Agree => {
                        md_web_contracts::domains::office_ui::MessageAct::Agree
                    }
                    md_web_contracts::domains::hive_tasks::MessageAct::Refuse => {
                        md_web_contracts::domains::office_ui::MessageAct::Refuse
                    }
                    md_web_contracts::domains::hive_tasks::MessageAct::Done => {
                        md_web_contracts::domains::office_ui::MessageAct::Done
                    }
                };
                super::super::office::office_live_update(
                    md_web_contracts::domains::office_ui::OfficeLiveUpdate::Handoff(
                        md_web_contracts::domains::office_ui::HiveHandoff {
                            event_id: message.id.clone(),
                            sequence: envelope.seq,
                            from: message.from.clone(),
                            targets: targets.clone(),
                            act,
                            needs_human: message.needs_human,
                        },
                    ),
                )
                .await?;
            }
            cursor = envelope.seq;
            runtime.local_event_cursor.store(cursor, Ordering::Release);
        }
        Ok(())
    }

    pub(super) async fn flush_events_retry(runtime: &HiveRuntime) -> Result<(), ServerFnError> {
        let mut last_error = None;
        for attempt in 0..3 {
            match flush_events(runtime).await {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Err(last_error.unwrap_or_else(|| ServerFnError::new("Hive event persistence failed")))
    }

    pub(super) async fn replay(
        after: u64,
    ) -> Result<(bool, Vec<HiveEventEnvelope>), ServerFnError> {
        let repository = super::super::persistence_repository()
            .await
            .map_err(|_| ServerFnError::new("Hive PostgreSQL runtime is unavailable"))?;
        let page = repository
            .replay_after(EVENT_STREAM, after, 500)
            .await
            .map_err(|_| ServerFnError::new("Hive event replay failed"))?;
        decode_replay_page(page)
    }

    fn decode_replay_page(
        page: ReplayPage,
    ) -> Result<(bool, Vec<HiveEventEnvelope>), ServerFnError> {
        let events = page
            .events
            .into_iter()
            .map(|event| {
                let domain_event = serde_json::from_str(&event.payload_json)
                    .map_err(|_| ServerFnError::new("Hive event replay is invalid"))?;
                Ok(HiveEventEnvelope {
                    seq: event.sequence,
                    ts_ms: event.occurred_at_ms,
                    event: domain_event,
                })
            })
            .collect::<Result<Vec<_>, ServerFnError>>()?;
        Ok((page.gap, events))
    }

    fn event_uuid(envelope: &HiveEventEnvelope) -> Result<String, ServerFnError> {
        let time = u64::from_ne_bytes(envelope.ts_ms.to_ne_bytes());
        let high = u128::from(time) << 64;
        let event = serde_json::to_vec(&envelope.event)
            .map_err(|_| ServerFnError::new("Hive event is invalid"))?;
        let content_hash = event.iter().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
        let value = high | u128::from(envelope.seq ^ content_hash);
        let hex = format!("{value:032x}");
        Ok(format!(
            "{}-{}-4{}-8{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[13..16],
            &hex[17..20],
            &hex[20..32]
        ))
    }

    pub(super) async fn event_stream(headers: HeaderMap) -> Response {
        if service().await.is_err() {
            return (StatusCode::SERVICE_UNAVAILABLE, "Hive service unavailable").into_response();
        }
        let cursor = headers
            .get("last-event-id")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let interval = tokio::time::interval(Duration::from_millis(750));
        let shutdown = super::super::shutdown_receiver();
        let stream_reset = super::super::stream_reset_receiver();
        let stream = stream::unfold(
            (
                cursor,
                VecDeque::<HiveEventEnvelope>::new(),
                interval,
                false,
                shutdown,
                stream_reset,
            ),
            move |(
                mut cursor,
                mut pending,
                mut interval,
                mut reset,
                mut shutdown,
                mut stream_reset,
            )| async move {
                loop {
                    if *shutdown.borrow() {
                        return None;
                    }
                    if reset {
                        reset = false;
                        let event = Event::default().event("hive-reset").data("{\"gap\":true}");
                        return Some((
                            Ok::<Event, Infallible>(event),
                            (cursor, pending, interval, reset, shutdown, stream_reset),
                        ));
                    }
                    if let Some(envelope) = pending.pop_front() {
                        cursor = envelope.seq;
                        let Ok(data) = serde_json::to_string(&envelope) else {
                            continue;
                        };
                        let event = Event::default()
                            .event("hive")
                            .id(envelope.seq.to_string())
                            .data(data);
                        return Some((
                            Ok::<Event, Infallible>(event),
                            (cursor, pending, interval, reset, shutdown, stream_reset),
                        ));
                    }
                    tokio::select! {
                        _ = interval.tick() => {},
                        _ = shutdown.changed() => return None,
                        _ = stream_reset.changed() => return None,
                    }
                    let Ok((gap, events)) = replay(cursor).await else {
                        continue;
                    };
                    reset = gap;
                    pending = VecDeque::from(events);
                }
            },
        );
        Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
            .into_response()
    }

    pub(super) const fn message_limit() -> usize {
        MESSAGE_LIMIT
    }

    #[cfg(test)]
    mod tests {
        use md_web_contracts::domains::hive_tasks::{HiveDomainEvent, HiveEventEnvelope};
        use md_web_contracts::domains::persistence::{ReplayEvent, ReplayPage};

        use super::{EVENT_STREAM, ServerFnError, decode_replay_page, event_uuid};

        #[test]
        fn durable_replay_restores_post_restart_sequence() -> Result<(), ServerFnError> {
            let domain_event = HiveDomainEvent::TaskDeleted {
                task_id: String::from("task-41"),
            };
            let page = ReplayPage {
                gap: false,
                events: vec![ReplayEvent {
                    stream: String::from(EVENT_STREAM),
                    sequence: 41,
                    event_id: String::from("00000000-0000-4000-8000-000000000029"),
                    occurred_at_ms: 1_725_000_000_000,
                    payload_json: serde_json::to_string(&domain_event)
                        .map_err(|_| ServerFnError::new("serialize event"))?,
                }],
            };

            let (gap, restored) = decode_replay_page(page)?;

            assert!(!gap);
            assert_eq!(restored.len(), 1);
            assert_eq!(restored[0].seq, 41);
            assert_eq!(restored[0].event, domain_event);
            Ok(())
        }

        #[test]
        fn event_id_is_stable_for_retry() -> Result<(), ServerFnError> {
            let envelope = HiveEventEnvelope {
                seq: 7,
                ts_ms: 123,
                event: HiveDomainEvent::TaskDeleted {
                    task_id: String::from("task-7"),
                },
            };

            let first_id = event_uuid(&envelope)?;
            let retry_id = event_uuid(&envelope)?;
            assert_eq!(first_id, retry_id);

            let different = HiveEventEnvelope {
                seq: envelope.seq,
                ts_ms: envelope.ts_ms,
                event: HiveDomainEvent::TaskDeleted {
                    task_id: String::from("task-8"),
                },
            };
            assert_ne!(event_uuid(&envelope)?, event_uuid(&different)?);
            Ok(())
        }
    }
}

#[cfg_attr(
    not(feature = "server"),
    expect(
        dead_code,
        reason = "Dioxus replaces Server Function bodies in web builds"
    )
)]
fn safe_error() -> ServerFnError {
    ServerFnError::new("Hive service is unavailable")
}

#[get("/api/hive/snapshot")]
pub(crate) async fn hive_snapshot(
    selected_agent_id: Option<String>,
) -> Result<HiveSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let snapshot = server::map_error(
            server::service()
                .await?
                .snapshot(selected_agent_id.as_deref(), server::message_limit()),
        )?;
        let tasks = snapshot
            .tasks
            .iter()
            .map(|task| md_web_contracts::domains::office_ui::OfficeTask {
                id: task.id.clone(),
                status: match task.status {
                    md_web_contracts::domains::hive_tasks::TaskStatus::Todo => {
                        md_web_contracts::domains::office_ui::TaskStatus::Todo
                    }
                    md_web_contracts::domains::hive_tasks::TaskStatus::Doing => {
                        md_web_contracts::domains::office_ui::TaskStatus::Doing
                    }
                    md_web_contracts::domains::hive_tasks::TaskStatus::Blocked => {
                        md_web_contracts::domains::office_ui::TaskStatus::Blocked
                    }
                    md_web_contracts::domains::hive_tasks::TaskStatus::Done => {
                        md_web_contracts::domains::office_ui::TaskStatus::Done
                    }
                },
                assignee: task.assignee.clone(),
                has_unanswered_human_qa: task.open_question().is_some(),
            })
            .collect();
        super::office::office_live_update(
            md_web_contracts::domains::office_ui::OfficeLiveUpdate::ReplaceTasks { tasks },
        )
        .await?;
        Ok(snapshot)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = selected_agent_id;
        Err(safe_error())
    }
}

#[get("/api/hive/inbox")]
pub(crate) async fn hive_inbox(agent_id: String) -> Result<Vec<HiveMessage>, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::map_error(server::service().await?.inbox(&agent_id))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = agent_id;
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/add")]
pub(crate) async fn hive_add_task(task: HiveTask) -> Result<HiveTask, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let task = server::map_error(service.add_task(task, ts_ms))?;
        server::flush_events(&service).await?;
        Ok(task)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = task;
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/create")]
pub(crate) async fn hive_create_task(
    title: String,
    description: Option<String>,
    assignee: Option<String>,
    priority: i32,
) -> Result<HiveTask, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, created_at) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let task = server::map_error(service.create_task(
            &title,
            description,
            assignee,
            priority,
            &created_at,
            ts_ms,
        ))?;
        server::flush_events(&service).await?;
        Ok(task)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (title, description, assignee, priority);
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/patch")]
pub(crate) async fn hive_patch_task(
    task_id: String,
    patch: Map<String, Value>,
) -> Result<HiveTask, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let task = server::map_error(service.patch_task(&task_id, &patch, ts_ms))?;
        server::flush_events(&service).await?;
        Ok(task)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (task_id, patch);
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/move")]
pub(crate) async fn hive_move_task(
    task_id: String,
    status: TaskStatus,
) -> Result<HiveTask, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let task = server::map_error(service.move_task(&task_id, status, ts_ms))?;
        server::flush_events(&service).await?;
        Ok(task)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (task_id, status);
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/delete")]
pub(crate) async fn hive_delete_task(task_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        server::map_error(service.delete_task(&task_id, ts_ms))?;
        server::flush_events(&service).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = task_id;
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/answer")]
pub(crate) async fn hive_answer_question(
    task_id: String,
    answer: String,
) -> Result<HiveTask, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, created_at) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let task =
            server::map_error(service.answer_question(&task_id, &answer, &created_at, ts_ms))?;
        server::flush_events(&service).await?;
        Ok(task)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (task_id, answer);
        Err(safe_error())
    }
}

#[post("/api/hive/tasks/dismiss-question")]
pub(crate) async fn hive_dismiss_question(task_id: String) -> Result<HiveTask, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, created_at) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let task = server::map_error(service.dismiss_question(&task_id, &created_at, ts_ms))?;
        server::flush_events(&service).await?;
        Ok(task)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = task_id;
        Err(safe_error())
    }
}

#[post("/api/hive/messages/send")]
pub(crate) async fn hive_send(
    message: HiveMessage,
) -> Result<(Vec<String>, Vec<String>), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let outcome = server::map_error(service.send(&message, ts_ms))?;
        server::flush_events(&service).await?;
        Ok((outcome.delivered, outcome.unknown))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = message;
        Err(safe_error())
    }
}

#[post("/api/hive/messages/reply")]
pub(crate) async fn hive_reply(
    conversation: String,
    body: String,
) -> Result<(Vec<String>, Vec<String>), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, created_at) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let outcome =
            server::map_error(service.reply_to_god(&conversation, &body, &created_at, ts_ms))?;
        server::flush_events(&service).await?;
        Ok((outcome.delivered, outcome.unknown))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (conversation, body);
        Err(safe_error())
    }
}

#[post("/api/hive/messages/thread")]
pub(crate) async fn hive_new_thread(
    subject: String,
    body: String,
) -> Result<(Vec<String>, Vec<String>), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, created_at) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let outcome = server::map_error(service.new_thread(&subject, &body, &created_at, ts_ms))?;
        server::flush_events(&service).await?;
        Ok((outcome.delivered, outcome.unknown))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (subject, body);
        Err(safe_error())
    }
}

#[post("/api/hive/agents/role")]
pub(crate) async fn hive_patch_role(agent_id: String, role: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    return server::map_error(server::service().await?.patch_role(&agent_id, &role));
    #[cfg(not(feature = "server"))]
    {
        let _ = (agent_id, role);
        Err(safe_error())
    }
}

#[post("/api/hive/agents/hold")]
pub(crate) async fn hive_set_hold(agent_id: String, on: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    return server::map_error(server::service().await?.set_hold(&agent_id, on));
    #[cfg(not(feature = "server"))]
    {
        let _ = (agent_id, on);
        Err(safe_error())
    }
}

#[post("/api/hive/control/pause")]
pub(crate) async fn hive_control_pause(
    agent_id: String,
    on: bool,
) -> Result<AgentControlSnapshot, ServerFnError> {
    control_call(agent_id, ControlCommand::Pause(on)).await
}

#[post("/api/hive/control/auto-delivery")]
pub(crate) async fn hive_control_auto_delivery(
    agent_id: String,
    paused: bool,
) -> Result<AgentControlSnapshot, ServerFnError> {
    control_call(agent_id, ControlCommand::AutoDelivery(paused)).await
}

#[post("/api/hive/control/gate-tool")]
pub(crate) async fn hive_control_gate(
    agent_id: String,
    tool: String,
    on: bool,
) -> Result<AgentControlSnapshot, ServerFnError> {
    control_call(agent_id, ControlCommand::GateTool { tool, on }).await
}

#[post("/api/hive/control/resume")]
pub(crate) async fn hive_control_resume(
    agent_id: String,
) -> Result<AgentControlSnapshot, ServerFnError> {
    control_call(agent_id, ControlCommand::Resume).await
}

#[post("/api/hive/control/steer")]
pub(crate) async fn hive_control_steer(
    agent_id: String,
    text: String,
) -> Result<AgentControlSnapshot, ServerFnError> {
    control_call(agent_id, ControlCommand::Steer(text)).await
}

#[post("/api/hive/control/halt")]
pub(crate) async fn hive_control_halt(
    agent_id: String,
) -> Result<AgentControlSnapshot, ServerFnError> {
    control_call(agent_id, ControlCommand::Halt).await
}

#[cfg_attr(
    not(feature = "server"),
    expect(
        dead_code,
        reason = "Dioxus replaces Server Function bodies in web builds"
    )
)]
enum ControlCommand {
    Pause(bool),
    AutoDelivery(bool),
    GateTool { tool: String, on: bool },
    Resume,
    Steer(String),
    Halt,
}

#[cfg_attr(
    not(feature = "server"),
    expect(
        dead_code,
        reason = "Dioxus replaces Server Function bodies in web builds"
    )
)]
async fn control_call(
    agent_id: String,
    command: ControlCommand,
) -> Result<AgentControlSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        let snapshot = server::map_error(match command {
            ControlCommand::Pause(on) => service.pause(&agent_id, on, ts_ms),
            ControlCommand::AutoDelivery(on) => service.pause_auto_delivery(&agent_id, on, ts_ms),
            ControlCommand::GateTool { tool, on } => service.gate_tool(&agent_id, &tool, on, ts_ms),
            ControlCommand::Resume => service.resume(&agent_id, ts_ms),
            ControlCommand::Steer(text) => service.steer(&agent_id, &text, ts_ms),
            ControlCommand::Halt => service.halt(&agent_id, ts_ms),
        })?;
        server::flush_events(&service).await?;
        Ok(snapshot)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (agent_id, command);
        Err(safe_error())
    }
}

#[get("/api/hive/workers")]
pub(crate) async fn hive_workers()
-> Result<(Vec<WorkerSnapshot>, Vec<PreservedWorktreeSnapshot>, usize), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::map_error(server::service().await?.workers_snapshot())
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/hive/workers/stop")]
pub(crate) async fn hive_stop_worker(
    worker_id: String,
) -> Result<WorkerTeardownReceipt, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
        let service = server::service().await?;
        server::map_error(service.stop_worker(&worker_id, ts_ms))?;
        server::flush_events_retry(&service).await?;
        let (active, _) = super::pty::list_agents().await?;
        let agent = active
            .into_iter()
            .find(|agent| agent.id == worker_id)
            .ok_or_else(|| ServerFnError::new("Worker PTY is unavailable"))?;
        let pty_id = agent
            .pty_id
            .ok_or_else(|| ServerFnError::new("Worker PTY is unavailable"))?;
        super::pty::pty_kill(pty_id).await?;
        let receipt = server::map_error(service.complete_worker_stop(
            &worker_id,
            agent.worktree_path,
            ts_ms,
        ))?;
        server::flush_events_retry(&service).await?;
        Ok(receipt)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = worker_id;
        Err(safe_error())
    }
}

/// Shared PTY spawn adapter: registers the worker in the same Hive projection.
#[cfg(feature = "server")]
pub(crate) async fn hive_register_worker_projection(
    worker: WorkerSnapshot,
) -> Result<(), ServerFnError> {
    let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
    let service = server::service().await?;
    server::map_error(service.register_worker(worker, ts_ms))?;
    server::flush_events(&service).await
}

/// Shared PTY/tool hook adapter: consumes pause/halt/gate/steer at execution time.
#[cfg(feature = "server")]
pub(crate) async fn hive_control_hook_decision(
    agent_id: &str,
    tool: Option<&str>,
) -> Result<HiveHookDecision, ServerFnError> {
    server::map_error(server::service().await?.hook_decision(agent_id, tool))
}

/// Verified CLI-hook adapter. Capability and arbitrary payload never enter Hive persistence.
#[cfg(feature = "server")]
pub(crate) async fn hive_agent_hook_event(
    request: &AgentHookRequest,
) -> Result<HiveHookDecision, ServerFnError> {
    let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
    let service = server::service().await?;
    let decision = server::map_error(service.process_agent_hook(
        &request.agent_id,
        &request.event_id,
        request.event,
        request.tool_name.as_deref(),
        ts_ms,
    ))?;
    server::flush_events(&service).await?;
    Ok(decision)
}

/// Scheduler adapter for durable task enqueue through the canonical Hive producer.
#[cfg(feature = "server")]
pub(crate) async fn hive_scheduler_enqueue_task(
    title: &str,
    description: Option<String>,
    assignee: Option<String>,
    priority: i32,
) -> Result<HiveTask, ServerFnError> {
    let (ts_ms, created_at) = server::now().map_err(|_| safe_error())?;
    let service = server::service().await?;
    let task = server::map_error(service.create_task(
        title,
        description,
        assignee,
        priority,
        &created_at,
        ts_ms,
    ))?;
    server::flush_events(&service).await?;
    Ok(task)
}

/// Scheduler adapter for normalized messages without a browser round trip.
#[cfg(feature = "server")]
pub(crate) async fn hive_scheduler_enqueue_message(
    message: &HiveMessage,
) -> Result<(), ServerFnError> {
    let (ts_ms, _) = server::now().map_err(|_| safe_error())?;
    let service = server::service().await?;
    server::map_error(service.send(message, ts_ms))?;
    server::flush_events(&service).await
}

#[get("/api/hive/events/replay")]
pub(crate) async fn hive_events_replay(
    after: u64,
) -> Result<(bool, Vec<HiveEventEnvelope>), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::replay(after).await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = after;
        Err(safe_error())
    }
}

#[cfg(feature = "server")]
pub(crate) async fn hive_event_stream(
    headers: dioxus::server::axum::http::HeaderMap,
) -> dioxus::server::axum::response::Response {
    server::event_stream(headers).await
}

/// Invalidates the keyed Hive runtime after a committed harness-home change.
#[cfg(feature = "server")]
pub(crate) async fn hive_reinitialize_harness_home() {
    server::clear_service().await;
}
