use std::fmt::{Debug, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Normalized lifecycle event emitted by an agent CLI hook.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    UserPromptSubmit,
    Notification,
    Stop,
    SubagentStop,
    SessionStart,
    SessionEnd,
}

/// Authenticated hook request accepted by the server-owned internal endpoint.
///
/// `capability` is a bearer secret. Endpoint adapters must never log, persist, or return it.
#[derive(Clone, Deserialize, PartialEq, Serialize)]
pub struct AgentHookRequest {
    pub agent_id: String,
    pub capability: String,
    pub event_id: String,
    pub event: AgentHookEvent,
    pub tool_name: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

/// Hive decision returned to a CLI hook. A missing reason is intentionally allowed on permit.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookDecision {
    pub allow: bool,
    pub reason_ja: Option<String>,
    /// One-shot steering body consumed by the CLI hook after the Hive decision.
    #[serde(default)]
    pub steer: Option<String>,
}

impl Debug for AgentHookDecision {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentHookDecision")
            .field("allow", &self.allow)
            .field("reason_ja", &self.reason_ja)
            .field("steer", &self.steer.as_ref().map(|_| "[REDACTED]"))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{AgentHookDecision, AgentHookEvent, AgentHookRequest};

    #[test]
    fn hook_request_carries_identity_tool_and_typed_event() {
        let request = AgentHookRequest {
            agent_id: String::from("dev-1"),
            capability: String::from("0123456789abcdef0123456789abcdef"),
            event_id: String::from("evt-1"),
            event: AgentHookEvent::PreToolUse,
            tool_name: Some(String::from("Bash")),
            payload: json!({"command": "cargo test"}),
        };

        let encoded = serde_json::to_value(request);
        assert!(matches!(
            encoded,
            Ok(value)
                if value["agent_id"] == "dev-1"
                    && value["tool_name"] == "Bash"
                    && value["event"] == "pre_tool_use"
        ));
    }

    #[test]
    fn decision_defaults_missing_steer_for_older_producers() {
        let decision = serde_json::from_value::<AgentHookDecision>(json!({
            "allow": true,
            "reason_ja": null
        }));
        assert!(matches!(decision, Ok(decision) if decision.steer.is_none()));
    }

    #[test]
    fn decision_serializes_steer_but_redacts_it_from_debug_output() {
        let decision = AgentHookDecision {
            allow: true,
            reason_ja: None,
            steer: Some(String::from("非公開の追加指示")),
        };
        let encoded = serde_json::to_value(&decision);
        assert!(matches!(encoded, Ok(value) if value["steer"] == "非公開の追加指示"));
        let debug = format!("{decision:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("非公開の追加指示"));
    }
}
