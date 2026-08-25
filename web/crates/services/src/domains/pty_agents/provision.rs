use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use md_web_contracts::domains::pty_agents::{AgentHookDecision, AgentProvider};
use serde_json::json;

use super::error::PtyServiceError;
use super::hook::AgentHookLaunch;

const HELPER_NAME: &str = "md-hive-hook-relay.sh";

/// Per-generation provider hook files and command-line/environment projection.
pub(crate) struct AgentHookRuntime {
    root: PathBuf,
    prepend_args: Vec<String>,
    append_args: Vec<String>,
    environment: BTreeMap<String, String>,
}

impl AgentHookRuntime {
    pub(crate) const fn supports(provider: AgentProvider) -> bool {
        matches!(
            provider,
            AgentProvider::Claude | AgentProvider::Codex | AgentProvider::Gemini
        )
    }

    pub(crate) fn provision(
        provider: AgentProvider,
        command_path: &str,
        hook: &AgentHookLaunch,
        generation: u64,
        pty_id: &str,
    ) -> Result<Self, PtyServiceError> {
        if !Self::supports(provider) {
            return Err(PtyServiceError::InvalidRequest(
                "このproviderには実行可能なtool hook bridgeがありません。",
            ));
        }
        if provider == AgentProvider::Codex {
            validate_codex_cli(command_path)?;
        }
        let hooks_root = hook.runtime_root().join("hooks");
        fs::create_dir_all(&hooks_root).map_err(PtyServiceError::Io)?;
        secure_directory(&hooks_root)?;
        let root = hooks_root.join(format!("{}-{generation}-{}", std::process::id(), pty_id));
        fs::create_dir(&root).map_err(PtyServiceError::Io)?;
        secure_directory(&root)?;
        let mut runtime = Self {
            root,
            prepend_args: Vec::new(),
            append_args: Vec::new(),
            environment: BTreeMap::new(),
        };
        let helper = runtime.root.join(HELPER_NAME);
        let headers = runtime.root.join("headers");
        fs::write(
            &helper,
            "#!/bin/sh\nset -eu\nprovider=$1\nexec curl --silent --show-error --fail-with-body --header @\"$MD_HIVE_HOOK_HEADERS\" --data-binary @- \"$MD_HIVE_HOOK_URL/$provider\"\n",
        )
        .map_err(PtyServiceError::Io)?;
        fs::write(
            &headers,
            format!(
                "Content-Type: application/json\nX-MD-Agent-ID: {}\nX-MD-Hook-Capability: {}\n",
                hook.agent_id(),
                hook.capability()
            ),
        )
        .map_err(PtyServiceError::Io)?;
        secure_file(&helper, 0o700)?;
        secure_file(&headers, 0o600)?;

        let command = format!("{} {}", helper.display(), provider_slug(provider));
        runtime.environment.extend([
            (String::from("MD_HIVE_HOOK_HEADERS"), path_text(&headers)?),
            (String::from("MD_HIVE_HOOK_HELPER"), path_text(&helper)?),
        ]);
        match provider {
            AgentProvider::Claude => {
                let settings = runtime.root.join("claude-settings.json");
                fs::write(&settings, claude_settings(&command)).map_err(PtyServiceError::Io)?;
                secure_file(&settings, 0o600)?;
                runtime
                    .append_args
                    .extend([String::from("--settings"), path_text(&settings)?]);
            }
            AgentProvider::Codex => {
                runtime.prepend_args.extend(codex_override_args(&command));
            }
            AgentProvider::Gemini => {
                let settings = runtime.root.join("gemini-settings.json");
                fs::write(&settings, gemini_settings(&command)).map_err(PtyServiceError::Io)?;
                secure_file(&settings, 0o600)?;
                runtime.environment.insert(
                    String::from("GEMINI_CLI_SYSTEM_SETTINGS_PATH"),
                    path_text(&settings)?,
                );
            }
            AgentProvider::Grok
            | AgentProvider::Kimi
            | AgentProvider::Antigravity
            | AgentProvider::Qwen
            | AgentProvider::OpenCode
            | AgentProvider::Crush
            | AgentProvider::Pi
            | AgentProvider::Copilot
            | AgentProvider::Cursor
            | AgentProvider::Custom => {
                return Err(PtyServiceError::InvalidRequest(
                    "このproviderには実行可能なtool hook bridgeがありません。",
                ));
            }
        }
        Ok(runtime)
    }

    pub(crate) fn apply_args(&self, args: &mut Vec<String>) {
        if !self.prepend_args.is_empty() {
            args.splice(0..0, self.prepend_args.clone());
        }
        args.extend(self.append_args.iter().cloned());
    }

    pub(crate) fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }

    pub(crate) fn cleanup(&mut self) -> Result<(), PtyServiceError> {
        if self.root.as_os_str().is_empty() {
            return Ok(());
        }
        fs::remove_dir_all(&self.root).map_err(PtyServiceError::Io)?;
        self.root = PathBuf::new();
        Ok(())
    }
}

impl Drop for AgentHookRuntime {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Converts the normalized decision to the Claude/Codex hook response vocabulary.
pub fn render_claude_hook_response(
    event_name: &str,
    decision: &AgentHookDecision,
) -> serde_json::Value {
    if event_name == "PreToolUse" && !decision.allow {
        return json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": decision.reason_ja
            }
        });
    }
    if event_name == "Stop"
        && let Some(steer) = decision.steer.as_deref()
    {
        return json!({"decision": "block", "reason": steer});
    }
    if let Some(steer) = decision.steer.as_deref() {
        return json!({"systemMessage": steer});
    }
    json!({})
}

/// Converts the normalized decision to Gemini's direct deny/context vocabulary.
pub fn render_gemini_hook_response(
    event_name: &str,
    decision: &AgentHookDecision,
) -> serde_json::Value {
    if !decision.allow {
        return json!({"decision": "deny", "reason": decision.reason_ja});
    }
    if event_name == "Stop"
        && let Some(steer) = decision.steer.as_deref()
    {
        return json!({"decision": "deny", "reason": steer});
    }
    if let Some(steer) = decision.steer.as_deref() {
        return json!({"hookSpecificOutput": {"additionalContext": steer}});
    }
    json!({})
}

/// Parses the endpoint's normalized decision without ever formatting its body in errors.
pub fn parse_agent_hook_decision(bytes: &[u8]) -> Result<AgentHookDecision, PtyServiceError> {
    serde_json::from_slice(bytes).map_err(|_| {
        PtyServiceError::InvalidRequest("hook decision responseの形式が正しくありません。")
    })
}

fn claude_settings(command: &str) -> String {
    let entry = |matcher: Option<&str>| {
        json!({
            "matcher": matcher,
            "hooks": [{"type": "command", "command": command, "timeout": 30}]
        })
    };
    json!({
        "hooks": {
            "PreToolUse": [entry(Some("*"))],
            "PostToolUse": [entry(Some("*"))],
            "Stop": [entry(None)],
            "SubagentStop": [entry(None)],
            "SessionStart": [entry(None)],
            "UserPromptSubmit": [entry(None)]
        }
    })
    .to_string()
}

fn gemini_settings(command: &str) -> String {
    let hook = |name: &str, matcher: Option<&str>| {
        json!({
            "matcher": matcher,
            "sequential": true,
            "hooks": [{
                "name": format!("munder-hive-{name}"),
                "type": "command",
                "command": command,
                "timeout": 30000
            }]
        })
    };
    json!({
        "hooksConfig": {"enabled": true, "notifications": false},
        "hooks": {
            "SessionStart": [hook("session-start", None)],
            "BeforeAgent": [hook("before-agent", None)],
            "BeforeTool": [hook("before-tool", Some(".*"))],
            "AfterTool": [hook("after-tool", Some(".*"))],
            "AfterAgent": [hook("after-agent", None)]
        }
    })
    .to_string()
}

fn codex_override_args(command: &str) -> Vec<String> {
    let mut args = vec![String::from("--dangerously-bypass-hook-trust")];
    [
        "PreToolUse",
        "PostToolUse",
        "Stop",
        "SubagentStop",
        "SessionStart",
        "UserPromptSubmit",
    ]
    .into_iter()
    .for_each(|event| {
        args.push(String::from("-c"));
        args.push(format!(
            "hooks.{event}=[{{hooks=[{{type=\"command\",command={command:?},timeout=30}}]}}]"
        ));
    });
    args
}

fn validate_codex_cli(command_path: &str) -> Result<(), PtyServiceError> {
    let output = std::process::Command::new(command_path)
        .arg("--help")
        .output()
        .map_err(PtyServiceError::Io)?;
    let help = String::from_utf8_lossy(&output.stdout);
    if !output.status.success()
        || !help.contains("--dangerously-bypass-hook-trust")
        || !help.contains("--strict-config")
    {
        return Err(PtyServiceError::InvalidRequest(
            "インストール済みCodexは安全なtool hook設定に対応していません。",
        ));
    }
    Ok(())
}

const fn provider_slug(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "claude",
        AgentProvider::Codex => "codex",
        AgentProvider::Gemini => "gemini",
        _ => "generic",
    }
}

fn path_text(path: &Path) -> Result<String, PtyServiceError> {
    path.to_str()
        .map(String::from)
        .ok_or(PtyServiceError::InvalidRequest(
            "hook helper pathをUTF-8で表現できません。",
        ))
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<(), PtyServiceError> {
    secure_file(path, 0o700)
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<(), PtyServiceError> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path, mode: u32) -> Result<(), PtyServiceError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(PtyServiceError::Io)
}

#[cfg(not(unix))]
fn secure_file(_path: &Path, _mode: u32) -> Result<(), PtyServiceError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::io::{Read, Write};
    #[cfg(unix)]
    use std::net::TcpListener;
    #[cfg(unix)]
    use std::process::{Command, Stdio};
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;

    use md_web_contracts::domains::pty_agents::{AgentHookDecision, AgentProvider};

    use super::{
        AgentHookRuntime, parse_agent_hook_decision, render_claude_hook_response,
        render_gemini_hook_response,
    };
    #[cfg(unix)]
    use crate::domains::pty_agents::{AgentHookLaunch, PtyServiceError};

    #[test]
    fn pre_tool_denial_uses_provider_permission_vocabulary() {
        let response = render_claude_hook_response(
            "PreToolUse",
            &AgentHookDecision {
                allow: false,
                reason_ja: Some(String::from("停止中")),
                steer: None,
            },
        );
        assert_eq!(response["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    #[test]
    fn stop_steer_becomes_one_provider_block_instruction() {
        let response = render_claude_hook_response(
            "Stop",
            &AgentHookDecision {
                allow: true,
                reason_ja: None,
                steer: Some(String::from("次の作業へ")),
            },
        );
        assert_eq!(response["decision"], "block");
        assert_eq!(response["reason"], "次の作業へ");
    }

    #[test]
    fn normalized_reply_parser_preserves_allow_reason_and_steer() {
        let parsed =
            parse_agent_hook_decision(br#"{"allow":false,"reason_ja":"gate","steer":"next"}"#);
        assert!(matches!(
            parsed,
            Ok(decision)
                if !decision.allow
                    && decision.reason_ja.as_deref() == Some("gate")
                    && decision.steer.as_deref() == Some("next")
        ));
    }

    #[test]
    fn gemini_pre_tool_denial_uses_direct_decision_vocabulary() {
        let response = render_gemini_hook_response(
            "PreToolUse",
            &AgentHookDecision {
                allow: false,
                reason_ja: Some(String::from("停止中")),
                steer: None,
            },
        );
        assert_eq!(response["decision"], "deny");
        assert_eq!(response["reason"], "停止中");
    }

    #[test]
    fn hook_support_is_truthful_for_every_provider_variant() {
        let supported = [
            AgentProvider::Claude,
            AgentProvider::Codex,
            AgentProvider::Gemini,
        ];
        let unsupported = [
            AgentProvider::Grok,
            AgentProvider::Kimi,
            AgentProvider::Antigravity,
            AgentProvider::Qwen,
            AgentProvider::OpenCode,
            AgentProvider::Crush,
            AgentProvider::Pi,
            AgentProvider::Copilot,
            AgentProvider::Cursor,
            AgentProvider::Custom,
        ];
        assert!(supported.into_iter().all(AgentHookRuntime::supports));
        assert!(!unsupported.into_iter().any(AgentHookRuntime::supports));
    }

    #[cfg(unix)]
    #[test]
    fn fake_cli_relay_authenticates_and_receives_provider_decision() -> Result<(), PtyServiceError>
    {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(PtyServiceError::Io)?;
        let address = listener.local_addr().map_err(PtyServiceError::Io)?;
        let runtime_root = std::env::current_dir()
            .map_err(PtyServiceError::Io)?
            .join("target")
            .join(format!("md-hook-test-{}", std::process::id()));
        std::fs::create_dir_all(&runtime_root).map_err(PtyServiceError::Io)?;
        let capability = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let launch = AgentHookLaunch::new(
            format!("http://{address}/internal/hive-hook"),
            "fake-agent",
            capability,
            &runtime_root,
        )?;
        let runtime = AgentHookRuntime::provision(
            AgentProvider::Claude,
            "/bin/sh",
            &launch,
            91,
            "pty-fake-agent",
        )?;
        let server = thread::spawn(move || -> std::io::Result<Vec<u8>> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_millis(200)))?;
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => request.extend_from_slice(&chunk[..read]),
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            let response = br#"{"allow":false,"reason_ja":"gate","steer":null}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )?;
            stream.write_all(response)?;
            Ok(request)
        });
        let helper = runtime
            .environment()
            .get("MD_HIVE_HOOK_HELPER")
            .ok_or(PtyServiceError::InvalidRequest("hook helperがありません。"))?;
        let mut command = Command::new(helper);
        command
            .arg("claude")
            .env("MD_HIVE_HOOK_URL", launch.endpoint_url())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        for (key, value) in runtime.environment() {
            command.env(key, value);
        }
        let mut child = command.spawn().map_err(PtyServiceError::Io)?;
        child
            .stdin
            .take()
            .ok_or(PtyServiceError::InvalidRequest(
                "fake CLI stdinがありません。",
            ))?
            .write_all(br#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#)
            .map_err(PtyServiceError::Io)?;
        let output = child.wait_with_output().map_err(PtyServiceError::Io)?;
        if !output.status.success() {
            return Err(PtyServiceError::InvalidRequest(
                "fake CLI hook relayが失敗しました。",
            ));
        }
        let decision = parse_agent_hook_decision(&output.stdout)?;
        assert!(!decision.allow);
        let request = server
            .join()
            .map_err(|_| PtyServiceError::InvalidRequest("fake hook serverが失敗しました。"))?
            .map_err(PtyServiceError::Io)?;
        let request_text = String::from_utf8_lossy(&request);
        assert!(request_text.contains("X-MD-Agent-ID: fake-agent"));
        assert!(request_text.contains("X-MD-Hook-Capability: aaaaaaaa"));
        drop(runtime);
        std::fs::remove_dir_all(runtime_root).map_err(PtyServiceError::Io)?;
        Ok(())
    }
}
