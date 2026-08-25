use serde::{Deserialize, Serialize};

/// Condition that is re-checked immediately before a queued terminal delivery.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryPrecondition {
    InboxNonempty,
}

/// Durable user or router message waiting for an agent terminal to become available.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct QueuedTerminalMessage {
    pub id: String,
    pub agent_id: String,
    pub text: String,
    pub instruction: Option<String>,
    pub queued_at_ms: i64,
    pub manual: bool,
    pub precondition: Option<DeliveryPrecondition>,
    pub failed_attempts: u8,
}

#[cfg(test)]
mod tests {
    use super::{DeliveryPrecondition, QueuedTerminalMessage};

    #[test]
    fn queued_message_accepts_epoch_boundary() {
        let message = QueuedTerminalMessage {
            id: String::from("q-1"),
            agent_id: String::from("dev-1"),
            text: String::new(),
            instruction: None,
            queued_at_ms: i64::MIN,
            manual: false,
            precondition: Some(DeliveryPrecondition::InboxNonempty),
            failed_attempts: 0,
        };

        assert_eq!(message.queued_at_ms, i64::MIN);
    }
}
