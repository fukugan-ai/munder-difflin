use dioxus::prelude::*;
#[cfg(feature = "server")]
use md_web_contracts::domains::connections::ConnectionOperationStatus;

#[cfg(feature = "server")]
const CONNECTIONS_RECORD_KIND: &str = "state";
#[cfg(feature = "server")]
const CONNECTIONS_RECORD_ID: &str = "main";

#[cfg(feature = "server")]
static CONNECTIONS_HYDRATED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "server")]
static CONNECTIONS_RESETTING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(feature = "server")]
static CONNECTIONS_HYDRATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(feature = "server")]
static CONNECTIONS_PERSISTENCE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(feature = "server")]
struct AutomationRuntime {
    stop: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "server")]
type BrokerHandler = std::sync::Arc<
    dyn Fn(
            md_web_services::domains::connections::HttpRequest,
        ) -> md_web_services::domains::connections::HttpResponse
        + Send
        + Sync,
>;

#[cfg(feature = "server")]
static AUTOMATION_RUNTIME: tokio::sync::Mutex<Option<AutomationRuntime>> =
    tokio::sync::Mutex::const_new(None);

#[cfg(feature = "server")]
fn connections_record_key() -> md_web_contracts::domains::persistence::RecordKey {
    md_web_contracts::domains::persistence::RecordKey {
        domain: md_web_contracts::domains::persistence::RecordDomain::Connections,
        kind: String::from(CONNECTIONS_RECORD_KIND),
        record_id: String::from(CONNECTIONS_RECORD_ID),
    }
}

#[cfg(feature = "server")]
fn master_key() -> Result<Option<Vec<u8>>, ServerFnError> {
    let Some(value) = std::env::var_os("MD_CONNECTIONS_MASTER_KEY") else {
        return Ok(None);
    };
    let bytes = value.to_string_lossy().into_owned().into_bytes();
    if bytes.len() < 32 {
        return Err(ServerFnError::new(
            "MD_CONNECTIONS_MASTER_KEY must contain at least 32 bytes",
        ));
    }
    Ok(Some(bytes))
}

#[cfg(feature = "server")]
fn require_master_key() -> Result<Vec<u8>, ServerFnError> {
    master_key()?
        .ok_or_else(|| ServerFnError::new("秘密情報の保存にはMD_CONNECTIONS_MASTER_KEYが必要です"))
}

#[cfg(feature = "server")]
async fn ensure_connections_hydrated() -> Result<(), ServerFnError> {
    use std::sync::atomic::Ordering;

    if CONNECTIONS_RESETTING.load(Ordering::Acquire) {
        return Err(ServerFnError::new("接続設定をリセット中です"));
    }
    if CONNECTIONS_HYDRATED.load(Ordering::Acquire) {
        return Ok(());
    }
    let _guard = CONNECTIONS_HYDRATION_LOCK.lock().await;
    if CONNECTIONS_RESETTING.load(Ordering::Acquire) {
        return Err(ServerFnError::new("接続設定をリセット中です"));
    }
    if CONNECTIONS_HYDRATED.load(Ordering::Acquire) {
        return Ok(());
    }
    if hydrate_connections_projection().await.is_err() {
        let _ = service().reset_projection();
        return Err(ServerFnError::new("接続設定をPostgreSQLから復元できません"));
    }
    CONNECTIONS_HYDRATED.store(true, Ordering::Release);
    start_automation_executor().await;
    Ok(())
}

#[cfg(feature = "server")]
async fn hydrate_connections_projection() -> Result<(), String> {
    let repository = super::persistence_repository()
        .await
        .map_err(|_| String::from("PostgreSQL persistence unavailable"))?;
    let record = repository
        .get_record(&connections_record_key())
        .await
        .map_err(|_| String::from("connections load failed"))?;
    if let Some(record) = record {
        let envelope: serde_json::Value = serde_json::from_str(&record.payload_json)
            .map_err(|_| String::from("connections durable envelope is invalid"))?;
        let metadata = envelope
            .get("metadata")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| String::from("connections metadata is missing"))?;
        let plan = service()
            .hydrate_durable_metadata(metadata)
            .map_err(|_| String::from("connections metadata hydration failed"))?;
        if let Some(sealed) = envelope
            .get("sealed_secrets")
            .and_then(serde_json::Value::as_str)
        {
            let key = std::env::var("MD_CONNECTIONS_MASTER_KEY")
                .map_err(|_| String::from("connections master key is missing"))?;
            service()
                .hydrate_encrypted_secrets(key.as_bytes(), sealed)
                .map_err(|_| String::from("connections secret hydration failed"))?;
        }
        if plan.restart_slack {
            service()
                .start_slack_runtime(slack_dispatch())
                .map_err(|_| String::from("Slack listener restart failed"))?;
        }
        if plan.restart_webhooks {
            service()
                .start_webhook_runtime(webhook_dispatch())
                .map_err(|_| String::from("webhook listener restart failed"))?;
        }
        if plan.restart_broker {
            let enabled_ids: Vec<String> = service()
                .snapshot()
                .map_err(|_| String::from("broker metadata unavailable"))?
                .integrations
                .into_iter()
                .filter(|integration| integration.enabled)
                .map(|integration| integration.id)
                .collect();
            if enabled_ids.is_empty() {
                return Err(String::from("broker integrations are missing"));
            }
            service()
                .grant_broker_capability(
                    mint_secret().map_err(|_| String::from("broker capability unavailable"))?,
                    enabled_ids,
                )
                .map_err(|_| String::from("broker capability restart failed"))?;
            service()
                .start_broker_runtime(broker_handler())
                .map_err(|_| String::from("broker listener restart failed"))?;
        }
    }
    Ok(())
}

#[cfg(feature = "server")]
async fn persist_connections() -> Result<(), ServerFnError> {
    ensure_connections_hydrated().await?;
    persist_hydrated_connections().await
}

#[cfg(feature = "server")]
async fn persist_hydrated_connections() -> Result<(), ServerFnError> {
    let _guard = CONNECTIONS_PERSISTENCE_LOCK.lock().await;
    let repository = super::persistence_repository().await?;
    let key = connections_record_key();
    let existing = repository
        .get_record(&key)
        .await
        .map_err(|_| ServerFnError::new("接続設定の永続化に失敗しました"))?;
    let metadata = service()
        .export_durable_metadata()
        .map_err(|_| safe_error())?;
    let sealed_secrets = match (
        service().secret_count().map_err(|_| safe_error())?,
        master_key()?,
    ) {
        (0, None) => None,
        (_, Some(key)) => Some(
            service()
                .export_encrypted_secrets(&key)
                .map_err(|_| safe_error())?,
        ),
        (_, None) => {
            return Err(ServerFnError::new(
                "秘密情報の永続化にはMD_CONNECTIONS_MASTER_KEYが必要です",
            ));
        }
    };
    let payload_json = serde_json::json!({
        "metadata": metadata,
        "sealed_secrets": sealed_secrets,
    })
    .to_string();
    let request = md_web_contracts::domains::persistence::RecordWrite {
        key,
        expected_revision: existing.as_ref().map_or(0, |record| record.revision),
        payload_json,
    };
    repository
        .write_record(&request)
        .await
        .map_err(|_| ServerFnError::new("接続設定の永続化に失敗しました"))?;
    Ok(())
}

/// Server-only bridge for sibling adapters (for example Voice) that share the
/// typed secret provider. This hydrates before reads and never crosses a
/// Server Function/browser boundary.
#[cfg(feature = "server")]
pub(super) async fn hydrated_secret_provider()
-> Result<&'static md_web_services::domains::connections::ConnectionsService, ServerFnError> {
    ensure_connections_hydrated().await?;
    Ok(service())
}

/// Durably seals provider-secret mutations made by sibling server adapters.
#[cfg(feature = "server")]
pub(super) async fn persist_connections_state() -> Result<(), ServerFnError> {
    persist_connections().await
}

#[cfg(feature = "server")]
pub(super) async fn voice_durable_settings()
-> Result<md_web_services::domains::connections::VoiceDurableSettings, ServerFnError> {
    ensure_connections_hydrated().await?;
    service().voice_settings().map_err(|_| safe_error())
}

#[cfg(feature = "server")]
pub(super) async fn update_voice_durable_settings(
    settings: md_web_services::domains::connections::VoiceDurableSettings,
) -> Result<md_web_services::domains::connections::VoiceDurableSettings, ServerFnError> {
    ensure_connections_hydrated().await?;
    let previous = service().voice_settings().map_err(|_| safe_error())?;
    let updated = service()
        .update_voice_settings(settings)
        .map_err(|_| safe_error())?;
    if let Err(error) = persist_hydrated_connections().await {
        let _ = service().update_voice_settings(previous);
        return Err(error);
    }
    Ok(updated)
}

#[cfg(feature = "server")]
async fn start_automation_executor() {
    let _ = super::memory::install_memory_context_usage_provider();
    let mut runtime = AUTOMATION_RUNTIME.lock().await;
    if runtime.is_some() {
        return;
    }
    let (stop, mut stop_receiver) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        let mut shutdown = super::shutdown_receiver();
        loop {
            if *shutdown.borrow() || *stop_receiver.borrow() {
                break;
            }
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown.changed() => break,
                _ = stop_receiver.changed() => break,
            }
            let usage = md_web_services::domains::connections::context_usage_samples();
            let Ok(batch) = service().poll_automations(now_ms(), &usage) else {
                continue;
            };
            for mission in &batch.missions {
                let title = mission.label.clone();
                let body = match mission.kind {
                    md_web_contracts::domains::connections::MissionKind::Dispatch => {
                        mission.body.clone()
                    }
                    md_web_contracts::domains::connections::MissionKind::Heartbeat => {
                        format!("Heartbeat: {}", mission.body)
                    }
                    md_web_contracts::domains::connections::MissionKind::Compact => {
                        mission.body.clone()
                    }
                };
                let _ = dispatch_hive_work("scheduled-mission", title, body).await;
            }
            for event in &batch.events {
                if let md_web_contracts::domains::connections::ConnectionEvent::ContextTriggerDue {
                    action,
                    rule,
                } = event
                {
                    let label = match action {
                        ContextAction::Compact => "context-compact",
                        ContextAction::Clear => "context-clear",
                    };
                    let _ = dispatch_hive_work(label, String::from(label), rule.message.clone()).await;
                }
            }
            if !batch.events.is_empty() {
                let _ = persist_hydrated_connections().await;
            }
        }
        let _ = service().stop_slack_runtime();
        let _ = service().stop_webhook_runtime();
        let _ = service().stop_broker_runtime();
    });
    *runtime = Some(AutomationRuntime { stop, task });
}

#[cfg(feature = "server")]
async fn stop_automation_executor() {
    let runtime = AUTOMATION_RUNTIME.lock().await.take();
    if let Some(runtime) = runtime {
        let _ = runtime.stop.send(true);
        let _ = runtime.task.await;
    }
}

/// Reset transaction preflight: rejects new connection operations, drains the
/// scheduler and transports, and clears all process-local metadata and secrets.
/// The integration owner calls this before the PostgreSQL namespace reset.
#[cfg(feature = "server")]
pub(crate) async fn prepare_connections_namespace_reset() -> Result<(), ServerFnError> {
    use std::sync::atomic::Ordering;

    CONNECTIONS_RESETTING.store(true, Ordering::Release);
    stop_automation_executor().await;
    let _hydration_guard = CONNECTIONS_HYDRATION_LOCK.lock().await;
    let _persistence_guard = CONNECTIONS_PERSISTENCE_LOCK.lock().await;
    service().stop_slack_runtime().map_err(|_| safe_error())?;
    service().stop_webhook_runtime().map_err(|_| safe_error())?;
    service().stop_broker_runtime().map_err(|_| safe_error())?;
    service().reset_projection().map_err(|_| safe_error())?;
    CONNECTIONS_HYDRATED.store(false, Ordering::Release);
    Ok(())
}

/// Reset transaction completion: permits a fresh read of the committed
/// PostgreSQL namespace and restarts the scheduler in the same server process.
#[cfg(feature = "server")]
pub(crate) async fn reinitialize_connections_after_reset() -> Result<(), ServerFnError> {
    use std::sync::atomic::Ordering;

    {
        let _guard = CONNECTIONS_HYDRATION_LOCK.lock().await;
        service().reset_projection().map_err(|_| safe_error())?;
        CONNECTIONS_HYDRATED.store(false, Ordering::Release);
        CONNECTIONS_RESETTING.store(false, Ordering::Release);
    }
    ensure_connections_hydrated().await
}
use md_web_contracts::domains::connections::{
    BrokerStartResult, ConnectionOperationResult, ConnectionsSnapshot, ContextAction,
    ContextTriggerConfig, IntegrationUpsert, OneTimeSecret, OrgTriggerView, ScheduledMission,
    SlackConfigPatch, SlackSecretKind, SlackSecretWrite, TriggerDecision, TriggerMode,
    TriggerSource, WebhookCreateResult, WebhookSecretWrite, WebhookUpsert, WriteOnlySecret,
};

#[cfg_attr(
    not(feature = "server"),
    expect(
        dead_code,
        reason = "the web Server Function macro removes the local fallback call"
    )
)]
fn safe_error() -> ServerFnError {
    ServerFnError::new("接続設定の操作に失敗しました")
}

#[cfg(feature = "server")]
fn service() -> &'static md_web_services::domains::connections::ConnectionsService {
    md_web_services::domains::connections::connections_service()
}

#[cfg(feature = "server")]
fn snapshot() -> Result<ConnectionsSnapshot, ServerFnError> {
    service().snapshot().map_err(|_| safe_error())
}

#[cfg(feature = "server")]
async fn persisted_snapshot() -> Result<ConnectionsSnapshot, ServerFnError> {
    persist_connections().await?;
    snapshot()
}

#[cfg(feature = "server")]
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(feature = "server")]
fn operation(
    status: ConnectionOperationStatus,
    detail: &str,
) -> Result<ConnectionOperationResult, ServerFnError> {
    Ok(ConnectionOperationResult {
        status,
        detail: String::from(detail),
        snapshot: snapshot()?,
    })
}

#[cfg(feature = "server")]
fn runtime_operation(
    status: md_web_contracts::domains::connections::ListenerStatus,
) -> Result<ConnectionOperationResult, ServerFnError> {
    let operation_status = if status
        .detail
        .as_deref()
        .is_some_and(|detail| detail.contains("tunnel unavailable"))
    {
        ConnectionOperationStatus::TransportUnavailable
    } else {
        ConnectionOperationStatus::Applied
    };
    operation(
        operation_status,
        status.detail.as_deref().unwrap_or("listener is running"),
    )
}

#[cfg(feature = "server")]
fn runtime_error_operation(
    error: md_web_services::domains::connections::ConnectionsServiceError,
) -> Result<ConnectionOperationResult, ServerFnError> {
    use md_web_services::domains::connections::ConnectionsServiceError;
    let status = match error {
        ConnectionsServiceError::MissingSecret(_)
        | ConnectionsServiceError::FeatureDisabled(_)
        | ConnectionsServiceError::InvalidInput(_)
        | ConnectionsServiceError::NotFound(_) => ConnectionOperationStatus::MissingConfiguration,
        _ => ConnectionOperationStatus::TransportUnavailable,
    };
    operation(status, &error.to_string())
}

#[cfg(feature = "server")]
fn dispatch_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    format!(
        "{prefix}-{}-{}",
        now_ms(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(feature = "server")]
async fn dispatch_hive_work(
    source: &str,
    title: String,
    body: String,
) -> Result<String, ServerFnError> {
    use md_web_contracts::domains::hive_tasks::{HiveMessage, MessageAct};

    let task = super::hive_scheduler_enqueue_task(
        &title,
        Some(body.clone()),
        Some(String::from("god")),
        0,
    )
    .await?;
    let task_id = task.id;
    let message = HiveMessage {
        id: dispatch_id("message"),
        conversation: task_id.clone(),
        in_reply_to: None,
        from: String::from(source),
        to: String::from("god"),
        act: MessageAct::Request,
        subject: title,
        body,
        hops: 0,
        requires_reply: true,
        needs_human: false,
        created_at: task.created_at,
    };
    super::hive_scheduler_enqueue_message(&message).await?;
    Ok(task_id)
}

#[cfg(feature = "server")]
fn slack_dispatch() -> md_web_services::domains::connections::SlackDispatch {
    let runtime = tokio::runtime::Handle::current();
    std::sync::Arc::new(move |incoming| {
        let title = format!("Slack · {}", incoming.channel);
        runtime
            .block_on(dispatch_hive_work("slack", title, incoming.text))
            .map(|_| ())
            .map_err(|_| String::from("Hive dispatch failed"))
    })
}

#[cfg(feature = "server")]
fn webhook_dispatch() -> md_web_services::domains::connections::InboundDispatch {
    use md_web_contracts::domains::connections::{TriggerDirection, TriggerHistoryEntry};

    let runtime = tokio::runtime::Handle::current();
    std::sync::Arc::new(move |incoming| {
        let entry_id = dispatch_id("trigger");
        let title = incoming.title.clone().unwrap_or_else(|| {
            let mut title: String = incoming.body.chars().take(80).collect();
            if incoming.body.chars().count() > 80 {
                title.push('…');
            }
            title
        });
        if !incoming.mode.permits(incoming.kind) {
            service()
                .append_history(TriggerHistoryEntry {
                    id: entry_id,
                    source: TriggerSource::Webhook,
                    source_id: incoming.source_id,
                    source_name: incoming.source_name,
                    direction: TriggerDirection::Inbound,
                    peer: incoming.peer,
                    title: Some(title),
                    body: incoming.body,
                    kind: incoming.kind,
                    decision: Some(TriggerDecision::Pending),
                    correlation_id: None,
                    task_id: None,
                    at_ms: now_ms(),
                })
                .map_err(|_| String::from("history write failed"))?;
            runtime
                .block_on(persist_hydrated_connections())
                .map_err(|_| String::from("history persistence failed"))?;
            return Ok(String::new());
        }
        let task_id = runtime
            .block_on(dispatch_hive_work(
                &format!("webhook:{}", incoming.source_id),
                title.clone(),
                incoming.body.clone(),
            ))
            .map_err(|_| String::from("Hive dispatch failed"))?;
        service()
            .append_history(TriggerHistoryEntry {
                id: entry_id,
                source: TriggerSource::Webhook,
                source_id: incoming.source_id,
                source_name: incoming.source_name,
                direction: TriggerDirection::Inbound,
                peer: incoming.peer,
                title: Some(title),
                body: incoming.body,
                kind: incoming.kind,
                decision: Some(TriggerDecision::AutoAllowed),
                correlation_id: None,
                task_id: Some(task_id.clone()),
                at_ms: now_ms(),
            })
            .map_err(|_| String::from("history write failed"))?;
        runtime
            .block_on(persist_hydrated_connections())
            .map_err(|_| String::from("history persistence failed"))?;
        Ok(task_id)
    })
}

#[cfg(feature = "server")]
fn broker_handler() -> BrokerHandler {
    let runtime = tokio::runtime::Handle::current();
    std::sync::Arc::new(
        move |request: md_web_services::domains::connections::HttpRequest| {
            let md_web_services::domains::connections::HttpRequest {
                method,
                path,
                headers,
                body,
            } = request;
            let capability = headers
                .get("authorization")
                .map(String::as_str)
                .and_then(|value| value.strip_prefix("Bearer "))
                .or_else(|| headers.get("x-md-broker-token").map(String::as_str))
                .map(String::from);
            let Some(capability) = capability else {
                return md_web_services::domains::connections::HttpResponse::json(
                    401,
                    serde_json::json!({ "ok": false, "error": "unauthorized" }),
                );
            };
            match runtime.block_on(service().forward_broker_request(
                &capability,
                &method,
                &path,
                body,
            )) {
                Ok(response) => md_web_services::domains::connections::HttpResponse::raw(
                    response.status,
                    response.content_type,
                    response.body,
                ),
                Err(_) => md_web_services::domains::connections::HttpResponse::json(
                    502,
                    serde_json::json!({ "ok": false, "error": "upstream unavailable" }),
                ),
            }
        },
    )
}

#[cfg(feature = "server")]
fn mint_secret() -> Result<String, ServerFnError> {
    use std::fs::File;
    use std::io::Read;

    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut bytes))
        .map_err(|_| ServerFnError::new("安全なシークレット生成器を利用できません"))?;
    let mut encoded = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

#[get("/api/connections/domain-snapshot")]
pub(crate) async fn connections_domain_snapshot() -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        ensure_connections_hydrated().await?;
        snapshot()
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/slack/config")]
pub(crate) async fn connections_update_slack(
    patch: SlackConfigPatch,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .update_slack_config(patch)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = patch;
        Err(safe_error())
    }
}

#[post("/api/connections/slack/secret")]
pub(crate) async fn connections_write_slack_secret(
    request: SlackSecretWrite,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = require_master_key()?;
        service()
            .write_slack_secret(&request)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/connections/slack/secret/clear")]
pub(crate) async fn connections_clear_slack_secret(
    kind: SlackSecretKind,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .clear_slack_secret(kind)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = kind;
        Err(safe_error())
    }
}

#[post("/api/connections/slack/start")]
pub(crate) async fn connections_start_slack() -> Result<ConnectionOperationResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let result = match service().start_slack_runtime(slack_dispatch()) {
            Ok(status) => runtime_operation(status),
            Err(error) => runtime_error_operation(error),
        };
        if result.is_ok() {
            persist_connections().await?;
        }
        result
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/slack/stop")]
pub(crate) async fn connections_stop_slack() -> Result<ConnectionOperationResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service().stop_slack_runtime().map_err(|_| safe_error())?;
        persist_connections().await?;
        operation(ConnectionOperationStatus::Applied, "Slack listener stopped")
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/integrations/upsert")]
pub(crate) async fn connections_upsert_integration(
    request: IntegrationUpsert,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .upsert_integration(&request, now_ms())
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/connections/integrations/from-template")]
pub(crate) async fn connections_add_integration_template(
    template_id: String,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let current = snapshot()?;
        let template = current
            .integration_templates
            .iter()
            .find(|template| template.id_suggestion == template_id)
            .ok_or_else(safe_error)?;
        if template.base_url.trim().is_empty() {
            return Err(ServerFnError::new(
                "カスタムREST連携にはベースURLの入力が必要です",
            ));
        }
        let request = IntegrationUpsert {
            id: template.id_suggestion.clone(),
            label: template.label.clone(),
            kind: template.kind,
            base_url: template.base_url.clone(),
            auth_type: template.auth_type,
            auth_header: template.auth_header.clone(),
            enabled: false,
        };
        service()
            .upsert_integration(&request, now_ms())
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = template_id;
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/create-default")]
pub(crate) async fn connections_create_default_webhook(
    name: String,
) -> Result<WebhookCreateResult, ServerFnError> {
    connections_create_webhook(
        name,
        TriggerMode::Strict,
        String::from(md_web_contracts::domains::connections::DEFAULT_WEBHOOK_SCHEMA),
    )
    .await
}

#[post("/api/connections/integrations/secret")]
pub(crate) async fn connections_write_integration_secret(
    id: String,
    secret: WriteOnlySecret,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = require_master_key()?;
        service()
            .write_integration_secret(&id, &secret, now_ms())
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (id, secret);
        Err(safe_error())
    }
}

#[post("/api/connections/integrations/remove")]
pub(crate) async fn connections_remove_integration(
    id: String,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .remove_integration(&id)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = id;
        Err(safe_error())
    }
}

#[post("/api/connections/integrations/probe")]
pub(crate) async fn connections_probe_integration(
    id: String,
    path: String,
) -> Result<ConnectionOperationResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        match service().probe_integration(&id, &path).await {
            Ok(detail) => operation(ConnectionOperationStatus::Applied, &detail),
            Err(error @ (md_web_services::domains::connections::ConnectionsServiceError::MissingSecret(_)
                | md_web_services::domains::connections::ConnectionsServiceError::FeatureDisabled(_)
                | md_web_services::domains::connections::ConnectionsServiceError::NotFound(_)
                | md_web_services::domains::connections::ConnectionsServiceError::InvalidInput(_))) =>
                operation(ConnectionOperationStatus::MissingConfiguration, &error.to_string()),
            Err(error) => operation(ConnectionOperationStatus::TransportUnavailable, &error.to_string()),
        }
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (id, path);
        Err(safe_error())
    }
}

#[post("/api/connections/broker/start")]
pub(crate) async fn connections_start_broker() -> Result<BrokerStartResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let enabled_ids: Vec<String> = snapshot()?
            .integrations
            .into_iter()
            .filter(|integration| integration.enabled)
            .map(|integration| integration.id)
            .collect();
        if enabled_ids.is_empty() {
            return Err(ServerFnError::new("有効な外部連携が必要です"));
        }
        let capability = service()
            .grant_broker_capability(mint_secret()?, enabled_ids)
            .map_err(|_| safe_error())?;
        let status = service()
            .start_broker_runtime(broker_handler())
            .map_err(|_| safe_error())?;
        persist_connections().await?;
        Ok(BrokerStartResult {
            operation: ConnectionOperationResult {
                status: ConnectionOperationStatus::Applied,
                detail: status
                    .public_url
                    .unwrap_or_else(|| String::from("broker is running")),
                snapshot: snapshot()?,
            },
            capability,
        })
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/broker/stop")]
pub(crate) async fn connections_stop_broker() -> Result<ConnectionOperationResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service().stop_broker_runtime().map_err(|_| safe_error())?;
        persist_connections().await?;
        operation(ConnectionOperationStatus::Applied, "broker stopped")
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/create")]
pub(crate) async fn connections_create_webhook(
    name: String,
    mode: TriggerMode,
    schema: String,
) -> Result<WebhookCreateResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = require_master_key()?;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = format!(
            "wh-{}-{}",
            now_ms(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        let request = WebhookUpsert {
            id: id.clone(),
            name,
            enabled: false,
            mode,
            schema,
        };
        service()
            .upsert_webhook(&request, now_ms())
            .map_err(|_| safe_error())?;
        let generated = mint_secret()?;
        let secret = service()
            .apply_generated_webhook_secret(&id, generated)
            .map_err(|_| safe_error())?;
        persist_connections().await?;
        let current = snapshot()?;
        let webhook = current
            .webhooks
            .iter()
            .find(|webhook| webhook.id == id)
            .cloned()
            .ok_or_else(safe_error)?;
        Ok(WebhookCreateResult {
            webhook,
            secret,
            snapshot: current,
        })
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (name, mode, schema);
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/upsert")]
pub(crate) async fn connections_upsert_webhook(
    request: WebhookUpsert,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .upsert_webhook(&request, now_ms())
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/secret")]
pub(crate) async fn connections_write_webhook_secret(
    request: WebhookSecretWrite,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = require_master_key()?;
        service()
            .write_webhook_secret(&request)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/rotate")]
pub(crate) async fn connections_rotate_webhook_secret(
    id: String,
) -> Result<(OneTimeSecret, ConnectionsSnapshot), ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = require_master_key()?;
        let generated = mint_secret()?;
        let secret = service()
            .apply_generated_webhook_secret(&id, generated)
            .map_err(|_| safe_error())?;
        Ok((secret, persisted_snapshot().await?))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = id;
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/remove")]
pub(crate) async fn connections_remove_webhook(
    id: String,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service().remove_webhook(&id).map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = id;
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/start")]
pub(crate) async fn connections_start_webhooks() -> Result<ConnectionOperationResult, ServerFnError>
{
    #[cfg(feature = "server")]
    {
        let result = match service().start_webhook_runtime(webhook_dispatch()) {
            Ok(status) => runtime_operation(status),
            Err(error) => runtime_error_operation(error),
        };
        if result.is_ok() {
            persist_connections().await?;
        }
        result
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/webhooks/stop")]
pub(crate) async fn connections_stop_webhooks() -> Result<ConnectionOperationResult, ServerFnError>
{
    #[cfg(feature = "server")]
    {
        service().stop_webhook_runtime().map_err(|_| safe_error())?;
        persist_connections().await?;
        operation(
            ConnectionOperationStatus::Applied,
            "webhook listener stopped",
        )
    }
    #[cfg(not(feature = "server"))]
    {
        Err(safe_error())
    }
}

#[post("/api/connections/context")]
pub(crate) async fn connections_set_context(
    context: ContextTriggerConfig,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service().set_context(context).map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = context;
        Err(safe_error())
    }
}

#[post("/api/connections/context/enabled")]
pub(crate) async fn connections_set_context_enabled(
    action: ContextAction,
    enabled: bool,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut context = snapshot()?.context;
        match action {
            ContextAction::Compact => context.compact.enabled = enabled,
            ContextAction::Clear => context.clear.enabled = enabled,
        }
        service().set_context(context).map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (action, enabled);
        Err(safe_error())
    }
}

#[post("/api/connections/organisation")]
pub(crate) async fn connections_set_organisation(
    enabled: bool,
    mode: TriggerMode,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .set_organisation(enabled, mode)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (enabled, mode);
        Err(safe_error())
    }
}

#[post("/api/connections/organisation/key")]
pub(crate) async fn connections_write_organisation_key(
    secret: WriteOnlySecret,
) -> Result<OrgTriggerView, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = require_master_key()?;
        let view = service()
            .write_organisation_key(&secret)
            .map_err(|_| safe_error())?;
        persist_connections().await?;
        Ok(view)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = secret;
        Err(safe_error())
    }
}

#[post("/api/connections/history/decide")]
pub(crate) async fn connections_decide_history(
    id: String,
    decision: TriggerDecision,
    task_id: Option<String>,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut dispatched_task_id = task_id;
        if matches!(decision, TriggerDecision::Approved) && dispatched_task_id.is_none() {
            let entry = snapshot()?
                .trigger_history
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or_else(safe_error)?;
            if matches!(entry.decision, Some(TriggerDecision::Pending)) {
                let title = entry
                    .title
                    .unwrap_or_else(|| entry.body.chars().take(80).collect());
                dispatched_task_id =
                    Some(dispatch_hive_work("approved-trigger", title, entry.body).await?);
            }
        }
        service()
            .decide_history(&id, decision, dispatched_task_id)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (id, decision, task_id);
        Err(safe_error())
    }
}

#[post("/api/connections/history/clear")]
pub(crate) async fn connections_clear_history(
    source: Option<TriggerSource>,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service().clear_history(source).map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = source;
        Err(safe_error())
    }
}

#[post("/api/connections/missions")]
pub(crate) async fn connections_replace_missions(
    missions: Vec<ScheduledMission>,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        service()
            .replace_missions(missions)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = missions;
        Err(safe_error())
    }
}

#[post("/api/connections/missions/upsert")]
pub(crate) async fn connections_upsert_mission(
    mission: ScheduledMission,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut missions = snapshot()?.missions;
        if let Some(existing) = missions.iter_mut().find(|item| item.id == mission.id) {
            *existing = mission;
        } else {
            missions.push(mission);
        }
        service()
            .replace_missions(missions)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = mission;
        Err(safe_error())
    }
}

#[post("/api/connections/missions/enabled")]
pub(crate) async fn connections_set_mission_enabled(
    id: String,
    enabled: bool,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut missions = snapshot()?.missions;
        let mission = missions
            .iter_mut()
            .find(|mission| mission.id == id)
            .ok_or_else(safe_error)?;
        mission.enabled = enabled;
        service()
            .replace_missions(missions)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = (id, enabled);
        Err(safe_error())
    }
}

#[post("/api/connections/missions/remove")]
pub(crate) async fn connections_remove_mission(
    id: String,
) -> Result<ConnectionsSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut missions = snapshot()?.missions;
        missions.retain(|mission| mission.id != id);
        service()
            .replace_missions(missions)
            .map_err(|_| safe_error())?;
        persisted_snapshot().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = id;
        Err(safe_error())
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "server")]
    use super::mint_secret;

    #[cfg(feature = "server")]
    #[test]
    fn minted_secret_has_256_bits_of_hex() -> Result<(), dioxus::prelude::ServerFnError> {
        let secret = mint_secret()?;

        assert_eq!(secret.len(), 64);
        assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
        Ok(())
    }
}
