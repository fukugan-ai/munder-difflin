#![forbid(unsafe_code)]

mod events;
mod model;

pub use events::{HiveDomainEvent, HiveEventEnvelope};
pub use model::{
    AgentControlSnapshot, BreakerLevel, BreakerState, HiveAgent, HiveHookDecision, HiveMessage,
    HiveRegistry, HiveSnapshot, HiveTask, HumanQa, MessageAct, PreservedWorktreeSnapshot,
    TaskLedger, TaskStatus, WorkerSnapshot, WorkerStatus, WorkerTeardownReceipt,
};
