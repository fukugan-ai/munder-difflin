//! Typed contracts for browser-managed local agent processes and terminal streams.

mod agent;
mod hook;
mod queue;
mod terminal;

pub use agent::{
    AgentProvider, AgentRecord, AgentRole, AgentStatus, RestartAgentRequest, RestoreAgentRequest,
    SpawnAgentRequest, SpawnAgentResult,
};
pub use hook::{AgentHookDecision, AgentHookEvent, AgentHookRequest};
pub use queue::{DeliveryPrecondition, QueuedTerminalMessage};
pub use terminal::{
    ProcessExit, PtyDimensions, PtyExitEvent, PtySummary, TerminalActivityStatus,
    TerminalClientFrame, TerminalErrorCode, TerminalPresence, TerminalReadiness,
    TerminalServerFrame,
};
