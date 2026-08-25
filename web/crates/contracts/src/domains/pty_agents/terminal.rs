use serde::{Deserialize, Serialize};

/// Terminal grid dimensions sent to the server-side process owner.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PtyDimensions {
    pub cols: u16,
    pub rows: u16,
}

/// Browser-safe summary of one live child process.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PtySummary {
    pub id: String,
    pub cwd: String,
    pub command: String,
    pub pid: Option<u32>,
    pub process_group_id: Option<i64>,
    pub dimensions: PtyDimensions,
    pub last_output_at_ms: i64,
    pub has_output: bool,
}

/// Exit information emitted once for the process generation that owned the PTY id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProcessExit {
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
}

/// Process-owned natural-exit event delivered independently of browser WebSocket clients.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PtyExitEvent {
    pub pty_id: String,
    pub generation: u64,
    pub exit: ProcessExit,
}

/// Browser-owned interaction facts that block automated prompt delivery.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalPresence {
    pub draft_nonempty: bool,
    pub picker_open: bool,
    pub composing: bool,
    pub last_activity_at_ms: i64,
}

impl TerminalPresence {
    pub const fn blocks_automation(self) -> bool {
        self.draft_nonempty || self.picker_open || self.composing
    }
}

/// Readiness status projected per process generation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalActivityStatus {
    Booting,
    Busy,
    Ready,
    UserOwned,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalReadiness {
    pub has_initial_output: bool,
    pub boot_grace_remaining_ms: u64,
    pub quiet_remaining_ms: u64,
    pub cooldown_remaining_ms: u64,
    pub presence: TerminalPresence,
    pub status: TerminalActivityStatus,
}

/// Stable terminal-domain error category.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalErrorCode {
    InvalidRequest,
    NotFound,
    Conflict,
    ResumeUnavailable,
    SpawnFailed,
    IoFailed,
    ProcessExited,
    Unsupported,
}

/// Typed command accepted by the terminal WebSocket.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum TerminalClientFrame {
    Attach {
        pty_id: String,
        after_seq: u64,
    },
    Input {
        pty_id: String,
        data: String,
    },
    Resize {
        pty_id: String,
        dimensions: PtyDimensions,
    },
    Redraw {
        pty_id: String,
    },
    Presence {
        pty_id: String,
        presence: TerminalPresence,
    },
    Detach {
        pty_id: String,
    },
}

/// Ordered terminal event emitted by the server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum TerminalServerFrame {
    Attached {
        pty: PtySummary,
        generation: u64,
        oldest_seq: u64,
        next_seq: u64,
        truncated: bool,
    },
    Output {
        pty_id: String,
        generation: u64,
        seq: u64,
        data: String,
    },
    Exited {
        pty_id: String,
        generation: u64,
        seq: u64,
        exit: ProcessExit,
    },
    Relaunching {
        pty_id: String,
        generation: u64,
        seq: u64,
    },
    Readiness {
        pty_id: String,
        generation: u64,
        readiness: TerminalReadiness,
    },
    Error {
        pty_id: Option<String>,
        code: TerminalErrorCode,
        message_ja: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{PtyDimensions, TerminalClientFrame, TerminalPresence, TerminalServerFrame};

    #[test]
    fn attach_frame_accepts_zero_sequence() {
        let frame = TerminalClientFrame::Attach {
            pty_id: String::from("pty-dev-1"),
            after_seq: 0,
        };

        assert!(matches!(
            frame,
            TerminalClientFrame::Attach { after_seq: 0, .. }
        ));
    }

    #[test]
    fn resize_frame_preserves_minimum_dimensions() {
        let frame = TerminalClientFrame::Resize {
            pty_id: String::from("pty-dev-1"),
            dimensions: PtyDimensions { cols: 1, rows: 1 },
        };

        assert!(matches!(
            frame,
            TerminalClientFrame::Resize {
                dimensions: PtyDimensions { cols: 1, rows: 1 },
                ..
            }
        ));
    }

    #[test]
    fn error_frame_allows_connection_level_failure() {
        let frame = TerminalServerFrame::Error {
            pty_id: None,
            code: super::TerminalErrorCode::InvalidRequest,
            message_ja: String::new(),
        };

        assert!(matches!(
            frame,
            TerminalServerFrame::Error { pty_id: None, .. }
        ));
    }

    #[test]
    fn composition_presence_blocks_automation() {
        let presence = TerminalPresence {
            composing: true,
            ..TerminalPresence::default()
        };
        assert!(presence.blocks_automation());
        assert!(matches!(
            TerminalClientFrame::Presence {
                pty_id: String::new(),
                presence,
            },
            TerminalClientFrame::Presence { presence, .. } if presence.composing
        ));
    }
}
