use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use md_web_contracts::domains::config_onboarding::{HostPlatform, ToolKind, ToolStatus};

struct ToolSpec {
    id: &'static str,
    binary: Option<&'static str>,
    label: &'static str,
    kind: ToolKind,
    why_ja: &'static str,
    essential: bool,
    install_posix: Option<&'static str>,
    install_windows: Option<&'static str>,
    docs_url: Option<&'static str>,
}

const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        id: "uv",
        binary: Some("uv"),
        label: "uv",
        kind: ToolKind::Prerequisite,
        why_ja: "MemPalaceを既存Python環境から分離して導入します。",
        essential: true,
        install_posix: Some("curl -LsSf https://astral.sh/uv/install.sh | sh"),
        install_windows: Some(
            "powershell -ExecutionPolicy ByPass -c \"irm https://astral.sh/uv/install.ps1 | iex\"",
        ),
        docs_url: Some("https://docs.astral.sh/uv/"),
    },
    ToolSpec {
        id: "mempalace",
        binary: None,
        label: "MemPalace",
        kind: ToolKind::Memory,
        why_ja: "エージェントの記憶を意味で検索できます。",
        essential: true,
        install_posix: Some("uv tool install mempalace"),
        install_windows: Some("uv tool install mempalace"),
        docs_url: None,
    },
    ToolSpec {
        id: "git",
        binary: Some("git"),
        label: "git",
        kind: ToolKind::Prerequisite,
        why_ja: "worktreeと履歴を使って複数エージェントの作業を分離します。",
        essential: true,
        install_posix: Some("sudo apt install git"),
        install_windows: Some("winget install --id Git.Git -e"),
        docs_url: Some("https://git-scm.com/downloads"),
    },
    ToolSpec {
        id: "node",
        binary: Some("node"),
        label: "Node.js",
        kind: ToolKind::Prerequisite,
        why_ja: "npm配布のエージェントCLIを実行します。",
        essential: false,
        install_posix: None,
        install_windows: None,
        docs_url: Some("https://nodejs.org"),
    },
    ToolSpec {
        id: "engine:claude",
        binary: Some("claude"),
        label: "Claude Code",
        kind: ToolKind::Engine,
        why_ja: "既定のエージェントエンジンです。",
        essential: true,
        install_posix: Some("npm install -g @anthropic-ai/claude-code"),
        install_windows: Some("npm install -g @anthropic-ai/claude-code"),
        docs_url: Some("https://docs.anthropic.com/en/docs/claude-code"),
    },
    ToolSpec {
        id: "engine:codex",
        binary: Some("codex"),
        label: "Codex",
        kind: ToolKind::Engine,
        why_ja: "Codex CLIエージェントを利用できます。",
        essential: false,
        install_posix: Some("npm install -g @openai/codex"),
        install_windows: Some("npm install -g @openai/codex"),
        docs_url: Some("https://developers.openai.com/codex/cli"),
    },
];

/// Returns the server host's platform family.
pub const fn host_platform() -> HostPlatform {
    if cfg!(target_os = "windows") {
        HostPlatform::Windows
    } else if cfg!(target_os = "macos") {
        HostPlatform::MacOs
    } else if cfg!(target_os = "linux") {
        HostPlatform::Linux
    } else {
        HostPlatform::Other
    }
}

/// Resolves a trusted executable name without invoking a shell.
pub fn resolve_on_path(binary: &str, path: &OsStr, platform: HostPlatform) -> Option<PathBuf> {
    if binary.is_empty()
        || binary
            .bytes()
            .any(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    {
        return None;
    }
    for directory in env::split_paths(path) {
        let candidate = directory.join(binary);
        if executable_candidate(&candidate) {
            return Some(candidate);
        }
        if platform == HostPlatform::Windows {
            for extension in ["exe", "cmd", "bat"] {
                let candidate = directory.join(format!("{binary}.{extension}"));
                if executable_candidate(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// Probes the server host. `mempalace` availability comes from its owning service.
pub fn probe_host_tools(mempalace_path: Option<&Path>) -> Vec<ToolStatus> {
    let platform = host_platform();
    let path = env::var_os("PATH").unwrap_or_default();
    let mut statuses = Vec::with_capacity(TOOLS.len());
    for spec in TOOLS {
        let resolved = if spec.id == "mempalace" {
            mempalace_path
                .filter(|candidate| executable_candidate(candidate))
                .map(Path::to_path_buf)
        } else {
            spec.binary
                .and_then(|binary| resolve_on_path(binary, &path, platform))
        };
        let install_command = match platform {
            HostPlatform::Windows => spec.install_windows,
            HostPlatform::Linux | HostPlatform::MacOs | HostPlatform::Other => spec.install_posix,
        };
        statuses.push(ToolStatus {
            id: String::from(spec.id),
            label: String::from(spec.label),
            kind: spec.kind,
            why_ja: String::from(spec.why_ja),
            essential: spec.essential,
            found: resolved.is_some(),
            path: resolved.map(|value| value.to_string_lossy().into_owned()),
            detail_ja: None,
            install_command: install_command.map(String::from),
            docs_url: spec.docs_url.map(String::from),
            observed_on: String::from("server_host"),
        });
    }
    statuses
}

fn executable_candidate(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use md_web_contracts::domains::config_onboarding::HostPlatform;

    use super::{host_platform, probe_host_tools, resolve_on_path};

    #[test]
    fn invalid_binary_name_is_not_resolved() {
        let result = resolve_on_path("git;echo", &OsString::from("/usr/bin"), HostPlatform::Linux);

        assert_eq!(result, None);
    }

    #[test]
    fn host_platform_is_known_variant() {
        assert!(matches!(
            host_platform(),
            HostPlatform::Linux | HostPlatform::MacOs | HostPlatform::Windows | HostPlatform::Other
        ));
    }

    #[test]
    fn probe_marks_results_as_server_host() {
        let statuses = probe_host_tools(None);

        assert!(
            statuses
                .iter()
                .all(|status| status.observed_on == "server_host")
        );
    }
}
