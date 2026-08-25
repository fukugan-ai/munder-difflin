#![forbid(unsafe_code)]

pub mod domains;

mod error;
mod event;
mod health;

pub use domains::hive_tasks::{
    AgentControlSnapshot, HiveAgent, HiveDomainEvent, HiveEventEnvelope, HiveMessage, HiveRegistry,
    HiveSnapshot, HiveTask, HumanQa, MessageAct, PreservedWorktreeSnapshot, TaskLedger, TaskStatus,
    WorkerSnapshot, WorkerStatus,
};
pub use domains::{DomainId, DomainInvalidated};
pub use error::{ApiError, ErrorCode};
pub use event::{AppEvent, EventEnvelope};
pub use health::{HealthSnapshot, PersistenceCode, PersistenceStatus};
