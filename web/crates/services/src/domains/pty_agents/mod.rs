//! Local process, agent-lifecycle, queue, and terminal-stream services.

mod error;
mod hook;
mod lifecycle;
mod process;
mod provision;
mod queue;
mod registry;
mod transport;
mod wake;

pub use error::PtyServiceError;
pub use hook::{AgentHookCapabilities, AgentHookLaunch};
pub use lifecycle::{AgentLifecycle, restart_spawn_request, restore_spawn_request};
pub use process::NativePtyBackend;
pub use provision::{
    parse_agent_hook_decision, render_claude_hook_response, render_gemini_hook_response,
};
pub use queue::{
    DELIVERY_BOOT_GRACE_MS, DELIVERY_COOLDOWN_MS, DELIVERY_QUIET_MS, DeliveryDecision,
    DeliveryGate, TerminalQueue, evaluate_terminal_readiness,
};
pub use registry::{PtyRegistry, QUEUE_ENTER_DELAY};
pub use transport::TerminalFrameRouter;
pub use wake::{
    WORKER_WAKE_BOOT_GRACE_MS, WORKER_WAKE_COOLDOWN_MS, WORKER_WAKE_HITL_REARM_MS,
    WORKER_WAKE_IDLE_MS, WORKER_WAKE_NUDGE, WorkerWakeFacts, WorkerWakeWatchdog,
};
