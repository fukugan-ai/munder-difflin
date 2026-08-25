use serde::{Deserialize, Serialize};

use crate::domains::pty_agents::AgentHookEvent;

use super::{
    AgentControlSnapshot, BreakerState, HiveMessage, HiveTask, WorkerSnapshot,
    WorkerTeardownReceipt,
};

/// Coordination events delivered to the browser in strict sequence order.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum HiveDomainEvent {
    MessageRouted {
        message: HiveMessage,
        targets: Vec<String>,
    },
    TaskAdded(HiveTask),
    TaskPatched(HiveTask),
    TaskDeleted {
        task_id: String,
    },
    AgentSpawned {
        agent_id: String,
    },
    AgentArchived {
        agent_id: String,
    },
    BreakerChanged(BreakerState),
    ControlChanged {
        agent_id: String,
        snapshot: AgentControlSnapshot,
    },
    WorkerChanged(WorkerSnapshot),
    WorkerTeardown(WorkerTeardownReceipt),
    AgentHookObserved {
        agent_id: String,
        event_id: String,
        event: AgentHookEvent,
        tool_name: Option<String>,
    },
}

/// Replayable event stream item for SSE/WebSocket consumers.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HiveEventEnvelope {
    pub seq: u64,
    pub ts_ms: i64,
    pub event: HiveDomainEvent,
}

#[cfg(test)]
mod tests {
    use super::{HiveDomainEvent, HiveEventEnvelope};

    #[test]
    fn envelope_accepts_maximum_sequence() {
        let event = HiveEventEnvelope {
            seq: u64::MAX,
            ts_ms: 0,
            event: HiveDomainEvent::TaskDeleted {
                task_id: String::from("t-1"),
            },
        };

        assert_eq!(event.seq, u64::MAX);
    }
}
