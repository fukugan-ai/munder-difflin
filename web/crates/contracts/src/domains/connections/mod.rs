//! Browser-safe contracts for connections, inbound triggers, and schedules.
//!
//! Persisted secrets are deliberately absent from every view and snapshot. A
//! [`WriteOnlySecret`] may cross from a form to a server function, but no read
//! contract can return it. Webhook rotation is the sole exception: the freshly
//! minted value is returned once as [`OneTimeSecret`].

use std::fmt::{Debug, Display, Formatter};

use serde::{Deserialize, Serialize};

pub const DEFAULT_SLACK_PORT: u16 = 3_847;
pub const DEFAULT_WEBHOOK_PORT: u16 = 3_849;
pub const DEFAULT_BROKER_PORT: u16 = 3_851;
pub const TRIGGER_HISTORY_LIMIT: usize = 500;
pub const DEFAULT_WEBHOOK_SCHEMA: &str = r#"{
  "type": "object",
  "required": ["message"],
  "properties": {
    "message": { "type": "string" },
    "title": { "type": "string" },
    "kind": { "type": "string", "enum": ["directive", "communication"] },
    "from": { "type": "string" }
  }
}"#;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RuntimeStatus {
    Stopped,
    Starting,
    Running,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ListenerStatus {
    pub state: RuntimeStatus,
    pub public_url: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct WriteOnlySecret(String);

impl WriteOnlySecret {
    pub fn new(value: String) -> Result<Self, ContractValidationError> {
        if value.trim().is_empty() {
            return Err(ContractValidationError::MissingField("secret"));
        }
        Ok(Self(value))
    }

    pub fn expose_for_server(&self) -> &str {
        &self.0
    }
}

impl Debug for WriteOnlySecret {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WriteOnlySecret([REDACTED])")
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct OneTimeSecret(String);

impl OneTimeSecret {
    pub fn from_server(value: String) -> Result<Self, ContractValidationError> {
        if value.is_empty() {
            return Err(ContractValidationError::MissingField("secret"));
        }
        Ok(Self(value))
    }

    pub fn reveal_once(&self) -> &str {
        &self.0
    }
}

impl Debug for OneTimeSecret {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OneTimeSecret([REDACTED])")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SlackConfigView {
    pub enabled: bool,
    pub has_signing_secret: bool,
    pub has_bot_token: bool,
    pub channel_id: Option<String>,
    pub port: u16,
    pub proactive_posting: bool,
    pub listener: ListenerStatus,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SlackConfigPatch {
    pub enabled: Option<bool>,
    pub channel_id: Option<String>,
    pub port: Option<u16>,
    pub proactive_posting: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SlackSecretKind {
    SigningSecret,
    BotToken,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SlackSecretWrite {
    pub kind: SlackSecretKind,
    pub secret: WriteOnlySecret,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IntegrationKind {
    Github,
    CustomRest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum IntegrationAuthType {
    None,
    Bearer,
    Header,
    Github,
}

impl IntegrationAuthType {
    pub const fn needs_secret(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IntegrationView {
    pub id: String,
    pub label: String,
    pub kind: IntegrationKind,
    pub base_url: String,
    pub auth_type: IntegrationAuthType,
    pub auth_header: Option<String>,
    pub enabled: bool,
    pub has_secret: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IntegrationUpsert {
    pub id: String,
    pub label: String,
    pub kind: IntegrationKind,
    pub base_url: String,
    pub auth_type: IntegrationAuthType,
    pub auth_header: Option<String>,
    pub enabled: bool,
}

impl IntegrationUpsert {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if !valid_slug(&self.id) {
            return Err(ContractValidationError::InvalidField("id"));
        }
        if self.label.trim().is_empty() || self.label.chars().count() > 60 {
            return Err(ContractValidationError::InvalidField("label"));
        }
        if !valid_base_url(&self.base_url) {
            return Err(ContractValidationError::InvalidField("base_url"));
        }
        match self.auth_type {
            IntegrationAuthType::Header if !valid_header(self.auth_header.as_deref()) => {
                Err(ContractValidationError::InvalidField("auth_header"))
            }
            IntegrationAuthType::Header => Ok(()),
            _ if self
                .auth_header
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()) =>
            {
                Err(ContractValidationError::InvalidField("auth_header"))
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IntegrationTemplate {
    pub id_suggestion: String,
    pub label: String,
    pub kind: IntegrationKind,
    pub base_url: String,
    pub auth_type: IntegrationAuthType,
    pub auth_header: Option<String>,
    pub secret_label: Option<String>,
    pub help: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TriggerMode {
    Strict,
    AllowAll,
    CommunicationOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum InboundKind {
    Directive,
    Communication,
}

impl TriggerMode {
    pub const fn permits(self, kind: InboundKind) -> bool {
        matches!(self, Self::AllowAll)
            || matches!(
                (self, kind),
                (Self::CommunicationOnly, InboundKind::Communication)
            )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextRule {
    pub enabled: bool,
    pub every_ms: u64,
    pub min_context_pct: u8,
    pub min_context_pct_large_window: u8,
    pub message: String,
}

impl ContextRule {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if !(60_000..=86_400_000).contains(&self.every_ms) {
            return Err(ContractValidationError::InvalidField("every_ms"));
        }
        if self.min_context_pct > 100 || self.min_context_pct_large_window > 100 {
            return Err(ContractValidationError::InvalidField("context_pct"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContextTriggerConfig {
    pub compact: ContextRule,
    pub clear: ContextRule,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub has_secret: bool,
    pub mode: TriggerMode,
    pub schema: String,
    pub created_at_ms: u64,
    pub endpoint_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookUpsert {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub mode: TriggerMode,
    pub schema: String,
}

impl WebhookUpsert {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(ContractValidationError::InvalidField("id"));
        }
        if self.name.trim().is_empty() {
            return Err(ContractValidationError::InvalidField("name"));
        }
        if self.schema.trim().is_empty() {
            return Err(ContractValidationError::InvalidField("schema"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookSecretWrite {
    pub webhook_id: String,
    pub secret: WriteOnlySecret,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WebhookCreateResult {
    pub webhook: WebhookView,
    pub secret: OneTimeSecret,
    pub snapshot: ConnectionsSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrgTriggerView {
    pub enabled: bool,
    pub mode: TriggerMode,
    pub has_api_key: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TriggerSource {
    Webhook,
    Organisation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TriggerDirection {
    Inbound,
    Outbound,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum TriggerDecision {
    AutoAllowed,
    Pending,
    Approved,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TriggerHistoryEntry {
    pub id: String,
    pub source: TriggerSource,
    pub source_id: String,
    pub source_name: String,
    pub direction: TriggerDirection,
    pub peer: String,
    pub title: Option<String>,
    pub body: String,
    pub kind: InboundKind,
    pub decision: Option<TriggerDecision>,
    pub correlation_id: Option<String>,
    pub task_id: Option<String>,
    pub at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WeeklySchedule {
    pub days: Vec<u8>,
    pub minute: u16,
}

impl WeeklySchedule {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.days.is_empty() || self.days.iter().any(|day| *day > 6) || self.minute >= 24 * 60 {
            return Err(ContractValidationError::InvalidField("weekly"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MissionKind {
    Dispatch,
    Heartbeat,
    Compact,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScheduledMission {
    pub id: String,
    pub label: String,
    pub interval_ms: u64,
    pub weekly: Option<WeeklySchedule>,
    pub to: String,
    pub body: String,
    pub enabled: bool,
    pub last_fired_at_ms: Option<u64>,
    pub kind: MissionKind,
    pub quiet_threshold_ms: Option<u64>,
}

impl ScheduledMission {
    pub fn validate(&self) -> Result<(), ContractValidationError> {
        if self.id.trim().is_empty() || self.label.trim().is_empty() || self.to.trim().is_empty() {
            return Err(ContractValidationError::MissingField("mission"));
        }
        if self.interval_ms < 60_000 {
            return Err(ContractValidationError::InvalidField("interval_ms"));
        }
        if let Some(weekly) = &self.weekly {
            weekly.validate()?;
        }
        if matches!(self.kind, MissionKind::Dispatch) && self.body.trim().is_empty() {
            return Err(ContractValidationError::MissingField("body"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionsSnapshot {
    pub slack: SlackConfigView,
    pub webhook_listener: ListenerStatus,
    pub integrations: Vec<IntegrationView>,
    pub integration_templates: Vec<IntegrationTemplate>,
    pub webhooks: Vec<WebhookView>,
    pub context: ContextTriggerConfig,
    pub organisation: OrgTriggerView,
    pub trigger_history: Vec<TriggerHistoryEntry>,
    pub missions: Vec<ScheduledMission>,
    /// Loopback-only credential broker. Its URL is safe to show; capability
    /// tokens and upstream credentials are never part of this snapshot.
    pub broker: ListenerStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ConnectionOperationStatus {
    Applied,
    MissingConfiguration,
    TransportUnavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionOperationResult {
    pub status: ConnectionOperationStatus,
    pub detail: String,
    pub snapshot: ConnectionsSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrokerStartResult {
    pub operation: ConnectionOperationResult,
    /// One-time admin capability covering the integrations enabled at start.
    /// Workers should receive narrower capabilities through the service hook.
    pub capability: OneTimeSecret,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ConnectionEvent {
    SlackIncoming {
        channel: String,
        thread_ts: String,
        text: String,
    },
    SlackStatusChanged(ListenerStatus),
    WebhookStatusChanged(ListenerStatus),
    ContextTriggerDue {
        action: ContextAction,
        rule: ContextRule,
    },
    TriggerHistoryUpdated,
    MissionsUpdated,
    MissionDue(ScheduledMission),
    IntegrationUpdated {
        id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ContextAction {
    Compact,
    Clear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContractValidationError {
    MissingField(&'static str),
    InvalidField(&'static str),
}

impl Display for ContractValidationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingField(field) => write!(formatter, "missing field: {field}"),
            Self::InvalidField(field) => write!(formatter, "invalid field: {field}"),
        }
    }
}

impl std::error::Error for ContractValidationError {}

fn valid_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    (2..=40).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_header(value: Option<&str>) -> bool {
    value.is_some_and(|header| {
        !header.is_empty()
            && header.len() <= 64
            && header
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn valid_base_url(value: &str) -> bool {
    let value = value.trim();
    if value.contains("..") || value.contains('@') || value.contains('#') || value.contains('?') {
        return false;
    }
    value.starts_with("https://")
        || value.starts_with("http://127.0.0.1")
        || value.starts_with("http://localhost")
        || value.starts_with("http://[::1]")
}

#[cfg(test)]
mod tests {
    use super::{
        ContextRule, ContractValidationError, InboundKind, IntegrationAuthType, IntegrationKind,
        IntegrationUpsert, MissionKind, OneTimeSecret, ScheduledMission, TriggerMode,
        WebhookUpsert, WeeklySchedule, WriteOnlySecret,
    };

    #[test]
    fn write_only_secret_rejects_blank_input() {
        assert_eq!(
            WriteOnlySecret::new(String::from("  ")),
            Err(ContractValidationError::MissingField("secret"))
        );
    }

    #[test]
    fn secret_debug_output_is_redacted() -> Result<(), ContractValidationError> {
        let secret = WriteOnlySecret::new(String::from("never-print-this"))?;

        assert_eq!(format!("{secret:?}"), "WriteOnlySecret([REDACTED])");
        Ok(())
    }

    #[test]
    fn one_time_secret_is_revealable_but_not_debuggable() -> Result<(), ContractValidationError> {
        let secret = OneTimeSecret::from_server(String::from("copy-once"))?;

        assert_eq!(secret.reveal_once(), "copy-once");
        assert_eq!(format!("{secret:?}"), "OneTimeSecret([REDACTED])");
        Ok(())
    }

    #[test]
    fn communication_only_rejects_directives() {
        assert!(!TriggerMode::CommunicationOnly.permits(InboundKind::Directive));
    }

    #[test]
    fn integration_requires_https_or_loopback() {
        let value = IntegrationUpsert {
            id: String::from("remote-api"),
            label: String::from("Remote API"),
            kind: IntegrationKind::CustomRest,
            base_url: String::from("http://example.com"),
            auth_type: IntegrationAuthType::Bearer,
            auth_header: None,
            enabled: true,
        };

        assert_eq!(
            value.validate(),
            Err(ContractValidationError::InvalidField("base_url"))
        );
    }

    #[test]
    fn webhook_rejects_nested_route_ids() {
        let value = WebhookUpsert {
            id: String::from("nested/path"),
            name: String::from("bad"),
            enabled: false,
            mode: TriggerMode::Strict,
            schema: String::from("{}"),
        };

        assert_eq!(
            value.validate(),
            Err(ContractValidationError::InvalidField("id"))
        );
    }

    #[test]
    fn context_rule_enforces_minimum_cadence() {
        let rule = ContextRule {
            enabled: true,
            every_ms: 59_999,
            min_context_pct: 60,
            min_context_pct_large_window: 40,
            message: String::new(),
        };

        assert_eq!(
            rule.validate(),
            Err(ContractValidationError::InvalidField("every_ms"))
        );
    }

    #[test]
    fn weekly_schedule_rejects_invalid_day() {
        let schedule = WeeklySchedule {
            days: vec![7],
            minute: 540,
        };

        assert_eq!(
            schedule.validate(),
            Err(ContractValidationError::InvalidField("weekly"))
        );
    }

    #[test]
    fn dispatch_mission_requires_body() {
        let mission = ScheduledMission {
            id: String::from("daily"),
            label: String::from("Daily"),
            interval_ms: 60_000,
            weekly: None,
            to: String::from("god"),
            body: String::new(),
            enabled: true,
            last_fired_at_ms: None,
            kind: MissionKind::Dispatch,
            quiet_threshold_ms: None,
        };

        assert_eq!(
            mission.validate(),
            Err(ContractValidationError::MissingField("body"))
        );
    }
}
