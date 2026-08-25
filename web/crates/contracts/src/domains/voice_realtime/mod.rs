#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

use crate::domains::connections::ScheduledMission;
use crate::domains::connections::WriteOnlySecret;
use crate::domains::pty_agents::SpawnAgentRequest;

pub const DEFAULT_FREEFLOW_MODEL: &str = "whisper-large-v3-turbo";
pub const DEFAULT_REALTIME_MODEL: &str = "gpt-realtime-2.1";
pub const MAX_AUDIO_BYTES: u64 = 25 * 1024 * 1024;
pub const DEFAULT_IDLE_DISCONNECT_MS: u64 = 180_000;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FreeflowStatus {
    #[default]
    Idle,
    Recording,
    Transcribing,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeflowSnapshot {
    pub status: FreeflowStatus,
    pub target_agent_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeflowConfig {
    pub enabled: bool,
    pub model: String,
    pub has_groq_key: bool,
}

impl Default for FreeflowConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::from(DEFAULT_FREEFLOW_MODEL),
            has_groq_key: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeflowConfigPatch {
    pub enabled: Option<bool>,
    pub model: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceProviderKey {
    Groq,
    OpenAi,
}

/// Browser-to-server only credential write. The secret's Debug representation is redacted,
/// and no credential value is ever returned by a voice DTO.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProviderKeyWrite {
    pub provider: VoiceProviderKey,
    pub secret: WriteOnlySecret,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceBootstrap {
    pub freeflow: FreeflowConfig,
    pub has_openai_key: bool,
    pub https_required_for_lan: bool,
    pub server_https_configured: bool,
    pub server_https_ready: bool,
    pub tls_cert_path_configured: bool,
    pub tls_key_path_configured: bool,
    pub realtime_model: String,
    pub idle_disconnect_ms: u64,
    pub realtime_cost: RealtimeCostSnapshot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionMetadata {
    pub byte_length: u64,
    pub mime_type: String,
    pub filename: String,
    pub language: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum TranscriptionResult {
    Ok {
        text: String,
    },
    Error {
        code: VoiceErrorCode,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeStatus {
    #[default]
    Off,
    Connecting,
    Listening,
    Responding,
    Working,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeSessionSnapshot {
    pub status: RealtimeStatus,
    pub error: Option<String>,
    pub muted: bool,
    pub model: Option<String>,
    pub expires_at: Option<i64>,
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    pub secure_context: bool,
    pub cost: RealtimeCostSnapshot,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeCostSnapshot {
    pub usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cap_usd: Option<f64>,
    pub over_cap: bool,
    pub last_activity_ms: Option<i64>,
    pub started_ms: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeMintRequest {
    pub model: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum RealtimeMintResult {
    Ok {
        ephemeral_token: String,
        expires_at: Option<i64>,
        model: String,
    },
    Error {
        code: VoiceErrorCode,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceView {
    pub id: String,
    pub label: String,
    pub kind: AudioDeviceKind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDeviceKind {
    Input,
    Output,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceActionVerb {
    Ping,
    Dispatch,
    Steer,
    CreateTask,
    AssignTask,
    UpdateTask,
    DeleteTask,
    WaitFor,
    Spawn,
    Kill,
    Pause,
    Halt,
    Resume,
    AutoDelivery,
    GateTool,
    Archive,
    Unarchive,
    ClearContext,
    EditSchedule,
    CreateSchedule,
    UpdateSetting,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeActionRequest {
    pub verb: VoiceActionVerb,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub setting_key: Option<String>,
    #[serde(default)]
    pub setting_value: Option<String>,
    #[serde(default)]
    pub spawn_request: Option<SpawnAgentRequest>,
    #[serde(default)]
    pub mission: Option<ScheduledMission>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    pub ok: bool,
    pub spoken: String,
    pub needs_confirm: bool,
    pub pending_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeCompletion {
    pub correlation_id: String,
    pub kind: CompletionKind,
    pub target_agent_id: String,
    pub task_id: Option<String>,
    pub summary: String,
    pub completed_at_ms: i64,
    pub objective: Option<String>,
    pub via: Option<CompletionVia>,
    pub message_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionKind {
    Dispatch,
    Task,
    Spawn,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionVia {
    CardDone,
    InboxReply,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FloorDelta {
    pub text: String,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeEnqueue {
    pub agent_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "payload")]
pub enum VoiceServerEvent {
    Completion(RealtimeCompletion),
    FloorDelta(FloorDelta),
    Enqueue(RealtimeEnqueue),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEventEnvelope {
    pub sequence: u64,
    pub event: VoiceServerEvent,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEventBatch {
    pub latest_sequence: u64,
    pub events: Vec<VoiceEventEnvelope>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceErrorCode {
    Disabled,
    NoKey,
    NoAudio,
    AudioTooLarge,
    InvalidMime,
    InsecureContext,
    PermissionDenied,
    Timeout,
    Upstream,
    InvalidAction,
    ConfirmationRequired,
    ConfirmationExpired,
}

#[cfg(test)]
mod tests {
    use crate::domains::connections::WriteOnlySecret;

    use super::{
        DEFAULT_FREEFLOW_MODEL, FreeflowConfig, RealtimeActionRequest, RealtimeStatus,
        VoiceActionVerb, VoiceProviderKey, VoiceProviderKeyWrite,
    };

    #[test]
    fn freeflow_defaults_to_disabled() {
        let config = FreeflowConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.model, DEFAULT_FREEFLOW_MODEL);
    }

    #[test]
    fn realtime_status_default_is_off() {
        assert_eq!(RealtimeStatus::default(), RealtimeStatus::Off);
    }

    #[test]
    fn destructive_verb_is_typed() {
        assert_eq!(VoiceActionVerb::Kill, VoiceActionVerb::Kill);
    }

    #[test]
    fn gate_action_accepts_typed_fields() -> Result<(), serde_json::Error> {
        let request = serde_json::from_value::<RealtimeActionRequest>(serde_json::json!({
            "verb": "gate_tool",
            "agentId": "worker",
            "toolName": "Bash",
            "enabled": true
        }))?;

        assert_eq!(request.tool_name.as_deref(), Some("Bash"));
        assert_eq!(request.enabled, Some(true));
        Ok(())
    }

    #[test]
    fn provider_key_debug_is_redacted() -> Result<(), Box<dyn std::error::Error>> {
        let request = VoiceProviderKeyWrite {
            provider: VoiceProviderKey::Groq,
            secret: WriteOnlySecret::new(String::from("top-secret"))?,
        };

        assert!(!format!("{request:?}").contains("top-secret"));
        Ok(())
    }
}
