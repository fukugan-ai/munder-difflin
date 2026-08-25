#![forbid(unsafe_code)]

use dioxus::prelude::*;
use md_web_contracts::domains::voice_realtime::{
    ActionResult, FreeflowConfig, FreeflowConfigPatch, RealtimeActionRequest, RealtimeCostSnapshot,
    RealtimeMintRequest, RealtimeMintResult, RealtimeUsage, TranscriptionMetadata,
    TranscriptionResult, VoiceBootstrap, VoiceEventBatch, VoiceProviderKey, VoiceProviderKeyWrite,
};

#[get("/api/voice/bootstrap")]
pub(crate) async fn voice_bootstrap() -> Result<VoiceBootstrap, ServerFnError> {
    #[cfg(feature = "server")]
    {
        drop(voice_tls_paths());
        server::bootstrap().await
    }

    #[cfg(not(feature = "server"))]
    {
        Err(ServerFnError::new("voice service is server-only"))
    }
}

#[post("/api/voice/freeflow/config")]
pub(crate) async fn voice_set_freeflow_config(
    patch: FreeflowConfigPatch,
) -> Result<FreeflowConfig, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::set_freeflow_config(patch).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = patch;
        Err(ServerFnError::new("voice service is server-only"))
    }
}

#[post("/api/voice/provider-key")]
pub(crate) async fn voice_write_provider_key(
    request: VoiceProviderKeyWrite,
) -> Result<VoiceBootstrap, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::write_provider_key(request).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(ServerFnError::new(
            "voice credential service is server-only",
        ))
    }
}

#[post("/api/voice/provider-key/clear")]
pub(crate) async fn voice_clear_provider_key(
    provider: VoiceProviderKey,
) -> Result<VoiceBootstrap, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::clear_provider_key(provider).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = provider;
        Err(ServerFnError::new(
            "voice credential service is server-only",
        ))
    }
}

#[post("/api/voice/realtime/cost/cap")]
pub(crate) async fn voice_set_realtime_cost_cap(
    cap_usd: Option<f64>,
) -> Result<RealtimeCostSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::set_realtime_cost_cap(cap_usd).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = cap_usd;
        Err(ServerFnError::new("voice cost service is server-only"))
    }
}

#[post("/api/voice/realtime/cost/usage")]
pub(crate) async fn voice_record_realtime_usage(
    usage: RealtimeUsage,
) -> Result<RealtimeCostSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::record_realtime_usage(usage)
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = usage;
        Err(ServerFnError::new("voice cost service is server-only"))
    }
}

#[server(endpoint = "/api/voice/freeflow/transcribe", input = Cbor)]
pub(crate) async fn voice_transcribe(
    metadata: TranscriptionMetadata,
    audio: Vec<u8>,
) -> Result<TranscriptionResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::transcribe(metadata, audio).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = (metadata, audio);
        Err(ServerFnError::new("voice service is server-only"))
    }
}

#[post("/api/voice/realtime/mint")]
pub(crate) async fn voice_mint_realtime_token(
    request: RealtimeMintRequest,
) -> Result<RealtimeMintResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::mint_realtime_token(request).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(ServerFnError::new("voice service is server-only"))
    }
}

#[post("/api/voice/realtime/action")]
pub(crate) async fn voice_action(
    request: RealtimeActionRequest,
) -> Result<ActionResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::propose_action(request).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(ServerFnError::new("voice action service is server-only"))
    }
}

#[post("/api/voice/realtime/action/confirm")]
pub(crate) async fn voice_confirm_action(
    pending_id: String,
    phrase: String,
) -> Result<ActionResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::confirm_action(&pending_id, &phrase).await
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = (pending_id, phrase);
        Err(ServerFnError::new("voice action service is server-only"))
    }
}

#[post("/api/voice/realtime/action/cancel")]
pub(crate) async fn voice_cancel_action() -> Result<ActionResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::cancel_action()
    }

    #[cfg(not(feature = "server"))]
    {
        Err(ServerFnError::new("voice action service is server-only"))
    }
}

#[post("/api/voice/realtime/session")]
pub(crate) async fn voice_set_session_live(live: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::set_session_live(live)
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = live;
        Err(ServerFnError::new("voice session service is server-only"))
    }
}

#[get("/api/voice/realtime/events/:after_sequence")]
pub(crate) async fn voice_events(after_sequence: u64) -> Result<VoiceEventBatch, ServerFnError> {
    #[cfg(feature = "server")]
    {
        server::poll_watchers().await?;
        server::events_after(after_sequence)
    }

    #[cfg(not(feature = "server"))]
    {
        let _ = after_sequence;
        Err(ServerFnError::new("voice event service is server-only"))
    }
}

/// Startup-only TLS seam for the shared HTTP shell. Paths never enter browser DTOs.
#[cfg(feature = "server")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct VoiceTlsPaths {
    pub cert_path: std::path::PathBuf,
    pub key_path: std::path::PathBuf,
}

#[cfg(feature = "server")]
pub(crate) fn voice_tls_paths() -> Result<Option<VoiceTlsPaths>, ServerFnError> {
    let paths = server::tls_paths()?;
    if let Some(paths) = paths.as_ref() {
        debug_assert!(paths.cert_path.is_file() && paths.key_path.is_file());
    }
    Ok(paths)
}

#[cfg(feature = "server")]
mod server {
    use std::collections::BTreeMap;
    use std::collections::VecDeque;
    use std::env;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use dioxus::prelude::ServerFnError;
    use md_web_contracts::domains::connections::WriteOnlySecret;
    use md_web_contracts::domains::voice_realtime::{
        ActionResult, DEFAULT_FREEFLOW_MODEL, DEFAULT_IDLE_DISCONNECT_MS, DEFAULT_REALTIME_MODEL,
        FreeflowConfig, FreeflowConfigPatch, RealtimeActionRequest, RealtimeCostSnapshot,
        RealtimeEnqueue, RealtimeMintRequest, RealtimeMintResult, RealtimeUsage,
        TranscriptionMetadata, TranscriptionResult, VoiceBootstrap, VoiceErrorCode,
        VoiceEventBatch, VoiceEventEnvelope, VoiceProviderKey, VoiceProviderKeyWrite,
        VoiceServerEvent,
    };
    use md_web_services::domains::connections::{
        ConnectionsService, ProviderSecretId, SecretId, SecretProvider,
    };
    use md_web_services::domains::voice_realtime::{
        ActionDisposition, ActionPolicy, AudioValidationError, CompletionDetector,
        CompletionInboxMessage, CompletionTask, ConfirmationOutcome, FloorAgent, FloorObserver,
        FloorTask, PendingDispatch, RealtimeCostMeter, VoicePolicyError, VoiceUpstreamClient,
        validate_audio,
    };

    use super::super::config::{config_get, config_patch};
    use super::super::connections::{
        connections_upsert_mission, hydrated_secret_provider, persist_connections_state,
        update_voice_durable_settings, voice_durable_settings,
    };
    use super::super::hive::{
        hive_add_task, hive_control_auto_delivery, hive_control_gate, hive_control_halt,
        hive_control_pause, hive_control_resume, hive_control_steer, hive_delete_task,
        hive_patch_task, hive_snapshot,
    };
    use super::super::pty::{list_agents, pty_kill, pty_queue, pty_spawn, pty_unarchive};

    const MAX_EVENTS: usize = 100;

    struct VoiceServerState {
        freeflow_enabled: bool,
        freeflow_model: String,
        idle_disconnect_ms: u64,
        cost_meter: RealtimeCostMeter,
        action_policy: ActionPolicy,
        completion_detector: CompletionDetector,
        floor_observer: FloorObserver,
        session_live: bool,
        next_sequence: u64,
        events: VecDeque<VoiceEventEnvelope>,
    }

    impl VoiceServerState {
        fn from_environment() -> Self {
            Self {
                freeflow_enabled: env_bool("MD_FREEFLOW_ENABLED", true),
                freeflow_model: env::var("MD_FREEFLOW_MODEL")
                    .ok()
                    .filter(|model| !model.trim().is_empty())
                    .unwrap_or_else(|| String::from(DEFAULT_FREEFLOW_MODEL)),
                idle_disconnect_ms: env::var("MD_REALTIME_IDLE_DISCONNECT_MS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(DEFAULT_IDLE_DISCONNECT_MS),
                cost_meter: RealtimeCostMeter::default(),
                action_policy: ActionPolicy::default(),
                completion_detector: CompletionDetector::default(),
                floor_observer: FloorObserver::default(),
                session_live: false,
                next_sequence: 0,
                events: VecDeque::with_capacity(MAX_EVENTS),
            }
        }

        fn freeflow_config(&self, has_groq_key: bool) -> FreeflowConfig {
            FreeflowConfig {
                enabled: self.freeflow_enabled,
                model: self.freeflow_model.clone(),
                has_groq_key,
            }
        }
    }

    static STATE: OnceLock<Mutex<VoiceServerState>> = OnceLock::new();
    static UPSTREAM: OnceLock<Result<VoiceUpstreamClient, ()>> = OnceLock::new();
    static ENV_SECRETS: OnceLock<Result<(), ()>> = OnceLock::new();

    fn state() -> &'static Mutex<VoiceServerState> {
        STATE.get_or_init(|| Mutex::new(VoiceServerState::from_environment()))
    }

    fn upstream() -> Result<&'static VoiceUpstreamClient, ServerFnError> {
        UPSTREAM
            .get_or_init(|| VoiceUpstreamClient::new().map_err(|_| ()))
            .as_ref()
            .map_err(|_| ServerFnError::new("voice upstream client is unavailable"))
    }

    fn secret_id(provider: VoiceProviderKey) -> SecretId {
        SecretId::Provider(match provider {
            VoiceProviderKey::Groq => ProviderSecretId::Groq,
            VoiceProviderKey::OpenAi => ProviderSecretId::OpenAi,
        })
    }

    fn ensure_environment_secrets(
        provider_service: &ConnectionsService,
    ) -> Result<(), ServerFnError> {
        ENV_SECRETS
            .get_or_init(|| {
                for (name, provider_key) in [
                    ("GROQ_API_KEY", VoiceProviderKey::Groq),
                    ("OPENAI_API_KEY", VoiceProviderKey::OpenAi),
                ] {
                    let id = secret_id(provider_key);
                    if provider_service.has_secret(&id).map_err(|_| ())? {
                        continue;
                    }
                    let Some(value) = env::var(name).ok().filter(|value| !value.trim().is_empty())
                    else {
                        continue;
                    };
                    let secret = WriteOnlySecret::new(value).map_err(|_| ())?;
                    provider_service.set_secret(id, &secret).map_err(|_| ())?;
                }
                Ok(())
            })
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))
    }

    async fn has_provider_key(provider: VoiceProviderKey) -> Result<bool, ServerFnError> {
        let secrets = hydrated_secret_provider().await?;
        ensure_environment_secrets(secrets)?;
        secrets
            .has_secret(&secret_id(provider))
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))
    }

    async fn provider_key(
        provider: VoiceProviderKey,
    ) -> Result<Option<md_web_services::domains::connections::ServerSecret>, ServerFnError> {
        let secrets = hydrated_secret_provider().await?;
        ensure_environment_secrets(secrets)?;
        secrets
            .get_secret(&secret_id(provider))
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))
    }

    pub(super) fn tls_paths() -> Result<Option<super::VoiceTlsPaths>, ServerFnError> {
        if !env_bool("MD_WEB_HTTPS", false) {
            return Ok(None);
        }
        let cert_path = env::var_os("MD_WEB_TLS_CERT_PATH")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| ServerFnError::new("TLS certificate path is not configured"))?;
        let key_path = env::var_os("MD_WEB_TLS_KEY_PATH")
            .map(std::path::PathBuf::from)
            .ok_or_else(|| ServerFnError::new("TLS private-key path is not configured"))?;
        if !cert_path.is_file() || !key_path.is_file() {
            return Err(ServerFnError::new("TLS certificate files are unavailable"));
        }
        Ok(Some(super::VoiceTlsPaths {
            cert_path,
            key_path,
        }))
    }

    fn https_ready(enabled: bool, cert_is_file: bool, key_is_file: bool) -> bool {
        enabled && cert_is_file && key_is_file
    }

    pub(super) async fn bootstrap() -> Result<VoiceBootstrap, ServerFnError> {
        let durable = voice_durable_settings().await?;
        {
            let mut state = state()
                .lock()
                .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
            if let Some(enabled) = durable.freeflow_enabled {
                state.freeflow_enabled = enabled;
            }
            if let Some(model) = durable.freeflow_model {
                state.freeflow_model = model;
            }
            if let Some(cap) = durable.realtime_cost_cap_microusd {
                state
                    .cost_meter
                    .set_cap(cap.map(|microusd| microusd as f64 / 1_000_000.0));
            }
        }
        let has_groq_key = has_provider_key(VoiceProviderKey::Groq).await?;
        let has_openai_key = has_provider_key(VoiceProviderKey::OpenAi).await?;
        let state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        let https_enabled = env_bool("MD_WEB_HTTPS", false);
        let cert_path = env::var_os("MD_WEB_TLS_CERT_PATH");
        let key_path = env::var_os("MD_WEB_TLS_KEY_PATH");
        let cert_path_configured = cert_path.is_some();
        let key_path_configured = key_path.is_some();
        let https_ready = https_ready(
            https_enabled,
            cert_path
                .as_ref()
                .is_some_and(|path| std::path::Path::new(path).is_file()),
            key_path
                .as_ref()
                .is_some_and(|path| std::path::Path::new(path).is_file()),
        );
        Ok(VoiceBootstrap {
            freeflow: state.freeflow_config(has_groq_key),
            has_openai_key,
            https_required_for_lan: true,
            server_https_configured: https_enabled,
            server_https_ready: https_ready,
            tls_cert_path_configured: cert_path_configured,
            tls_key_path_configured: key_path_configured,
            realtime_model: String::from(DEFAULT_REALTIME_MODEL),
            idle_disconnect_ms: state.idle_disconnect_ms,
            realtime_cost: state.cost_meter.snapshot().clone(),
        })
    }

    pub(super) async fn set_freeflow_config(
        patch: FreeflowConfigPatch,
    ) -> Result<FreeflowConfig, ServerFnError> {
        let mut durable = voice_durable_settings().await?;
        if let Some(enabled) = patch.enabled {
            durable.freeflow_enabled = Some(enabled);
        }
        if let Some(model) = patch.model {
            let model = model.trim();
            if !model.is_empty() && model.len() <= 200 {
                durable.freeflow_model = Some(String::from(model));
            }
        }
        let durable = update_voice_durable_settings(durable).await?;
        {
            let mut state = state()
                .lock()
                .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
            if let Some(enabled) = durable.freeflow_enabled {
                state.freeflow_enabled = enabled;
            }
            if let Some(model) = durable.freeflow_model {
                state.freeflow_model = model;
            }
        }
        let has_groq_key = has_provider_key(VoiceProviderKey::Groq).await?;
        let state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        Ok(state.freeflow_config(has_groq_key))
    }

    pub(super) async fn write_provider_key(
        request: VoiceProviderKeyWrite,
    ) -> Result<VoiceBootstrap, ServerFnError> {
        let secrets = hydrated_secret_provider().await?;
        ensure_environment_secrets(secrets)?;
        let id = secret_id(request.provider);
        let previous = secrets
            .get_secret(&id)
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))?;
        secrets
            .set_secret(id.clone(), &request.secret)
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))?;
        if let Err(error) = persist_connections_state().await {
            restore_provider_secret(secrets, id, previous)?;
            return Err(error);
        }
        bootstrap().await
    }

    pub(super) async fn clear_provider_key(
        provider: VoiceProviderKey,
    ) -> Result<VoiceBootstrap, ServerFnError> {
        let secrets = hydrated_secret_provider().await?;
        ensure_environment_secrets(secrets)?;
        let id = secret_id(provider);
        let previous = secrets
            .get_secret(&id)
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))?;
        secrets
            .clear_secret(&id)
            .map_err(|_| ServerFnError::new("voice credential provider is unavailable"))?;
        if let Err(error) = persist_connections_state().await {
            restore_provider_secret(secrets, id, previous)?;
            return Err(error);
        }
        bootstrap().await
    }

    fn restore_provider_secret(
        provider: &ConnectionsService,
        id: SecretId,
        previous: Option<md_web_services::domains::connections::ServerSecret>,
    ) -> Result<(), ServerFnError> {
        if let Some(previous) = previous {
            let secret = WriteOnlySecret::new(String::from(previous.expose_for_server()))
                .map_err(|_| ServerFnError::new("voice credential rollback failed"))?;
            provider
                .set_secret(id, &secret)
                .map_err(|_| ServerFnError::new("voice credential rollback failed"))
        } else {
            provider
                .clear_secret(&id)
                .map_err(|_| ServerFnError::new("voice credential rollback failed"))
        }
    }

    pub(super) async fn set_realtime_cost_cap(
        cap_usd: Option<f64>,
    ) -> Result<RealtimeCostSnapshot, ServerFnError> {
        let normalized = cap_usd.filter(|cap| cap.is_finite() && *cap > 0.0);
        let mut durable = voice_durable_settings().await?;
        durable.realtime_cost_cap_microusd = Some(
            normalized.map(|cap| (cap * 1_000_000.0).round().clamp(0.0, u64::MAX as f64) as u64),
        );
        let durable = update_voice_durable_settings(durable).await?;
        let mut state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        state.cost_meter.set_cap(
            durable
                .realtime_cost_cap_microusd
                .flatten()
                .map(|microusd| microusd as f64 / 1_000_000.0),
        );
        Ok(state.cost_meter.snapshot().clone())
    }

    pub(super) fn record_realtime_usage(
        usage: RealtimeUsage,
    ) -> Result<RealtimeCostSnapshot, ServerFnError> {
        let mut state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        state.cost_meter.record(&usage, now_ms());
        Ok(state.cost_meter.snapshot().clone())
    }

    pub(super) async fn transcribe(
        metadata: TranscriptionMetadata,
        audio: Vec<u8>,
    ) -> Result<TranscriptionResult, ServerFnError> {
        let (enabled, model) = {
            let state = state()
                .lock()
                .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
            (state.freeflow_enabled, state.freeflow_model.clone())
        };
        if !enabled {
            return Ok(TranscriptionResult::Error {
                code: VoiceErrorCode::Disabled,
                message: String::from("Free Flowは無効です"),
            });
        }
        let Some(key) = provider_key(VoiceProviderKey::Groq).await? else {
            return Ok(TranscriptionResult::Error {
                code: VoiceErrorCode::NoKey,
                message: String::from("Groq APIキーが未設定です"),
            });
        };
        let validated = validate_audio(&metadata, &audio).map_err(audio_error)?;
        Ok(upstream()?
            .transcribe(key.expose_for_server(), &model, validated)
            .await)
    }

    pub(super) async fn mint_realtime_token(
        request: RealtimeMintRequest,
    ) -> Result<RealtimeMintResult, ServerFnError> {
        let Some(key) = provider_key(VoiceProviderKey::OpenAi).await? else {
            return Ok(RealtimeMintResult::Error {
                code: VoiceErrorCode::NoKey,
                message: String::from("OpenAI APIキーが未設定です"),
            });
        };
        let model = request
            .model
            .as_deref()
            .filter(|model| !model.trim().is_empty() && model.len() <= 200)
            .unwrap_or(DEFAULT_REALTIME_MODEL);
        Ok(upstream()?
            .mint_realtime_token(key.expose_for_server(), model)
            .await)
    }

    pub(super) async fn propose_action(
        request: RealtimeActionRequest,
    ) -> Result<ActionResult, ServerFnError> {
        let disposition = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?
            .action_policy
            .propose(request, god_agent_id().as_deref(), now_ms())
            .map_err(policy_error)?;
        match disposition {
            ActionDisposition::Execute(request) => execute_action(*request).await,
            ActionDisposition::AwaitConfirmation {
                pending_id, spoken, ..
            } => Ok(ActionResult {
                ok: true,
                spoken,
                needs_confirm: true,
                pending_id: Some(pending_id),
            }),
        }
    }

    pub(super) async fn confirm_action(
        pending_id: &str,
        phrase: &str,
    ) -> Result<ActionResult, ServerFnError> {
        let outcome = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?
            .action_policy
            .confirm(pending_id, phrase, now_ms())
            .map_err(policy_error)?;
        match outcome {
            ConfirmationOutcome::Execute(request) => execute_action(*request).await,
            ConfirmationOutcome::Cancelled => Ok(cancelled_result()),
        }
    }

    pub(super) fn cancel_action() -> Result<ActionResult, ServerFnError> {
        let _ = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?
            .action_policy
            .cancel();
        Ok(cancelled_result())
    }

    pub(super) fn set_session_live(live: bool) -> Result<(), ServerFnError> {
        let mut state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        state.session_live = live;
        state.completion_detector.set_session_live(live);
        state.floor_observer.set_session_live(live);
        if live {
            state.cost_meter.start(now_ms());
            for completion in state.completion_detector.drain_queued() {
                push_event_locked(&mut state, VoiceServerEvent::Completion(completion));
            }
        } else {
            state.cost_meter.stop();
        }
        Ok(())
    }

    pub(super) async fn poll_watchers() -> Result<(), ServerFnError> {
        let snapshot = hive_snapshot(None).await?;
        let tasks = snapshot
            .tasks
            .iter()
            .map(|task| CompletionTask {
                id: task.id.clone(),
                status: task_status(task.status),
                title: task.title.clone(),
            })
            .collect::<Vec<_>>();
        let inbox = snapshot
            .messages
            .iter()
            .map(|message| CompletionInboxMessage {
                id: message.id.clone(),
                from: message.from.clone(),
                in_reply_to: message.in_reply_to.clone(),
                body: message.body.clone(),
                created_at_ms: message.created_at.parse::<i64>().unwrap_or(0),
            })
            .collect::<Vec<_>>();
        let floor_agents = snapshot
            .agents
            .iter()
            .map(|agent| FloorAgent {
                id: agent.id.clone(),
                name: agent.name.clone(),
                archived: agent.archived,
            })
            .collect::<Vec<_>>();
        let floor_tasks = snapshot
            .tasks
            .iter()
            .map(|task| FloorTask {
                id: task.id.clone(),
                title: task.title.clone(),
                status: task_status(task.status),
            })
            .collect::<Vec<_>>();
        let now = now_ms();
        let mut state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        for completion in state.completion_detector.poll(&tasks, &inbox, now) {
            push_event_locked(&mut state, VoiceServerEvent::Completion(completion));
        }
        if let Some(delta) = state
            .floor_observer
            .observe(&floor_agents, &floor_tasks, &[], now)
        {
            push_event_locked(&mut state, VoiceServerEvent::FloorDelta(delta));
        }
        Ok(())
    }

    pub(super) fn events_after(after_sequence: u64) -> Result<VoiceEventBatch, ServerFnError> {
        let mut state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        let events = state
            .events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .cloned()
            .collect();
        if !state.session_live {
            state.events.clear();
        }
        Ok(VoiceEventBatch {
            latest_sequence: state.next_sequence,
            events,
        })
    }

    async fn execute_action(request: RealtimeActionRequest) -> Result<ActionResult, ServerFnError> {
        use md_web_contracts::domains::voice_realtime::VoiceActionVerb;

        let result = match request.verb {
            VoiceActionVerb::Ping | VoiceActionVerb::Dispatch => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                let text = request
                    .text
                    .as_deref()
                    .or(request.objective.as_deref())
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| ServerFnError::new("voice action text is required"))?;
                pty_queue(String::from(agent_id), String::from(text)).await
            }
            VoiceActionVerb::Steer => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                let text = required(request.text.as_deref(), "steer text is required")?;
                hive_control_steer(String::from(agent_id), String::from(text))
                    .await
                    .map(|_| ())
            }
            VoiceActionVerb::CreateTask => create_task(&request).await,
            VoiceActionVerb::AssignTask => patch_task_assignee(&request).await,
            VoiceActionVerb::UpdateTask => patch_task(&request).await,
            VoiceActionVerb::DeleteTask => {
                let task_id = required(request.task_id.as_deref(), "task id is required")?;
                hive_delete_task(String::from(task_id)).await
            }
            VoiceActionVerb::WaitFor => patch_task_dependency(&request).await,
            VoiceActionVerb::ClearContext => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                enqueue_for_browser(agent_id, "/clear")
            }
            VoiceActionVerb::Kill => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                let (active, _) = list_agents().await?;
                let pty_id = active
                    .iter()
                    .find(|agent| agent.id == agent_id)
                    .and_then(|agent| agent.pty_id.clone())
                    .ok_or_else(|| ServerFnError::new("target agent has no active terminal"))?;
                pty_kill(pty_id).await
            }
            VoiceActionVerb::Spawn => {
                let spawn = request
                    .spawn_request
                    .clone()
                    .ok_or_else(|| ServerFnError::new("typed spawn request is required"))?;
                pty_spawn(spawn).await.map(|_| ())
            }
            VoiceActionVerb::Pause => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                hive_control_pause(String::from(agent_id), true)
                    .await
                    .map(|_| ())
            }
            VoiceActionVerb::Halt => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                hive_control_halt(String::from(agent_id)).await.map(|_| ())
            }
            VoiceActionVerb::Resume => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                hive_control_resume(String::from(agent_id))
                    .await
                    .map(|_| ())
            }
            VoiceActionVerb::AutoDelivery => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                let paused = request
                    .setting_value
                    .as_deref()
                    .map(parse_bool_value)
                    .transpose()?
                    .unwrap_or(false);
                hive_control_auto_delivery(String::from(agent_id), paused)
                    .await
                    .map(|_| ())
            }
            VoiceActionVerb::GateTool => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                let tool = required(request.tool_name.as_deref(), "tool name is required")?;
                let enabled = request
                    .enabled
                    .ok_or_else(|| ServerFnError::new("tool gate state is required"))?;
                hive_control_gate(String::from(agent_id), String::from(tool), enabled)
                    .await
                    .map(|_| ())
            }
            VoiceActionVerb::Archive => archive_agent(&request).await,
            VoiceActionVerb::Unarchive => {
                let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
                pty_unarchive(String::from(agent_id)).await.map(|_| ())
            }
            VoiceActionVerb::EditSchedule | VoiceActionVerb::CreateSchedule => {
                let mission = request
                    .mission
                    .clone()
                    .ok_or_else(|| ServerFnError::new("typed schedule is required"))?;
                connections_upsert_mission(mission).await.map(|_| ())
            }
            VoiceActionVerb::UpdateSetting => update_setting(&request).await,
        };
        match result {
            Ok(()) => {
                track_completion(&request)?;
                Ok(ActionResult {
                    ok: true,
                    spoken: action_success_message(request.verb),
                    needs_confirm: false,
                    pending_id: None,
                })
            }
            Err(error) => Err(error),
        }
    }

    fn track_completion(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        use md_web_contracts::domains::voice_realtime::CompletionKind;

        let target_agent_id = request
            .agent_id
            .clone()
            .or_else(|| request.spawn_request.as_ref().map(|spawn| spawn.id.clone()));
        let Some(target_agent_id) = target_agent_id else {
            return Ok(());
        };
        let kind = match request.verb {
            md_web_contracts::domains::voice_realtime::VoiceActionVerb::CreateTask => {
                CompletionKind::Task
            }
            md_web_contracts::domains::voice_realtime::VoiceActionVerb::Spawn => {
                CompletionKind::Spawn
            }
            _ => CompletionKind::Dispatch,
        };
        let now = now_ms();
        state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?
            .completion_detector
            .track(
                PendingDispatch {
                    correlation_id: format!("voice-{now}"),
                    kind,
                    target_agent_id,
                    task_id: request.task_id.clone(),
                    objective: request.objective.clone().or_else(|| request.text.clone()),
                    dispatched_at_ms: now,
                    dispatch_message_id: None,
                },
                now,
            );
        Ok(())
    }

    async fn create_task(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        use md_web_contracts::domains::hive_tasks::{HiveTask, TaskStatus};

        let title = required(request.title.as_deref(), "task title is required")?;
        let now = now_ms();
        let id = request
            .task_id
            .clone()
            .unwrap_or_else(|| format!("voice-{now}"));
        let task = HiveTask {
            id,
            title: String::from(title),
            description: request.objective.clone().or_else(|| request.text.clone()),
            assignee: request.agent_id.clone(),
            status: TaskStatus::Todo,
            depends_on: Vec::new(),
            priority: 0,
            created_at: now.to_string(),
            human_qa: Vec::new(),
            result: None,
            extra: BTreeMap::new(),
        };
        hive_add_task(task).await.map(|_| ())
    }

    async fn archive_agent(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
        let (active, _) = list_agents().await?;
        let pty_id = active
            .iter()
            .find(|agent| agent.id == agent_id)
            .and_then(|agent| agent.pty_id.clone())
            .ok_or_else(|| ServerFnError::new("target agent has no active terminal"))?;
        pty_kill(pty_id).await
    }

    async fn update_setting(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        use md_web_contracts::domains::config_onboarding::{
            AgentProvider, ConfigPatch, TerminalTheme,
        };

        let key = required(request.setting_key.as_deref(), "setting key is required")?;
        let value = required(
            request.setting_value.as_deref(),
            "setting value is required",
        )?;
        let current = config_get().await?;
        let mut patch = ConfigPatch {
            expected_revision: current.revision,
            ..ConfigPatch::default()
        };
        match key {
            "notifications" => patch.notifications = Some(parse_bool_value(value)?),
            "freeflowEnabled" => patch.freeflow_enabled = Some(parse_bool_value(value)?),
            "strongKeepalive" => patch.strong_keepalive = Some(parse_bool_value(value)?),
            "autoUpdate" => patch.auto_update = Some(parse_bool_value(value)?),
            "autoMode" => patch.auto_mode = Some(parse_bool_value(value)?),
            "semanticMemory" => patch.semantic_memory = Some(parse_bool_value(value)?),
            "terminalTheme" => {
                patch.terminal_theme = Some(match value.trim() {
                    "light" => TerminalTheme::Light,
                    "dark" => TerminalTheme::Dark,
                    _ => return Err(ServerFnError::new("terminal theme is invalid")),
                });
            }
            "realtimeIdleDisconnectMs" => {
                patch.realtime_idle_disconnect_ms = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| ServerFnError::new("idle timeout is invalid"))?,
                );
            }
            "defaultModel" => patch.default_model = Some(String::from(value)),
            "godModel" => patch.god_model = Some(String::from(value)),
            "godProvider" => {
                patch.god_provider = Some(match value.trim().to_ascii_lowercase().as_str() {
                    "claude" => AgentProvider::Claude,
                    "codex" => AgentProvider::Codex,
                    "antigravity" => AgentProvider::Antigravity,
                    "gemini" => AgentProvider::Gemini,
                    "qwen" => AgentProvider::Qwen,
                    "opencode" => AgentProvider::OpenCode,
                    "crush" => AgentProvider::Crush,
                    "pi" => AgentProvider::Pi,
                    "copilot" => AgentProvider::Copilot,
                    "cursor" => AgentProvider::Cursor,
                    "grok" => AgentProvider::Grok,
                    "kimi" => AgentProvider::Kimi,
                    custom => AgentProvider::Custom(String::from(custom)),
                });
            }
            _ => return Err(ServerFnError::new("setting is not voice-allowlisted")),
        }
        let updated = config_patch(patch).await?;
        let mut voice_state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        voice_state.freeflow_enabled = updated.freeflow_enabled;
        voice_state.idle_disconnect_ms = updated.realtime_idle_disconnect_ms;
        Ok(())
    }

    async fn patch_task_assignee(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        let task_id = required(request.task_id.as_deref(), "task id is required")?;
        let agent_id = required(request.agent_id.as_deref(), "agent target is required")?;
        let mut patch = serde_json::Map::new();
        patch.insert(
            String::from("assignee"),
            serde_json::Value::String(String::from(agent_id)),
        );
        hive_patch_task(String::from(task_id), patch)
            .await
            .map(|_| ())
    }

    async fn patch_task(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        let task_id = required(request.task_id.as_deref(), "task id is required")?;
        let mut patch = serde_json::Map::new();
        if let Some(title) = request
            .title
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            patch.insert(
                String::from("title"),
                serde_json::Value::String(String::from(title)),
            );
        }
        if let Some(description) = request
            .objective
            .as_deref()
            .or(request.text.as_deref())
            .filter(|value| !value.trim().is_empty())
        {
            patch.insert(
                String::from("description"),
                serde_json::Value::String(String::from(description)),
            );
        }
        if patch.is_empty() {
            return Err(ServerFnError::new("task update is empty"));
        }
        hive_patch_task(String::from(task_id), patch)
            .await
            .map(|_| ())
    }

    async fn patch_task_dependency(request: &RealtimeActionRequest) -> Result<(), ServerFnError> {
        let task_id = required(request.task_id.as_deref(), "task id is required")?;
        let dependency = required(
            request.text.as_deref().or(request.objective.as_deref()),
            "dependency task id is required",
        )?;
        let mut patch = serde_json::Map::new();
        patch.insert(
            String::from("dependsOn"),
            serde_json::Value::Array(vec![serde_json::Value::String(String::from(dependency))]),
        );
        hive_patch_task(String::from(task_id), patch)
            .await
            .map(|_| ())
    }

    fn parse_bool_value(value: &str) -> Result<bool, ServerFnError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "yes" | "1" => Ok(true),
            "false" | "off" | "no" | "0" => Ok(false),
            _ => Err(ServerFnError::new("boolean setting value is invalid")),
        }
    }

    fn push_event_locked(state: &mut VoiceServerState, event: VoiceServerEvent) {
        state.next_sequence = state.next_sequence.saturating_add(1);
        let sequence = state.next_sequence;
        state
            .events
            .push_back(VoiceEventEnvelope { sequence, event });
        if state.events.len() > MAX_EVENTS {
            let _ = state.events.pop_front();
        }
    }

    fn enqueue_for_browser(agent_id: &str, text: &str) -> Result<(), ServerFnError> {
        let mut state = state()
            .lock()
            .map_err(|_| ServerFnError::new("voice state is unavailable"))?;
        push_event_locked(
            &mut state,
            VoiceServerEvent::Enqueue(RealtimeEnqueue {
                agent_id: String::from(agent_id),
                text: String::from(text),
            }),
        );
        Ok(())
    }

    fn task_status(status: md_web_contracts::domains::hive_tasks::TaskStatus) -> String {
        use md_web_contracts::domains::hive_tasks::TaskStatus;

        String::from(match status {
            TaskStatus::Todo => "todo",
            TaskStatus::Doing => "doing",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Done => "done",
        })
    }

    fn action_success_message(
        verb: md_web_contracts::domains::voice_realtime::VoiceActionVerb,
    ) -> String {
        use md_web_contracts::domains::voice_realtime::VoiceActionVerb;

        String::from(match verb {
            VoiceActionVerb::Ping => "エージェントへ確認を送りました",
            VoiceActionVerb::Dispatch => "エージェントへ依頼を送りました",
            VoiceActionVerb::Steer => "エージェントへ方針を送りました",
            VoiceActionVerb::CreateTask => "タスクを作成しました",
            VoiceActionVerb::AssignTask => "タスクを割り当てました",
            VoiceActionVerb::UpdateTask => "タスクを更新しました",
            VoiceActionVerb::DeleteTask => "タスクを削除しました",
            VoiceActionVerb::WaitFor => "タスクの依存関係を更新しました",
            VoiceActionVerb::Spawn => "エージェントを起動しました",
            VoiceActionVerb::Kill => "エージェントを終了してアーカイブしました",
            VoiceActionVerb::Pause => "エージェントを一時停止しました",
            VoiceActionVerb::Halt => "エージェントへ停止を要求しました",
            VoiceActionVerb::Resume => "エージェントを再開しました",
            VoiceActionVerb::AutoDelivery => "自動配信設定を更新しました",
            VoiceActionVerb::GateTool => "ツールゲートを更新しました",
            VoiceActionVerb::Archive => "エージェントをアーカイブしました",
            VoiceActionVerb::Unarchive => "エージェントを復元しました",
            VoiceActionVerb::ClearContext => "コンテキストをクリアしました",
            VoiceActionVerb::EditSchedule => "スケジュールを更新しました",
            VoiceActionVerb::CreateSchedule => "スケジュールを作成しました",
            VoiceActionVerb::UpdateSetting => "設定を更新しました",
        })
    }

    fn god_agent_id() -> Option<String> {
        env::var("MD_GOD_AGENT_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .unwrap_or(0)
    }

    fn required<'a>(value: Option<&'a str>, message: &str) -> Result<&'a str, ServerFnError> {
        value
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| ServerFnError::new(message))
    }

    fn env_bool(name: &str, fallback: bool) -> bool {
        env::var(name)
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(fallback)
    }

    fn audio_error(error: AudioValidationError) -> ServerFnError {
        let message = match error {
            AudioValidationError::Empty => "audio is empty",
            AudioValidationError::TooLarge => "audio exceeds 25 MiB",
            AudioValidationError::UnsupportedMime => "audio MIME type is unsupported",
            AudioValidationError::LengthMismatch => "audio length does not match metadata",
        };
        ServerFnError::new(message)
    }

    fn policy_error(error: VoicePolicyError) -> ServerFnError {
        let message = match error {
            VoicePolicyError::MissingTarget => "操作対象がありません",
            VoicePolicyError::MassTargetForbidden => "全エージェント一括操作は禁止されています",
            VoicePolicyError::GodTargetForbidden => {
                "オーケストレーターへのこの操作は禁止されています"
            }
            VoicePolicyError::SettingForbidden => "この設定は音声操作できません",
            VoicePolicyError::InvalidSettingValue => "設定値が不正です",
            VoicePolicyError::MissingTypedInput => "操作に必要な入力が不足しています",
            VoicePolicyError::NoPendingAction => "確認待ちの操作はありません",
            VoicePolicyError::ConfirmationExpired => "操作確認の有効期限が切れました",
            VoicePolicyError::ConfirmationMismatch => "確認フレーズが一致しません",
        };
        ServerFnError::new(message)
    }

    fn cancelled_result() -> ActionResult {
        ActionResult {
            ok: true,
            spoken: String::from("操作をキャンセルしました"),
            needs_confirm: false,
            pending_id: None,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{VoiceServerState, env_bool, https_ready};

        #[test]
        fn freeflow_config_exposes_only_key_presence() -> Result<(), serde_json::Error> {
            let state = VoiceServerState::from_environment();
            let encoded = serde_json::to_string(&state.freeflow_config(true))?;

            assert!(encoded.contains("hasGroqKey"));
            assert!(!encoded.contains("secret"));
            Ok(())
        }

        #[test]
        fn https_status_requires_enablement_and_both_files() {
            assert!(!https_ready(false, true, true));
            assert!(!https_ready(true, true, false));
            assert!(https_ready(true, true, true));
        }

        #[test]
        fn missing_boolean_uses_fallback() {
            assert!(env_bool("MD_TEST_VOICE_MISSING_BOOLEAN", true));
        }
    }
}
