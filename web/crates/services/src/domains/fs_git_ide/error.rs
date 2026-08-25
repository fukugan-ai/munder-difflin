use std::fmt::{Display, Formatter};

/// Narrow failures for the local workspace/Git bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainError {
    InvalidWorkspace,
    InvalidPath,
    NotFound,
    NotRegularFile,
    FileTooLarge,
    BinaryFile,
    Io,
    NotGitRepository,
    InvalidRevision,
    InvalidWorktreeName,
    UnknownWorktree,
    ArchivedWorktree,
    DirtyWorktree,
    BusyWorktree,
    ConfirmationRequired,
    CommandUnavailable,
    CommandFailed,
    CommandTimedOut,
    OutputTooLarge,
    InvalidResponse,
    RepositoryNotAllowed,
}

impl Display for DomainError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidWorkspace => "管理対象のワークスペースではありません",
            Self::InvalidPath => "ワークスペース外のパスは利用できません",
            Self::NotFound => "ファイルが見つかりません",
            Self::NotRegularFile => "通常ファイルではありません",
            Self::FileTooLarge => "ファイルが大きすぎます",
            Self::BinaryFile => "バイナリファイルはテキストとして開けません",
            Self::Io => "ファイル操作に失敗しました",
            Self::NotGitRepository => "Gitリポジトリではありません",
            Self::InvalidRevision => "Gitのrevisionが不正です",
            Self::InvalidWorktreeName => "worktree名が不正です",
            Self::UnknownWorktree => "管理対象のworktreeではありません",
            Self::ArchivedWorktree => "archive済みworktreeは削除できません",
            Self::DirtyWorktree => "未コミットの変更があるため切り替えできません",
            Self::BusyWorktree => "エージェントが作業中のため切り替えできません",
            Self::ConfirmationRequired => "ローカルcheckoutには確認が必要です",
            Self::CommandUnavailable => "必要なローカルcommandを起動できません",
            Self::CommandFailed => "ローカルcommandの実行に失敗しました",
            Self::CommandTimedOut => "ローカルcommandが時間切れになりました",
            Self::OutputTooLarge => "command出力が上限を超えました",
            Self::InvalidResponse => "commandから不正な応答が返りました",
            Self::RepositoryNotAllowed => "許可されたforkではありません",
        })
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests {
    use super::DomainError;

    #[test]
    fn repository_error_does_not_disclose_a_path() {
        assert_eq!(
            DomainError::RepositoryNotAllowed.to_string(),
            "許可されたforkではありません"
        );
    }
}
