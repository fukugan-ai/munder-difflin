use std::fmt::{Display, Formatter};
use std::io;

use md_web_contracts::domains::pty_agents::TerminalErrorCode;

/// Concrete process-domain failure kept independent from operator-facing formatting.
#[derive(Debug)]
pub enum PtyServiceError {
    InvalidRequest(&'static str),
    NotFound,
    Conflict,
    ResumeUnavailable,
    Spawn(io::Error),
    Io(io::Error),
    ProcessExited,
    StatePoisoned,
}

impl PtyServiceError {
    /// Stable contract code returned at the client boundary.
    pub const fn code(&self) -> TerminalErrorCode {
        match self {
            Self::InvalidRequest(_) => TerminalErrorCode::InvalidRequest,
            Self::NotFound => TerminalErrorCode::NotFound,
            Self::Conflict => TerminalErrorCode::Conflict,
            Self::ResumeUnavailable => TerminalErrorCode::ResumeUnavailable,
            Self::Spawn(_) => TerminalErrorCode::SpawnFailed,
            Self::Io(_) | Self::StatePoisoned => TerminalErrorCode::IoFailed,
            Self::ProcessExited => TerminalErrorCode::ProcessExited,
        }
    }

    /// Concise Japanese message for the browser error surface.
    pub const fn message_ja(&self) -> &'static str {
        match self {
            Self::InvalidRequest(message) => message,
            Self::NotFound => "指定されたターミナルはありません。",
            Self::Conflict => "同じIDのターミナルがすでに動作しています。",
            Self::ResumeUnavailable => {
                "既存セッションを再開できないため、現在のプロセスは変更していません。"
            }
            Self::Spawn(_) => "エージェントのプロセスを起動できませんでした。",
            Self::Io(_) => "ターミナルとの入出力に失敗しました。",
            Self::ProcessExited => "ターミナルのプロセスは終了しています。",
            Self::StatePoisoned => "ターミナルの内部状態を読み取れませんでした。",
        }
    }
}

impl Display for PtyServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message_ja())
    }
}

impl std::error::Error for PtyServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spawn(error) | Self::Io(error) => Some(error),
            Self::InvalidRequest(_)
            | Self::NotFound
            | Self::Conflict
            | Self::ResumeUnavailable
            | Self::ProcessExited
            | Self::StatePoisoned => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::TerminalErrorCode;

    use super::PtyServiceError;

    #[test]
    fn not_found_maps_to_stable_contract_code() {
        assert_eq!(
            PtyServiceError::NotFound.code(),
            TerminalErrorCode::NotFound
        );
    }

    #[test]
    fn resume_error_explains_non_destructive_outcome() {
        assert!(
            PtyServiceError::ResumeUnavailable
                .message_ja()
                .contains("変更していません")
        );
    }
}
