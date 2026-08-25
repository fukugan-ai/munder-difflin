//! Browser-safe contracts for the office floor and roster.

mod model;

pub use model::{
    Accent, AgentStatus, CompletionNotice, HiveHandoff, MessageAct, OfficeAgent,
    OfficeAgentLiveState, OfficeAgentSpawnRequest, OfficeAgentTelemetry, OfficeCharacter,
    OfficeHireManifest, OfficeLiveUpdate, OfficeSnapshot, OfficeTask, OfficeTheme, OfficeUiAction,
    OfficeUiState, RestorableAgent, TaskStatus, ThemePreference,
};
