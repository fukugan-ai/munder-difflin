use md_web_contracts::domains::pty_agents::{
    AgentRecord, PtyDimensions, RestartAgentRequest, RestoreAgentRequest, SpawnAgentRequest,
    TerminalPresence, TerminalServerFrame,
};

/// Complete renderer input for the PTY/agent domain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PtyAgentsViewModel {
    pub agents: Vec<AgentRecord>,
    pub restorable_agents: Vec<AgentRecord>,
    pub selected_agent_id: Option<String>,
    pub frames: Vec<TerminalServerFrame>,
    pub loading: bool,
    pub error_ja: Option<String>,
}

/// User intent emitted to the route adapter. Process authority stays server-side.
#[derive(Clone, Debug, PartialEq)]
pub enum PtyAgentsAction {
    Select(String),
    Spawn(SpawnAgentRequest),
    // Raw xterm keystrokes are emitted by the narrow JS/WebSocket bridge.
    #[allow(dead_code)]
    Input {
        pty_id: String,
        data: String,
    },
    QueueMessage {
        agent_id: String,
        text: String,
    },
    Presence {
        pty_id: String,
        presence: TerminalPresence,
    },
    // ResizeObserver emits this through the narrow JS/WebSocket bridge.
    #[allow(dead_code)]
    Resize {
        pty_id: String,
        dimensions: PtyDimensions,
    },
    Redraw(String),
    Kill(String),
    Restart(RestartAgentRequest),
    Restore(RestoreAgentRequest),
    RestoreAll,
    Refresh,
}

impl PtyAgentsViewModel {
    pub(crate) fn selected_agent(&self) -> Option<&AgentRecord> {
        let selected_id = self.selected_agent_id.as_deref()?;
        self.agents.iter().find(|agent| agent.id == selected_id)
    }

    pub(crate) fn terminal_text(&self, pty_id: &str) -> String {
        let Some(current_generation) = self
            .frames
            .iter()
            .filter_map(|frame| terminal_frame_generation(frame, pty_id))
            .max()
        else {
            return String::new();
        };
        let mut text = String::new();
        for frame in &self.frames {
            match frame {
                TerminalServerFrame::Output {
                    pty_id: frame_pty,
                    generation,
                    data,
                    ..
                } if frame_pty == pty_id && *generation == current_generation => {
                    text.push_str(data);
                }
                TerminalServerFrame::Exited {
                    pty_id: frame_pty,
                    generation,
                    exit,
                    ..
                } if frame_pty == pty_id && *generation == current_generation => {
                    text.push_str("\n─ プロセス終了");
                    if let Some(code) = exit.exit_code {
                        text.push_str(" (code ");
                        text.push_str(&code.to_string());
                        text.push(')');
                    }
                    text.push_str(" ─\n");
                }
                _ => {}
            }
        }
        text
    }

    pub(crate) fn terminal_is_busy(&self, pty_id: &str) -> bool {
        self.frames.iter().rev().find_map(|frame| match frame {
            TerminalServerFrame::Readiness {
                pty_id: frame_pty,
                readiness,
                ..
            } if frame_pty == pty_id => Some(readiness.status),
            _ => None,
        }) == Some(md_web_contracts::domains::pty_agents::TerminalActivityStatus::Busy)
    }
}

fn terminal_frame_generation(frame: &TerminalServerFrame, pty_id: &str) -> Option<u64> {
    match frame {
        TerminalServerFrame::Attached {
            pty, generation, ..
        } if pty.id == pty_id => Some(*generation),
        TerminalServerFrame::Output {
            pty_id: frame_pty,
            generation,
            ..
        }
        | TerminalServerFrame::Exited {
            pty_id: frame_pty,
            generation,
            ..
        }
        | TerminalServerFrame::Relaunching {
            pty_id: frame_pty,
            generation,
            ..
        } if frame_pty == pty_id => Some(*generation),
        TerminalServerFrame::Readiness {
            pty_id: frame_pty,
            generation,
            ..
        } if frame_pty == pty_id => Some(*generation),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{
        AgentProvider, AgentRecord, AgentRole, AgentStatus, TerminalActivityStatus,
        TerminalPresence, TerminalReadiness, TerminalServerFrame,
    };

    use super::PtyAgentsViewModel;

    fn agent() -> AgentRecord {
        AgentRecord {
            id: String::from("dev-1"),
            name: String::from("Dev 1"),
            provider: AgentProvider::Codex,
            role: AgentRole::default(),
            description: String::new(),
            cwd: String::from("/repo"),
            command: String::from("codex"),
            args: Vec::new(),
            model: None,
            status: AgentStatus::Idle,
            action_ja: String::from("待機中"),
            pty_id: Some(String::from("pty-dev-1")),
            worktree_path: None,
            session_id: None,
            archived: false,
        }
    }

    #[test]
    fn selected_agent_matches_stable_id() {
        let view = PtyAgentsViewModel {
            agents: vec![agent()],
            selected_agent_id: Some(String::from("dev-1")),
            ..PtyAgentsViewModel::default()
        };
        assert!(matches!(view.selected_agent(), Some(agent) if agent.name == "Dev 1"));
    }

    #[test]
    fn terminal_text_ignores_other_processes() {
        let view = PtyAgentsViewModel {
            frames: vec![TerminalServerFrame::Output {
                pty_id: String::from("pty-other"),
                generation: 1,
                seq: 1,
                data: String::from("secret-other-output"),
            }],
            ..PtyAgentsViewModel::default()
        };
        assert!(view.terminal_text("pty-dev-1").is_empty());
    }

    #[test]
    fn terminal_text_uses_only_latest_generation_for_stable_pty_id() {
        let view = PtyAgentsViewModel {
            frames: vec![
                TerminalServerFrame::Output {
                    pty_id: String::from("pty-dev-1"),
                    generation: 1,
                    seq: 4,
                    data: String::from("旧世代の日本語"),
                },
                TerminalServerFrame::Output {
                    pty_id: String::from("pty-dev-1"),
                    generation: 2,
                    seq: 1,
                    data: String::from("日本語準備完了"),
                },
            ],
            ..PtyAgentsViewModel::default()
        };

        assert_eq!(view.terminal_text("pty-dev-1"), "日本語準備完了");
    }

    #[test]
    fn latest_readiness_projects_busy_status_per_agent() {
        let readiness = |status| TerminalServerFrame::Readiness {
            pty_id: String::from("pty-dev-1"),
            generation: 1,
            readiness: TerminalReadiness {
                has_initial_output: true,
                boot_grace_remaining_ms: 0,
                quiet_remaining_ms: 0,
                cooldown_remaining_ms: 0,
                presence: TerminalPresence::default(),
                status,
            },
        };
        let view = PtyAgentsViewModel {
            frames: vec![
                readiness(TerminalActivityStatus::Ready),
                readiness(TerminalActivityStatus::Busy),
            ],
            ..PtyAgentsViewModel::default()
        };
        assert!(view.terminal_is_busy("pty-dev-1"));
        assert!(!view.terminal_is_busy("pty-other"));
    }
}
