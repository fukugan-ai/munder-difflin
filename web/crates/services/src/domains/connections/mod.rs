//! Server-only state and explicitly-started transports for the connections domain.

mod durable;
mod runtime;
mod scheduler;

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::{Arc, OnceLock};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use md_web_contracts::domains::connections::{
    ConnectionEvent, ConnectionsSnapshot, ContextTriggerConfig, ContractValidationError,
    DEFAULT_BROKER_PORT, DEFAULT_SLACK_PORT, DEFAULT_WEBHOOK_PORT, IntegrationAuthType,
    IntegrationKind, IntegrationTemplate, IntegrationUpsert, IntegrationView, ListenerStatus,
    OneTimeSecret, OrgTriggerView, RuntimeStatus, ScheduledMission, SlackConfigPatch,
    SlackConfigView, SlackSecretKind, SlackSecretWrite, TRIGGER_HISTORY_LIMIT, TriggerDecision,
    TriggerHistoryEntry, TriggerSource, WebhookSecretWrite, WebhookUpsert, WebhookView,
    WriteOnlySecret,
};

pub use durable::HydrationPlan;
pub use runtime::{
    HttpRequest, HttpResponse, InboundDispatch, InboundWork, SlackDispatch, SlackInbound,
};
pub use scheduler::{AutomationBatch, ContextUsageSample};

static CONTEXT_USAGE_PROVIDER: OnceLock<Arc<dyn ContextUsageProvider>> = OnceLock::new();

/// Server-only source of current agent context pressure. The agent runtime
/// installs this once during boot; no usage samples cross the WASM boundary.
pub trait ContextUsageProvider: Send + Sync {
    fn samples(&self) -> Vec<ContextUsageSample>;
}

pub fn install_context_usage_provider(
    provider: Arc<dyn ContextUsageProvider>,
) -> Result<(), ConnectionsServiceError> {
    CONTEXT_USAGE_PROVIDER
        .set(provider)
        .map_err(|_| ConnectionsServiceError::InvalidInput("context usage provider"))
}

#[must_use]
pub fn context_usage_samples() -> Vec<ContextUsageSample> {
    CONTEXT_USAGE_PROVIDER
        .get()
        .map_or_else(Vec::new, |provider| provider.samples())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerForwardResult {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

const DEFAULT_COMPACTION_FOCUS: &str = "Keep the current task, recent decisions, open questions, and file paths in play. Drop resolved tangents.";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExternalEffect {
    StartSlackListener,
    StopSlackListener,
    StartWebhookListener,
    StopWebhookListener,
    ProbeIntegration {
        id: String,
        path: String,
    },
    PostSlackReply {
        channel: String,
        thread_ts: String,
        text: String,
    },
}

/// Typed server-only credential identity shared by connection and voice
/// adapters. This type is deliberately not serializable into browser DTOs.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SecretId {
    SlackSigning,
    SlackBot,
    Webhook(String),
    Integration(String),
    Organisation,
    Provider(ProviderSecretId),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ProviderSecretId {
    OpenAi,
    Groq,
}

/// A plaintext value that can exist only in server code. Debug output is always
/// redacted and the type has no serde implementation.
#[derive(Clone, Eq, PartialEq)]
pub struct ServerSecret(String);

impl ServerSecret {
    #[must_use]
    pub fn expose_for_server(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ServerSecret {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ServerSecret([REDACTED])")
    }
}

/// Server-only shared secret interface. Implementations must never log or
/// serialize values, and persistence must encrypt them before durable storage.
pub trait SecretProvider: Send + Sync {
    fn get_secret(&self, id: &SecretId) -> Result<Option<ServerSecret>, ConnectionsServiceError>;
    fn set_secret(
        &self,
        id: SecretId,
        secret: &WriteOnlySecret,
    ) -> Result<(), ConnectionsServiceError>;
    fn clear_secret(&self, id: &SecretId) -> Result<(), ConnectionsServiceError>;
    fn has_secret(&self, id: &SecretId) -> Result<bool, ConnectionsServiceError>;
}

/// Non-secret Voice preferences persisted with the same PostgreSQL CAS record.
/// Nested `Option` for the cap distinguishes “not hydrated/configured” from an
/// explicitly cleared cap.
#[derive(Clone, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct VoiceDurableSettings {
    pub freeflow_enabled: Option<bool>,
    pub freeflow_model: Option<String>,
    pub realtime_cost_cap_microusd: Option<Option<u64>>,
}

#[derive(Default)]
struct SecretStore {
    values: HashMap<SecretId, String>,
}

impl SecretStore {
    fn set(&mut self, key: SecretId, secret: &WriteOnlySecret) {
        self.values
            .insert(key, String::from(secret.expose_for_server()));
    }

    fn contains(&self, key: &SecretId) -> bool {
        self.values.contains_key(key)
    }

    fn remove(&mut self, key: &SecretId) {
        self.values.remove(key);
    }

    fn get(&self, key: &SecretId) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }
}

struct DomainState {
    slack: SlackConfigView,
    webhook_listener: ListenerStatus,
    integrations: Vec<IntegrationView>,
    webhooks: Vec<WebhookView>,
    context: ContextTriggerConfig,
    organisation: OrgTriggerView,
    trigger_history: Vec<TriggerHistoryEntry>,
    missions: Vec<ScheduledMission>,
    broker: ListenerStatus,
    broker_capabilities: HashMap<String, Vec<String>>,
    context_last_fired: [Option<u64>; 2],
    voice_settings: VoiceDurableSettings,
    secrets: SecretStore,
}

impl DomainState {
    fn snapshot(&self) -> ConnectionsSnapshot {
        ConnectionsSnapshot {
            slack: self.slack.clone(),
            webhook_listener: self.webhook_listener.clone(),
            integrations: self.integrations.clone(),
            integration_templates: integration_templates(),
            webhooks: self.webhooks.clone(),
            context: self.context.clone(),
            organisation: self.organisation.clone(),
            trigger_history: self.trigger_history.clone(),
            missions: self.missions.clone(),
            broker: self.broker.clone(),
        }
    }

    fn apply_snapshot(&mut self, snapshot: ConnectionsSnapshot) {
        self.slack = snapshot.slack;
        self.webhook_listener = snapshot.webhook_listener;
        self.integrations = snapshot.integrations;
        self.webhooks = snapshot.webhooks;
        self.context = snapshot.context;
        self.organisation = snapshot.organisation;
        self.trigger_history = snapshot.trigger_history;
        self.missions = snapshot.missions;
        self.broker = snapshot.broker;
    }

    fn sync_secret_flags(&mut self) {
        self.slack.has_signing_secret = self.secrets.contains(&SecretId::SlackSigning);
        self.slack.has_bot_token = self.secrets.contains(&SecretId::SlackBot);
        self.organisation.has_api_key = self.secrets.contains(&SecretId::Organisation);
        if !self.organisation.has_api_key {
            self.organisation.enabled = false;
        }
        for webhook in &mut self.webhooks {
            webhook.has_secret = self
                .secrets
                .contains(&SecretId::Webhook(webhook.id.clone()));
            if !webhook.has_secret {
                webhook.enabled = false;
            }
        }
        for integration in &mut self.integrations {
            integration.has_secret = self
                .secrets
                .contains(&SecretId::Integration(integration.id.clone()));
        }
    }
}

/// In-process owner for connection metadata. Raw persisted secrets never appear
/// in [`ConnectionsSnapshot`]; they remain in the private server-side store.
pub struct ConnectionsService {
    state: RwLock<DomainState>,
}

/// Returns the process-lifetime connections service used by every Server
/// Function. Keeping the singleton in the domain avoids separate snapshot and
/// mutation endpoints accidentally operating on different in-memory stores.
pub fn connections_service() -> &'static ConnectionsService {
    static SERVICE: OnceLock<ConnectionsService> = OnceLock::new();
    SERVICE.get_or_init(ConnectionsService::new)
}

impl ConnectionsService {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(default_state()),
        }
    }

    pub fn snapshot(&self) -> Result<ConnectionsSnapshot, ConnectionsServiceError> {
        Ok(self.read()?.snapshot())
    }

    /// Clears every process-local projection, including plaintext secrets and
    /// ephemeral broker capabilities. Used only after runtimes are drained for
    /// a committed namespace reset lifecycle.
    pub fn reset_projection(&self) -> Result<(), ConnectionsServiceError> {
        *self.write()? = default_state();
        Ok(())
    }

    pub fn export_durable_metadata(&self) -> Result<String, ConnectionsServiceError> {
        let state = self.read()?;
        durable::encode_metadata(&state)
    }

    pub fn hydrate_durable_metadata(
        &self,
        encoded: &str,
    ) -> Result<HydrationPlan, ConnectionsServiceError> {
        let mut state = self.write()?;
        durable::hydrate_metadata(&mut state, encoded)
    }

    pub fn export_encrypted_secrets(
        &self,
        master_key: &[u8],
    ) -> Result<String, ConnectionsServiceError> {
        let state = self.read()?;
        durable::seal_secrets(&state, master_key)
    }

    pub fn hydrate_encrypted_secrets(
        &self,
        master_key: &[u8],
        encoded: &str,
    ) -> Result<(), ConnectionsServiceError> {
        let mut state = self.write()?;
        durable::open_secrets(&mut state, master_key, encoded)
    }

    pub fn secret_count(&self) -> Result<usize, ConnectionsServiceError> {
        Ok(self.read()?.secrets.values.len())
    }

    pub fn voice_settings(&self) -> Result<VoiceDurableSettings, ConnectionsServiceError> {
        Ok(self.read()?.voice_settings.clone())
    }

    pub fn update_voice_settings(
        &self,
        settings: VoiceDurableSettings,
    ) -> Result<VoiceDurableSettings, ConnectionsServiceError> {
        if settings
            .freeflow_model
            .as_deref()
            .is_some_and(|model| model.trim().is_empty() || model.len() > 200)
        {
            return Err(ConnectionsServiceError::InvalidInput("voice model"));
        }
        let mut state = self.write()?;
        state.voice_settings = settings;
        Ok(state.voice_settings.clone())
    }

    pub fn poll_automations(
        &self,
        now_ms: u64,
        context_usage: &[ContextUsageSample],
    ) -> Result<AutomationBatch, ConnectionsServiceError> {
        let mut state = self.write()?;
        Ok(scheduler::poll(&mut state, now_ms, context_usage))
    }

    pub fn update_slack_config(
        &self,
        patch: SlackConfigPatch,
    ) -> Result<SlackConfigView, ConnectionsServiceError> {
        let mut state = self.write()?;
        if matches!(patch.port, Some(0)) {
            return Err(ConnectionsServiceError::InvalidInput("port"));
        }
        if let Some(enabled) = patch.enabled {
            state.slack.enabled = enabled;
        }
        if let Some(channel_id) = patch.channel_id {
            let channel_id = channel_id.trim();
            state.slack.channel_id = if channel_id.is_empty() {
                None
            } else {
                Some(String::from(channel_id))
            };
        }
        if let Some(port) = patch.port {
            state.slack.port = port;
        }
        if let Some(proactive_posting) = patch.proactive_posting {
            state.slack.proactive_posting = proactive_posting;
        }
        if !state.slack.enabled || !state.slack.has_signing_secret {
            state.slack.listener = stopped_listener();
        }
        Ok(state.slack.clone())
    }

    pub fn write_slack_secret(
        &self,
        request: &SlackSecretWrite,
    ) -> Result<SlackConfigView, ConnectionsServiceError> {
        let mut state = self.write()?;
        match request.kind {
            SlackSecretKind::SigningSecret => {
                state.secrets.set(SecretId::SlackSigning, &request.secret);
                state.slack.has_signing_secret = true;
            }
            SlackSecretKind::BotToken => {
                state.secrets.set(SecretId::SlackBot, &request.secret);
                state.slack.has_bot_token = true;
            }
        }
        Ok(state.slack.clone())
    }

    pub fn clear_slack_secret(
        &self,
        kind: SlackSecretKind,
    ) -> Result<SlackConfigView, ConnectionsServiceError> {
        let mut state = self.write()?;
        match kind {
            SlackSecretKind::SigningSecret => {
                state.secrets.remove(&SecretId::SlackSigning);
                state.slack.has_signing_secret = false;
                state.slack.listener = stopped_listener();
            }
            SlackSecretKind::BotToken => {
                state.secrets.remove(&SecretId::SlackBot);
                state.slack.has_bot_token = false;
            }
        }
        Ok(state.slack.clone())
    }

    pub fn request_start_slack(&self) -> Result<ExternalEffect, ConnectionsServiceError> {
        let state = self.read()?;
        if !state.slack.enabled {
            return Err(ConnectionsServiceError::FeatureDisabled("slack"));
        }
        if !state.secrets.contains(&SecretId::SlackSigning) {
            return Err(ConnectionsServiceError::MissingSecret(
                "slack signing secret",
            ));
        }
        Ok(ExternalEffect::StartSlackListener)
    }

    pub const fn request_stop_slack(&self) -> ExternalEffect {
        ExternalEffect::StopSlackListener
    }

    pub fn apply_slack_listener_status(
        &self,
        status: ListenerStatus,
    ) -> Result<ConnectionEvent, ConnectionsServiceError> {
        let mut state = self.write()?;
        state.slack.listener = status.clone();
        Ok(ConnectionEvent::SlackStatusChanged(status))
    }

    /// Start the signed Slack Events listener and its Tunnelmole child. This is
    /// called only by the explicit Start Server Function.
    pub fn start_slack_runtime(
        &'static self,
        dispatch: SlackDispatch,
    ) -> Result<ListenerStatus, ConnectionsServiceError> {
        self.request_start_slack()?;
        let state = self.read()?;
        let signing_secret = state
            .secrets
            .get(&SecretId::SlackSigning)
            .ok_or(ConnectionsServiceError::MissingSecret(
                "slack signing secret",
            ))?
            .to_owned();
        let config = runtime::SlackRuntimeConfig {
            port: state.slack.port,
            signing_secret,
            channel_id: state.slack.channel_id.clone(),
        };
        drop(state);
        let started =
            runtime::start_slack(config, dispatch).map_err(ConnectionsServiceError::Runtime)?;
        let status = ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: started.public_url,
            detail: started.detail,
        };
        self.apply_slack_listener_status(status.clone())?;
        Ok(status)
    }

    pub fn stop_slack_runtime(&self) -> Result<ListenerStatus, ConnectionsServiceError> {
        runtime::stop(runtime::RuntimeKind::Slack).map_err(ConnectionsServiceError::Runtime)?;
        let status = stopped_listener();
        self.apply_slack_listener_status(status.clone())?;
        Ok(status)
    }

    pub fn request_slack_reply(
        &self,
        channel: String,
        thread_ts: String,
        text: String,
    ) -> Result<ExternalEffect, ConnectionsServiceError> {
        let state = self.read()?;
        if !state.slack.proactive_posting {
            return Err(ConnectionsServiceError::FeatureDisabled(
                "slack proactive posting",
            ));
        }
        if !state.secrets.contains(&SecretId::SlackBot) {
            return Err(ConnectionsServiceError::MissingSecret("slack bot token"));
        }
        if channel.trim().is_empty() || thread_ts.trim().is_empty() || text.trim().is_empty() {
            return Err(ConnectionsServiceError::InvalidInput("slack reply"));
        }
        Ok(ExternalEffect::PostSlackReply {
            channel,
            thread_ts,
            text,
        })
    }

    pub fn upsert_integration(
        &self,
        request: &IntegrationUpsert,
        now_ms: u64,
    ) -> Result<IntegrationView, ConnectionsServiceError> {
        request
            .validate()
            .map_err(ConnectionsServiceError::Contract)?;
        let mut state = self.write()?;
        let position = state
            .integrations
            .iter()
            .position(|record| record.id == request.id);
        let has_secret = request.auth_type.needs_secret()
            && state
                .secrets
                .contains(&SecretId::Integration(request.id.clone()));
        let created_at_ms = position
            .and_then(|index| state.integrations.get(index))
            .map_or(now_ms, |record| record.created_at_ms);
        let record = IntegrationView {
            id: request.id.clone(),
            label: request.label.trim().into(),
            kind: request.kind,
            base_url: request.base_url.trim().into(),
            auth_type: request.auth_type,
            auth_header: request.auth_header.clone(),
            enabled: request.enabled,
            has_secret,
            created_at_ms,
            updated_at_ms: now_ms,
        };
        if let Some(index) = position {
            state.integrations[index] = record.clone();
        } else {
            state.integrations.push(record.clone());
        }
        Ok(record)
    }

    pub fn write_integration_secret(
        &self,
        id: &str,
        secret: &WriteOnlySecret,
        now_ms: u64,
    ) -> Result<IntegrationView, ConnectionsServiceError> {
        let mut state = self.write()?;
        let position = state
            .integrations
            .iter()
            .position(|record| record.id == id)
            .ok_or(ConnectionsServiceError::NotFound("integration"))?;
        if !state.integrations[position].auth_type.needs_secret() {
            return Err(ConnectionsServiceError::InvalidInput("integration auth"));
        }
        state
            .secrets
            .set(SecretId::Integration(id.to_owned()), secret);
        state.integrations[position].has_secret = true;
        state.integrations[position].updated_at_ms = now_ms;
        Ok(state.integrations[position].clone())
    }

    pub fn remove_integration(&self, id: &str) -> Result<bool, ConnectionsServiceError> {
        let mut state = self.write()?;
        let prior_len = state.integrations.len();
        state.integrations.retain(|record| record.id != id);
        state.secrets.remove(&SecretId::Integration(id.to_owned()));
        Ok(state.integrations.len() != prior_len)
    }

    pub fn request_integration_probe(
        &self,
        id: &str,
        path: &str,
    ) -> Result<ExternalEffect, ConnectionsServiceError> {
        let state = self.read()?;
        let record = state
            .integrations
            .iter()
            .find(|record| record.id == id)
            .ok_or(ConnectionsServiceError::NotFound("integration"))?;
        if !record.enabled {
            return Err(ConnectionsServiceError::FeatureDisabled("integration"));
        }
        if record.auth_type.needs_secret() && !record.has_secret {
            return Err(ConnectionsServiceError::MissingSecret("integration"));
        }
        if path.contains("..") || path.starts_with("//") || path.contains("://") {
            return Err(ConnectionsServiceError::InvalidInput("integration path"));
        }
        Ok(ExternalEffect::ProbeIntegration {
            id: String::from(id),
            path: String::from(path),
        })
    }

    /// Run a real HTTP reachability probe with the integration's server-held
    /// credential. The raw credential is never returned or included in errors.
    pub async fn probe_integration(
        &self,
        id: &str,
        path: &str,
    ) -> Result<String, ConnectionsServiceError> {
        self.request_integration_probe(id, path)?;
        let (record, secret) = {
            let state = self.read()?;
            let record = state
                .integrations
                .iter()
                .find(|record| record.id == id)
                .cloned()
                .ok_or(ConnectionsServiceError::NotFound("integration"))?;
            let secret = state
                .secrets
                .get(&SecretId::Integration(id.to_owned()))
                .map(String::from);
            (record, secret)
        };
        let base = record.base_url.trim_end_matches('/');
        let suffix = path.trim_start_matches('/');
        let url = if suffix.is_empty() {
            String::from(base)
        } else {
            format!("{base}/{suffix}")
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| ConnectionsServiceError::Transport("http client"))?;
        let mut request = client
            .get(url)
            .header("user-agent", "munder-difflin-web/0.1");
        if let Some(secret) = secret {
            request = match record.auth_type {
                IntegrationAuthType::Bearer | IntegrationAuthType::Github => {
                    request.bearer_auth(secret)
                }
                IntegrationAuthType::Header => {
                    request.header(record.auth_header.as_deref().unwrap_or("x-api-key"), secret)
                }
                IntegrationAuthType::None => request,
            };
        }
        let response = request
            .send()
            .await
            .map_err(|_| ConnectionsServiceError::Transport("integration probe"))?;
        Ok(format!("HTTP {}", response.status().as_u16()))
    }

    /// Mint a server-only broker capability. The value is returned once and is
    /// never included in snapshots, logs, or integration metadata.
    pub fn grant_broker_capability(
        &self,
        generated: String,
        integration_ids: Vec<String>,
    ) -> Result<OneTimeSecret, ConnectionsServiceError> {
        let mut state = self.write()?;
        if integration_ids.is_empty()
            || integration_ids.iter().any(|id| {
                !state
                    .integrations
                    .iter()
                    .any(|integration| integration.id == *id && integration.enabled)
            })
        {
            return Err(ConnectionsServiceError::InvalidInput("broker capability"));
        }
        state
            .broker_capabilities
            .insert(generated.clone(), integration_ids);
        OneTimeSecret::from_server(generated).map_err(ConnectionsServiceError::Contract)
    }

    pub fn start_broker_runtime(
        &'static self,
        handler: std::sync::Arc<dyn Fn(HttpRequest) -> HttpResponse + Send + Sync>,
    ) -> Result<ListenerStatus, ConnectionsServiceError> {
        let started = runtime::start_broker(DEFAULT_BROKER_PORT, handler)
            .map_err(ConnectionsServiceError::Runtime)?;
        let status = ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: started.public_url,
            detail: started.detail,
        };
        self.write()?.broker = status.clone();
        Ok(status)
    }

    pub fn stop_broker_runtime(&self) -> Result<ListenerStatus, ConnectionsServiceError> {
        runtime::stop(runtime::RuntimeKind::Broker).map_err(ConnectionsServiceError::Runtime)?;
        let mut state = self.write()?;
        state.broker = stopped_listener();
        state.broker_capabilities.clear();
        Ok(state.broker.clone())
    }

    pub async fn forward_broker_request(
        &self,
        capability: &str,
        method: &str,
        path: &str,
        body: Vec<u8>,
    ) -> Result<BrokerForwardResult, ConnectionsServiceError> {
        let (record, secret, upstream_path) = {
            let state = self.read()?;
            let allowed = state
                .broker_capabilities
                .get(capability)
                .ok_or(ConnectionsServiceError::InvalidInput("broker capability"))?;
            let mut segments = path.trim_start_matches('/').splitn(2, '/');
            let id = segments.next().unwrap_or_default();
            let upstream_path = segments.next().unwrap_or_default().to_owned();
            if !allowed.iter().any(|allowed_id| allowed_id == id) {
                return Err(ConnectionsServiceError::InvalidInput("broker capability"));
            }
            let record = state
                .integrations
                .iter()
                .find(|record| record.id == id && record.enabled)
                .cloned()
                .ok_or(ConnectionsServiceError::NotFound("integration"))?;
            let secret = state
                .secrets
                .get(&SecretId::Integration(id.to_owned()))
                .map(String::from);
            (record, secret, upstream_path)
        };
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| ConnectionsServiceError::InvalidInput("broker method"))?;
        let base = record.base_url.trim_end_matches('/');
        let url = if upstream_path.is_empty() {
            String::from(base)
        } else {
            format!("{base}/{upstream_path}")
        };
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| ConnectionsServiceError::Transport("broker client"))?;
        let mut request = client
            .request(method, url)
            .header("user-agent", "munder-difflin-broker/0.1")
            .body(body);
        if let Some(secret) = secret {
            request = match record.auth_type {
                IntegrationAuthType::Bearer | IntegrationAuthType::Github => {
                    request.bearer_auth(secret)
                }
                IntegrationAuthType::Header => {
                    request.header(record.auth_header.as_deref().unwrap_or("x-api-key"), secret)
                }
                IntegrationAuthType::None => request,
            };
        }
        let response = request
            .send()
            .await
            .map_err(|_| ConnectionsServiceError::Transport("broker upstream"))?;
        let status = response.status().as_u16();
        let content_type = if response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"))
        {
            "application/json; charset=utf-8"
        } else {
            "application/octet-stream"
        };
        let body = response
            .bytes()
            .await
            .map_err(|_| ConnectionsServiceError::Transport("broker response"))?
            .to_vec();
        Ok(BrokerForwardResult {
            status,
            content_type,
            body,
        })
    }

    pub fn upsert_webhook(
        &self,
        request: &WebhookUpsert,
        now_ms: u64,
    ) -> Result<WebhookView, ConnectionsServiceError> {
        request
            .validate()
            .map_err(ConnectionsServiceError::Contract)?;
        let mut state = self.write()?;
        let position = state
            .webhooks
            .iter()
            .position(|webhook| webhook.id == request.id);
        let has_secret = state
            .secrets
            .contains(&SecretId::Webhook(request.id.clone()));
        let created_at_ms = position
            .and_then(|index| state.webhooks.get(index))
            .map_or(now_ms, |webhook| webhook.created_at_ms);
        let view = WebhookView {
            id: request.id.clone(),
            name: request.name.trim().into(),
            enabled: request.enabled && has_secret,
            has_secret,
            mode: request.mode,
            schema: request.schema.clone(),
            created_at_ms,
            endpoint_url: endpoint_url(state.webhook_listener.public_url.as_deref(), &request.id),
        };
        if let Some(index) = position {
            state.webhooks[index] = view.clone();
        } else {
            state.webhooks.push(view.clone());
        }
        Ok(view)
    }

    pub fn write_webhook_secret(
        &self,
        request: &WebhookSecretWrite,
    ) -> Result<WebhookView, ConnectionsServiceError> {
        let mut state = self.write()?;
        let position = state
            .webhooks
            .iter()
            .position(|webhook| webhook.id == request.webhook_id)
            .ok_or(ConnectionsServiceError::NotFound("webhook"))?;
        state.secrets.set(
            SecretId::Webhook(request.webhook_id.clone()),
            &request.secret,
        );
        state.webhooks[position].has_secret = true;
        Ok(state.webhooks[position].clone())
    }

    pub fn apply_generated_webhook_secret(
        &self,
        webhook_id: &str,
        generated: String,
    ) -> Result<OneTimeSecret, ConnectionsServiceError> {
        let secret =
            WriteOnlySecret::new(generated.clone()).map_err(ConnectionsServiceError::Contract)?;
        let request = WebhookSecretWrite {
            webhook_id: String::from(webhook_id),
            secret,
        };
        self.write_webhook_secret(&request)?;
        OneTimeSecret::from_server(generated).map_err(ConnectionsServiceError::Contract)
    }

    pub fn remove_webhook(&self, id: &str) -> Result<bool, ConnectionsServiceError> {
        let mut state = self.write()?;
        let prior_len = state.webhooks.len();
        state.webhooks.retain(|webhook| webhook.id != id);
        state.secrets.remove(&SecretId::Webhook(id.to_owned()));
        Ok(state.webhooks.len() != prior_len)
    }

    pub fn request_start_webhooks(&self) -> Result<ExternalEffect, ConnectionsServiceError> {
        let state = self.read()?;
        if !state
            .webhooks
            .iter()
            .any(|webhook| webhook.enabled && webhook.has_secret)
        {
            return Err(ConnectionsServiceError::FeatureDisabled("webhooks"));
        }
        Ok(ExternalEffect::StartWebhookListener)
    }

    pub const fn request_stop_webhooks(&self) -> ExternalEffect {
        ExternalEffect::StopWebhookListener
    }

    pub fn apply_webhook_listener_status(
        &self,
        status: ListenerStatus,
    ) -> Result<ConnectionEvent, ConnectionsServiceError> {
        let mut state = self.write()?;
        state.webhook_listener = status.clone();
        for webhook in &mut state.webhooks {
            webhook.endpoint_url = endpoint_url(status.public_url.as_deref(), &webhook.id);
        }
        Ok(ConnectionEvent::WebhookStatusChanged(status))
    }

    pub fn start_webhook_runtime(
        &'static self,
        dispatch: InboundDispatch,
    ) -> Result<ListenerStatus, ConnectionsServiceError> {
        self.request_start_webhooks()?;
        let endpoints = {
            let state = self.read()?;
            state
                .webhooks
                .iter()
                .filter(|webhook| webhook.enabled && webhook.has_secret)
                .filter_map(|webhook| {
                    let secret = state.secrets.get(&SecretId::Webhook(webhook.id.clone()))?;
                    Some(runtime::WebhookRuntimeEndpoint {
                        id: webhook.id.clone(),
                        name: webhook.name.clone(),
                        secret: secret.to_owned(),
                        schema: webhook.schema.clone(),
                        mode: webhook.mode,
                    })
                })
                .collect()
        };
        let started = runtime::start_webhooks(DEFAULT_WEBHOOK_PORT, endpoints, dispatch)
            .map_err(ConnectionsServiceError::Runtime)?;
        let status = ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: started.public_url,
            detail: started.detail,
        };
        self.apply_webhook_listener_status(status.clone())?;
        Ok(status)
    }

    pub fn stop_webhook_runtime(&self) -> Result<ListenerStatus, ConnectionsServiceError> {
        runtime::stop(runtime::RuntimeKind::Webhooks).map_err(ConnectionsServiceError::Runtime)?;
        let status = stopped_listener();
        self.apply_webhook_listener_status(status.clone())?;
        Ok(status)
    }

    pub fn set_context(
        &self,
        context: ContextTriggerConfig,
    ) -> Result<ContextTriggerConfig, ConnectionsServiceError> {
        context
            .compact
            .validate()
            .map_err(ConnectionsServiceError::Contract)?;
        context
            .clear
            .validate()
            .map_err(ConnectionsServiceError::Contract)?;
        let mut state = self.write()?;
        state.context = context;
        Ok(state.context.clone())
    }

    pub fn set_organisation(
        &self,
        enabled: bool,
        mode: md_web_contracts::domains::connections::TriggerMode,
    ) -> Result<OrgTriggerView, ConnectionsServiceError> {
        let mut state = self.write()?;
        state.organisation.enabled = enabled && state.organisation.has_api_key;
        state.organisation.mode = mode;
        Ok(state.organisation.clone())
    }

    pub fn write_organisation_key(
        &self,
        secret: &WriteOnlySecret,
    ) -> Result<OrgTriggerView, ConnectionsServiceError> {
        let mut state = self.write()?;
        state.secrets.set(SecretId::Organisation, secret);
        state.organisation.has_api_key = true;
        Ok(state.organisation.clone())
    }

    pub fn append_history(
        &self,
        entry: TriggerHistoryEntry,
    ) -> Result<ConnectionEvent, ConnectionsServiceError> {
        let mut state = self.write()?;
        state.trigger_history.insert(0, entry);
        state.trigger_history.truncate(TRIGGER_HISTORY_LIMIT);
        Ok(ConnectionEvent::TriggerHistoryUpdated)
    }

    pub fn decide_history(
        &self,
        id: &str,
        decision: TriggerDecision,
        task_id: Option<String>,
    ) -> Result<TriggerHistoryEntry, ConnectionsServiceError> {
        if !matches!(
            decision,
            TriggerDecision::Approved | TriggerDecision::Rejected
        ) {
            return Err(ConnectionsServiceError::InvalidInput("decision"));
        }
        let mut state = self.write()?;
        let entry = state
            .trigger_history
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(ConnectionsServiceError::NotFound("trigger history"))?;
        if !matches!(entry.decision, Some(TriggerDecision::Pending)) {
            return Ok(entry.clone());
        }
        entry.decision = Some(decision);
        if matches!(decision, TriggerDecision::Approved) {
            entry.task_id = task_id;
        }
        Ok(entry.clone())
    }

    pub fn clear_history(
        &self,
        source: Option<TriggerSource>,
    ) -> Result<ConnectionEvent, ConnectionsServiceError> {
        let mut state = self.write()?;
        if let Some(source) = source {
            state.trigger_history.retain(|entry| entry.source != source);
        } else {
            state.trigger_history.clear();
        }
        Ok(ConnectionEvent::TriggerHistoryUpdated)
    }

    pub fn replace_missions(
        &self,
        incoming: Vec<ScheduledMission>,
    ) -> Result<ConnectionEvent, ConnectionsServiceError> {
        for mission in &incoming {
            mission
                .validate()
                .map_err(ConnectionsServiceError::Contract)?;
        }
        let mut state = self.write()?;
        let previous_last_fired: HashMap<String, u64> = state
            .missions
            .iter()
            .filter_map(|mission| {
                mission
                    .last_fired_at_ms
                    .map(|last_fired| (mission.id.clone(), last_fired))
            })
            .collect();
        state.missions = incoming
            .into_iter()
            .map(|mut mission| {
                let prior = previous_last_fired.get(&mission.id).copied().unwrap_or(0);
                let incoming = mission.last_fired_at_ms.unwrap_or(0);
                mission.last_fired_at_ms = match prior.max(incoming) {
                    0 => None,
                    value => Some(value),
                };
                mission
            })
            .collect();
        Ok(ConnectionEvent::MissionsUpdated)
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, DomainState>, ConnectionsServiceError> {
        self.state
            .read()
            .map_err(|_| ConnectionsServiceError::StateUnavailable)
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, DomainState>, ConnectionsServiceError> {
        self.state
            .write()
            .map_err(|_| ConnectionsServiceError::StateUnavailable)
    }
}

fn default_state() -> DomainState {
    DomainState {
        slack: SlackConfigView {
            enabled: false,
            has_signing_secret: false,
            has_bot_token: false,
            channel_id: None,
            port: DEFAULT_SLACK_PORT,
            proactive_posting: false,
            listener: stopped_listener(),
        },
        webhook_listener: stopped_listener(),
        integrations: Vec::new(),
        webhooks: Vec::new(),
        context: default_context(),
        organisation: OrgTriggerView {
            enabled: false,
            mode: md_web_contracts::domains::connections::TriggerMode::Strict,
            has_api_key: false,
        },
        trigger_history: Vec::new(),
        missions: Vec::new(),
        broker: stopped_listener(),
        broker_capabilities: HashMap::new(),
        context_last_fired: [None, None],
        voice_settings: VoiceDurableSettings::default(),
        secrets: SecretStore::default(),
    }
}

impl Default for ConnectionsService {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretProvider for ConnectionsService {
    fn get_secret(&self, id: &SecretId) -> Result<Option<ServerSecret>, ConnectionsServiceError> {
        let state = self.read()?;
        Ok(state
            .secrets
            .get(id)
            .map(|value| ServerSecret(value.to_owned())))
    }

    fn set_secret(
        &self,
        id: SecretId,
        secret: &WriteOnlySecret,
    ) -> Result<(), ConnectionsServiceError> {
        let mut state = self.write()?;
        state.secrets.set(id.clone(), secret);
        match id {
            SecretId::SlackSigning => state.slack.has_signing_secret = true,
            SecretId::SlackBot => state.slack.has_bot_token = true,
            SecretId::Organisation => state.organisation.has_api_key = true,
            SecretId::Webhook(id) => {
                if let Some(record) = state.webhooks.iter_mut().find(|record| record.id == id) {
                    record.has_secret = true;
                }
            }
            SecretId::Integration(id) => {
                if let Some(record) = state.integrations.iter_mut().find(|record| record.id == id) {
                    record.has_secret = true;
                }
            }
            SecretId::Provider(_) => {}
        }
        Ok(())
    }

    fn clear_secret(&self, id: &SecretId) -> Result<(), ConnectionsServiceError> {
        let mut state = self.write()?;
        state.secrets.remove(id);
        match id {
            SecretId::SlackSigning => {
                state.slack.has_signing_secret = false;
                state.slack.listener = stopped_listener();
            }
            SecretId::SlackBot => state.slack.has_bot_token = false,
            SecretId::Organisation => {
                state.organisation.has_api_key = false;
                state.organisation.enabled = false;
            }
            SecretId::Webhook(id) => {
                if let Some(record) = state.webhooks.iter_mut().find(|record| record.id == *id) {
                    record.has_secret = false;
                    record.enabled = false;
                }
            }
            SecretId::Integration(id) => {
                if let Some(record) = state
                    .integrations
                    .iter_mut()
                    .find(|record| record.id == *id)
                {
                    record.has_secret = false;
                }
            }
            SecretId::Provider(_) => {}
        }
        Ok(())
    }

    fn has_secret(&self, id: &SecretId) -> Result<bool, ConnectionsServiceError> {
        Ok(self.read()?.secrets.contains(id))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionsServiceError {
    Contract(ContractValidationError),
    InvalidInput(&'static str),
    MissingSecret(&'static str),
    FeatureDisabled(&'static str),
    NotFound(&'static str),
    Runtime(String),
    Transport(&'static str),
    InvalidData(&'static str),
    StateUnavailable,
}

impl Display for ConnectionsServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(error) => write!(formatter, "invalid contract: {error:?}"),
            Self::InvalidInput(field) => write!(formatter, "invalid input: {field}"),
            Self::MissingSecret(name) => write!(formatter, "missing secret: {name}"),
            Self::FeatureDisabled(name) => write!(formatter, "feature disabled: {name}"),
            Self::NotFound(name) => write!(formatter, "not found: {name}"),
            Self::Runtime(detail) => write!(formatter, "runtime unavailable: {detail}"),
            Self::Transport(name) => write!(formatter, "transport failed: {name}"),
            Self::InvalidData(name) => write!(formatter, "invalid durable data: {name}"),
            Self::StateUnavailable => formatter.write_str("connections state unavailable"),
        }
    }
}

impl std::error::Error for ConnectionsServiceError {}

fn stopped_listener() -> ListenerStatus {
    ListenerStatus {
        state: RuntimeStatus::Stopped,
        public_url: None,
        detail: None,
    }
}

fn default_context() -> ContextTriggerConfig {
    ContextTriggerConfig {
        compact: md_web_contracts::domains::connections::ContextRule {
            enabled: true,
            every_ms: 7_200_000,
            min_context_pct: 60,
            min_context_pct_large_window: 40,
            message: String::from(DEFAULT_COMPACTION_FOCUS),
        },
        clear: md_web_contracts::domains::connections::ContextRule {
            enabled: false,
            every_ms: 7_200_000,
            min_context_pct: 90,
            min_context_pct_large_window: 80,
            message: String::new(),
        },
    }
}

fn endpoint_url(base: Option<&str>, id: &str) -> Option<String> {
    let base = base?.trim_end_matches('/');
    Some(format!("{base}/{id}"))
}

fn integration_templates() -> Vec<IntegrationTemplate> {
    vec![
        IntegrationTemplate {
            id_suggestion: String::from("github"),
            label: String::from("GitHub"),
            kind: IntegrationKind::Github,
            base_url: String::from("https://api.github.com"),
            auth_type: IntegrationAuthType::Github,
            auth_header: None,
            secret_label: Some(String::from("GitHub personal access token")),
            help: String::from("GitHub REST APIへ接続します。"),
        },
        IntegrationTemplate {
            id_suggestion: String::from("custom-rest"),
            label: String::from("カスタムREST API"),
            kind: IntegrationKind::CustomRest,
            base_url: String::new(),
            auth_type: IntegrationAuthType::Bearer,
            auth_header: None,
            secret_label: Some(String::from("API key / token")),
            help: String::from("任意のHTTPS APIへ接続します。"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::connections::{
        InboundKind, IntegrationAuthType, IntegrationKind, IntegrationUpsert, ListenerStatus,
        MissionKind, RuntimeStatus, ScheduledMission, SlackConfigPatch, SlackSecretKind,
        SlackSecretWrite, TriggerDecision, TriggerDirection, TriggerHistoryEntry, TriggerMode,
        TriggerSource, WebhookSecretWrite, WebhookUpsert, WriteOnlySecret,
    };

    use super::{
        ConnectionsService, ConnectionsServiceError, ExternalEffect, SecretId, SecretProvider,
        VoiceDurableSettings, connections_service,
    };

    fn secret(
        value: &str,
    ) -> Result<WriteOnlySecret, md_web_contracts::domains::connections::ContractValidationError>
    {
        WriteOnlySecret::new(String::from(value))
    }

    #[test]
    fn snapshot_never_contains_secret_values() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.write_slack_secret(&SlackSecretWrite {
            kind: SlackSecretKind::SigningSecret,
            secret: secret("top-secret")?,
        })?;

        let encoded = format!("{:?}", service.snapshot()?);
        assert!(!encoded.contains("top-secret"));
        assert!(encoded.contains("has_signing_secret: true"));
        Ok(())
    }

    #[test]
    fn slack_start_requires_enable_and_secret() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        assert_eq!(
            service.request_start_slack(),
            Err(ConnectionsServiceError::FeatureDisabled("slack"))
        );
        service.update_slack_config(SlackConfigPatch {
            enabled: Some(true),
            ..SlackConfigPatch::default()
        })?;
        assert_eq!(
            service.request_start_slack(),
            Err(ConnectionsServiceError::MissingSecret(
                "slack signing secret"
            ))
        );
        service.write_slack_secret(&SlackSecretWrite {
            kind: SlackSecretKind::SigningSecret,
            secret: secret("signing")?,
        })?;
        assert_eq!(
            service.request_start_slack()?,
            ExternalEffect::StartSlackListener
        );
        Ok(())
    }

    #[test]
    fn listener_status_is_reflected_in_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.apply_slack_listener_status(ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: Some(String::from("https://example.tunnel")),
            detail: None,
        })?;

        assert_eq!(
            service.snapshot()?.slack.listener.state,
            RuntimeStatus::Running
        );
        Ok(())
    }

    #[test]
    fn integration_secret_is_write_only() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.upsert_integration(
            &IntegrationUpsert {
                id: String::from("github"),
                label: String::from("GitHub"),
                kind: IntegrationKind::Github,
                base_url: String::from("https://api.github.com"),
                auth_type: IntegrationAuthType::Github,
                auth_header: None,
                enabled: true,
            },
            1,
        )?;
        service.write_integration_secret("github", &secret("ghp-secret")?, 2)?;

        let record = &service.snapshot()?.integrations[0];
        assert!(record.has_secret);
        assert_eq!(record.updated_at_ms, 2);
        Ok(())
    }

    #[test]
    fn probe_rejects_origin_override() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.upsert_integration(
            &IntegrationUpsert {
                id: String::from("public-api"),
                label: String::from("Public API"),
                kind: IntegrationKind::CustomRest,
                base_url: String::from("https://api.example.com"),
                auth_type: IntegrationAuthType::None,
                auth_header: None,
                enabled: true,
            },
            1,
        )?;

        assert_eq!(
            service.request_integration_probe("public-api", "https://evil.example"),
            Err(ConnectionsServiceError::InvalidInput("integration path"))
        );
        Ok(())
    }

    #[test]
    fn webhook_enables_only_after_secret_is_written() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        let request = WebhookUpsert {
            id: String::from("build-hook"),
            name: String::from("Build hook"),
            enabled: true,
            mode: TriggerMode::Strict,
            schema: String::from("{}"),
        };
        let initial = service.upsert_webhook(&request, 1)?;
        assert!(!initial.enabled);
        service.write_webhook_secret(&WebhookSecretWrite {
            webhook_id: request.id.clone(),
            secret: secret("webhook-secret")?,
        })?;
        let enabled = service.upsert_webhook(&request, 2)?;
        assert!(enabled.enabled);
        Ok(())
    }

    #[test]
    fn generated_webhook_secret_is_returned_once() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.upsert_webhook(
            &WebhookUpsert {
                id: String::from("hook"),
                name: String::from("Hook"),
                enabled: false,
                mode: TriggerMode::Strict,
                schema: String::from("{}"),
            },
            1,
        )?;

        let minted = service.apply_generated_webhook_secret("hook", String::from("minted"))?;
        assert_eq!(minted.reveal_once(), "minted");
        assert!(!format!("{:?}", service.snapshot()?).contains("minted"));
        Ok(())
    }

    #[test]
    fn deciding_history_twice_does_not_change_the_first_verdict()
    -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.append_history(TriggerHistoryEntry {
            id: String::from("event-1"),
            source: TriggerSource::Webhook,
            source_id: String::from("hook"),
            source_name: String::from("Hook"),
            direction: TriggerDirection::Inbound,
            peer: String::from("ci"),
            title: None,
            body: String::from("build"),
            kind: InboundKind::Directive,
            decision: Some(TriggerDecision::Pending),
            correlation_id: None,
            task_id: None,
            at_ms: 1,
        })?;
        service.decide_history(
            "event-1",
            TriggerDecision::Approved,
            Some(String::from("task-1")),
        )?;
        let second = service.decide_history("event-1", TriggerDecision::Rejected, None)?;

        assert_eq!(second.decision, Some(TriggerDecision::Approved));
        assert_eq!(second.task_id.as_deref(), Some("task-1"));
        Ok(())
    }

    #[test]
    fn webhook_history_survives_process_projection_loss() -> Result<(), Box<dyn std::error::Error>>
    {
        let service = ConnectionsService::new();
        service.append_history(TriggerHistoryEntry {
            id: String::from("webhook-crash-safe"),
            source: TriggerSource::Webhook,
            source_id: String::from("hook"),
            source_name: String::from("Hook"),
            direction: TriggerDirection::Inbound,
            peer: String::from("ci"),
            title: Some(String::from("Build")),
            body: String::from("build now"),
            kind: InboundKind::Directive,
            decision: Some(TriggerDecision::AutoAllowed),
            correlation_id: None,
            task_id: Some(String::from("task-1")),
            at_ms: 42,
        })?;
        let durable = service.export_durable_metadata()?;
        service.reset_projection()?;
        assert!(service.snapshot()?.trigger_history.is_empty());

        service.hydrate_durable_metadata(&durable)?;
        assert_eq!(
            service.snapshot()?.trigger_history[0].id,
            "webhook-crash-safe"
        );
        Ok(())
    }

    #[test]
    fn mission_replace_preserves_newer_scheduler_stamp() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        let mission = |last_fired_at_ms| ScheduledMission {
            id: String::from("daily"),
            label: String::from("Daily"),
            interval_ms: 60_000,
            weekly: None,
            to: String::from("god"),
            body: String::from("status"),
            enabled: true,
            last_fired_at_ms,
            kind: MissionKind::Dispatch,
            quiet_threshold_ms: None,
        };
        service.replace_missions(vec![mission(Some(20))])?;
        service.replace_missions(vec![mission(Some(10))])?;

        assert_eq!(service.snapshot()?.missions[0].last_fired_at_ms, Some(20));
        Ok(())
    }

    #[test]
    fn process_service_is_stable() {
        assert!(std::ptr::eq(connections_service(), connections_service()));
    }

    #[test]
    fn restart_hydrates_metadata_and_sealed_secrets_without_client_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = ConnectionsService::new();
        original.update_slack_config(SlackConfigPatch {
            enabled: Some(true),
            channel_id: Some(String::from("C123")),
            ..SlackConfigPatch::default()
        })?;
        original.write_slack_secret(&SlackSecretWrite {
            kind: SlackSecretKind::SigningSecret,
            secret: secret("restart-only-secret")?,
        })?;
        original.apply_slack_listener_status(ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: Some(String::from("https://example.invalid")),
            detail: None,
        })?;
        original.upsert_integration(
            &IntegrationUpsert {
                id: String::from("restart-api"),
                label: String::from("Restart API"),
                kind: IntegrationKind::CustomRest,
                base_url: String::from("https://api.example.invalid"),
                auth_type: IntegrationAuthType::None,
                auth_header: None,
                enabled: true,
            },
            1,
        )?;
        original.write()?.broker = ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: Some(String::from("http://127.0.0.1:3851")),
            detail: None,
        };
        original.update_voice_settings(VoiceDurableSettings {
            freeflow_enabled: Some(true),
            freeflow_model: Some(String::from("whisper-restart")),
            realtime_cost_cap_microusd: Some(Some(25_000)),
        })?;
        let metadata = original.export_durable_metadata()?;
        let key = b"test-master-key-with-at-least-32-bytes";
        let sealed = original.export_encrypted_secrets(key)?;
        assert!(!metadata.contains("restart-only-secret"));
        assert!(!sealed.contains("restart-only-secret"));

        let restored = ConnectionsService::new();
        let plan = restored.hydrate_durable_metadata(&metadata)?;
        restored.hydrate_encrypted_secrets(key, &sealed)?;
        assert!(plan.restart_slack);
        assert!(plan.restart_broker);
        assert_eq!(
            restored.snapshot()?.slack.channel_id.as_deref(),
            Some("C123")
        );
        assert_eq!(
            restored
                .get_secret(&SecretId::SlackSigning)?
                .as_ref()
                .map(super::ServerSecret::expose_for_server),
            Some("restart-only-secret")
        );
        assert_eq!(
            restored.voice_settings()?,
            VoiceDurableSettings {
                freeflow_enabled: Some(true),
                freeflow_model: Some(String::from("whisper-restart")),
                realtime_cost_cap_microusd: Some(Some(25_000)),
            }
        );
        assert!(!format!("{:?}", restored.snapshot()?).contains("restart-only-secret"));
        Ok(())
    }

    #[test]
    fn encrypted_secret_tampering_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let source = ConnectionsService::new();
        source.set_secret(SecretId::SlackBot, &secret("sealed")?)?;
        let key = b"test-master-key-with-at-least-32-bytes";
        let sealed = source.export_encrypted_secrets(key)?;
        let mut envelope: serde_json::Value = serde_json::from_str(&sealed)?;
        let tag = envelope
            .get_mut("tag")
            .and_then(|value| value.as_str())
            .ok_or("missing tag")?;
        let replacement = if tag.starts_with('0') { "1" } else { "0" };
        let mut changed_tag = String::from(tag);
        changed_tag.replace_range(0..1, replacement);
        envelope["tag"] = serde_json::Value::String(changed_tag);
        let sealed = envelope.to_string();
        let restored = ConnectionsService::new();
        assert!(restored.hydrate_encrypted_secrets(key, &sealed).is_err());
        Ok(())
    }

    #[test]
    fn namespace_reset_clears_metadata_secrets_and_runtime_projection()
    -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.update_slack_config(SlackConfigPatch {
            enabled: Some(true),
            channel_id: Some(String::from("C-stale")),
            ..SlackConfigPatch::default()
        })?;
        service.set_secret(SecretId::SlackSigning, &secret("stale-secret")?)?;
        service.apply_slack_listener_status(ListenerStatus {
            state: RuntimeStatus::Running,
            public_url: Some(String::from("https://stale.invalid")),
            detail: None,
        })?;
        let metadata = service.export_durable_metadata()?;
        let key = b"test-master-key-with-at-least-32-bytes";
        let sealed = service.export_encrypted_secrets(key)?;

        service.reset_projection()?;

        let snapshot = service.snapshot()?;
        assert!(!snapshot.slack.enabled);
        assert!(snapshot.slack.channel_id.is_none());
        assert_eq!(snapshot.slack.listener.state, RuntimeStatus::Stopped);
        assert_eq!(service.secret_count()?, 0);
        assert!(!service.has_secret(&SecretId::SlackSigning)?);

        let plan = service.hydrate_durable_metadata(&metadata)?;
        service.hydrate_encrypted_secrets(key, &sealed)?;
        assert!(plan.restart_slack);
        assert_eq!(
            service.snapshot()?.slack.channel_id.as_deref(),
            Some("C-stale")
        );
        assert!(service.has_secret(&SecretId::SlackSigning)?);
        Ok(())
    }
}
