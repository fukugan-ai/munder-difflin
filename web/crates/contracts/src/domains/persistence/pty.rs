use serde::{Deserialize, Serialize};

use crate::domains::pty_agents::{AgentRecord, QueuedTerminalMessage};

/// Stable record kinds stored in `web_durable_records` under the `floors` domain.
pub const FLOOR_AGENT_KIND: &str = "agent";
pub const TERMINAL_QUEUE_KIND: &str = "terminal_queue";

/// Compare-and-swap write for one durable floor agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FloorAgentWrite {
    pub floor_id: String,
    pub expected_revision: i64,
    pub agent: AgentRecord,
}

/// Versioned durable agent returned by the PostgreSQL facade.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersistedFloorAgent {
    pub floor_id: String,
    pub revision: i64,
    pub agent: AgentRecord,
    pub updated_at_ms: i64,
}

/// Revision identity used for archive/restorable transitions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FloorAgentRevision {
    pub floor_id: String,
    pub agent_id: String,
    pub expected_revision: i64,
}

/// Durable FIFO snapshot for one floor agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersistedTerminalQueue {
    pub floor_id: String,
    pub agent_id: String,
    pub revision: i64,
    pub messages: Vec<QueuedTerminalMessage>,
    pub updated_at_ms: i64,
}

/// Appends one stable message id to a queue at an expected revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalQueueEnqueue {
    pub floor_id: String,
    pub expected_revision: i64,
    pub message: QueuedTerminalMessage,
}

/// Compare-and-swap mutation of the current queue head.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalQueueHeadMutation {
    pub floor_id: String,
    pub agent_id: String,
    pub message_id: String,
    pub expected_revision: i64,
}

/// Failure update receipt. `dropped` is populated when the third failure removes
/// the message from the durable queue.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalQueueFailureReceipt {
    pub queue: PersistedTerminalQueue,
    pub dropped: Option<QueuedTerminalMessage>,
}

/// Durable state selected after a process exit policy decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalExitDisposition {
    Exited,
    Archived,
    Restorable,
}

/// One idempotent explicit-kill or natural-exit persistence operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NaturalExitWrite {
    pub floor_id: String,
    pub agent_id: String,
    pub expected_agent_revision: i64,
    pub event_id: String,
    pub occurred_at_ms: i64,
    pub exit_code: Option<i32>,
    pub disposition: NaturalExitDisposition,
}

/// Transaction receipt safe to return after an ambiguous-commit retry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NaturalExitReceipt {
    pub event_id: String,
    pub event_sequence: u64,
    pub agent_revision: i64,
    pub queue_revision: Option<i64>,
    pub cleared_messages: u64,
    pub disposition: NaturalExitDisposition,
}

#[cfg(test)]
mod tests {
    use super::NaturalExitDisposition;

    #[test]
    fn natural_exit_dispositions_are_distinct() {
        assert_ne!(
            NaturalExitDisposition::Archived,
            NaturalExitDisposition::Restorable
        );
    }
}
