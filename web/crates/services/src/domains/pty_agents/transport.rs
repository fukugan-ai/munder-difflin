use md_web_contracts::domains::pty_agents::{TerminalClientFrame, TerminalServerFrame};

use super::{PtyRegistry, PtyServiceError};

/// Side-effect boundary used by the WebSocket adapter to route typed client frames.
pub struct TerminalFrameRouter<'registry> {
    registry: &'registry PtyRegistry,
}

impl<'registry> TerminalFrameRouter<'registry> {
    pub fn new(registry: &'registry PtyRegistry) -> Self {
        Self { registry }
    }

    /// Applies one client frame. Domain errors become typed socket frames and never panic.
    pub fn route(&self, frame: TerminalClientFrame) -> Vec<TerminalServerFrame> {
        let pty_id = client_pty_id(&frame).map(String::from);
        let result = match frame {
            TerminalClientFrame::Attach { pty_id, after_seq } => {
                self.registry.attach_frames(&pty_id, after_seq)
            }
            TerminalClientFrame::Input { pty_id, data } => {
                self.registry.write(&pty_id, &data).map(|()| Vec::new())
            }
            TerminalClientFrame::Resize { pty_id, dimensions } => self
                .registry
                .resize(&pty_id, dimensions)
                .map(|()| Vec::new()),
            TerminalClientFrame::Redraw { pty_id } => {
                self.registry.redraw(&pty_id).map(|_| Vec::new())
            }
            TerminalClientFrame::Presence { pty_id, presence } => self
                .registry
                .update_presence(&pty_id, presence)
                .map(|()| Vec::new()),
            TerminalClientFrame::Detach { .. } => Ok(Vec::new()),
        };
        result.unwrap_or_else(|error| vec![error_frame(pty_id, &error)])
    }
}

fn client_pty_id(frame: &TerminalClientFrame) -> Option<&str> {
    match frame {
        TerminalClientFrame::Attach { pty_id, .. }
        | TerminalClientFrame::Input { pty_id, .. }
        | TerminalClientFrame::Resize { pty_id, .. }
        | TerminalClientFrame::Presence { pty_id, .. }
        | TerminalClientFrame::Redraw { pty_id }
        | TerminalClientFrame::Detach { pty_id } => Some(pty_id),
    }
}

fn error_frame(pty_id: Option<String>, error: &PtyServiceError) -> TerminalServerFrame {
    TerminalServerFrame::Error {
        pty_id,
        code: error.code(),
        message_ja: String::from(error.message_ja()),
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{
        TerminalClientFrame, TerminalErrorCode, TerminalServerFrame,
    };

    use super::TerminalFrameRouter;
    use crate::domains::pty_agents::PtyRegistry;

    #[test]
    fn unknown_attach_is_a_typed_error_frame() {
        let registry = PtyRegistry::new();
        let frames = TerminalFrameRouter::new(&registry).route(TerminalClientFrame::Attach {
            pty_id: String::from("pty-missing"),
            after_seq: 0,
        });

        assert!(matches!(
            frames.as_slice(),
            [TerminalServerFrame::Error {
                code: TerminalErrorCode::NotFound,
                pty_id: Some(pty_id),
                ..
            }] if pty_id == "pty-missing"
        ));
    }

    #[test]
    fn detach_is_idempotent_and_silent() {
        let registry = PtyRegistry::new();
        let frames = TerminalFrameRouter::new(&registry).route(TerminalClientFrame::Detach {
            pty_id: String::from("pty-missing"),
        });
        assert!(frames.is_empty());
    }
}
