use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use md_web_contracts::domains::pty_agents::AgentHookRequest;

use super::error::PtyServiceError;

pub const HOOK_URL_ENV: &str = "MD_HIVE_HOOK_URL";
pub const HOOK_AGENT_ID_ENV: &str = "MD_HIVE_AGENT_ID";
pub const HOOK_CAPABILITY_ENV: &str = "MD_HIVE_HOOK_CAPABILITY";
pub const HOOK_HEADERS_ENV: &str = "MD_HIVE_HOOK_HEADERS";
pub const HOOK_HELPER_ENV: &str = "MD_HIVE_HOOK_HELPER";
pub const HOOK_ENV_KEYS: [&str; 5] = [
    HOOK_URL_ENV,
    HOOK_AGENT_ID_ENV,
    HOOK_CAPABILITY_ENV,
    HOOK_HEADERS_ENV,
    HOOK_HELPER_ENV,
];

const MIN_CAPABILITY_BYTES: usize = 32;
const MAX_CAPABILITY_BYTES: usize = 512;
const MAX_URL_BYTES: usize = 2048;
const MAX_AGENT_ID_BYTES: usize = 128;

/// Server-only launch material injected into exactly one agent child process.
///
/// This type deliberately has no serde implementation, so it cannot enter browser DTOs.
#[derive(Clone)]
pub struct AgentHookLaunch {
    endpoint_url: String,
    agent_id: String,
    capability: String,
    runtime_root: PathBuf,
}

impl Debug for AgentHookLaunch {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentHookLaunch")
            .field("endpoint_url", &self.endpoint_url)
            .field("agent_id", &self.agent_id)
            .field("capability", &"[REDACTED]")
            .field("runtime_root", &self.runtime_root)
            .finish()
    }
}

impl AgentHookLaunch {
    /// Validates a server-generated capability and an absolute internal HTTP(S) endpoint.
    pub fn new(
        endpoint_url: impl Into<String>,
        agent_id: impl Into<String>,
        capability: impl Into<String>,
        runtime_root: impl Into<PathBuf>,
    ) -> Result<Self, PtyServiceError> {
        let endpoint_url = endpoint_url.into();
        let agent_id = agent_id.into();
        let capability = capability.into();
        let runtime_root = runtime_root.into();
        validate_agent_id(&agent_id)?;
        validate_capability(&capability)?;
        validate_endpoint(&endpoint_url)?;
        validate_runtime_root(&runtime_root)?;
        Ok(Self {
            endpoint_url,
            agent_id,
            capability,
            runtime_root,
        })
    }

    pub(crate) fn endpoint_url(&self) -> &str {
        &self.endpoint_url
    }

    pub(crate) fn agent_id(&self) -> &str {
        &self.agent_id
    }

    pub(crate) fn capability(&self) -> &str {
        &self.capability
    }

    pub(crate) fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }
}

/// Process-owned bearer capability set. Rotation invalidates the previous value immediately.
#[derive(Default)]
pub struct AgentHookCapabilities {
    entries: Mutex<BTreeMap<String, String>>,
}

impl AgentHookCapabilities {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn rotate(&self, launch: &AgentHookLaunch) -> Result<(), PtyServiceError> {
        self.entries
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?
            .insert(launch.agent_id.clone(), launch.capability.clone());
        Ok(())
    }

    pub fn verify_request(&self, request: &AgentHookRequest) -> Result<bool, PtyServiceError> {
        validate_agent_id(&request.agent_id)?;
        validate_capability(&request.capability)?;
        let entries = self
            .entries
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        Ok(entries
            .get(&request.agent_id)
            .is_some_and(|expected| constant_time_eq(expected, &request.capability)))
    }

    pub(crate) fn remove_if_matches(
        &self,
        agent_id: &str,
        capability: &str,
    ) -> Result<(), PtyServiceError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        if entries
            .get(agent_id)
            .is_some_and(|expected| constant_time_eq(expected, capability))
        {
            entries.remove(agent_id);
        }
        Ok(())
    }
}

fn validate_agent_id(agent_id: &str) -> Result<(), PtyServiceError> {
    let normalized = agent_id.strip_prefix("pty-").unwrap_or(agent_id);
    if normalized.is_empty()
        || normalized.len() > MAX_AGENT_ID_BYTES
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PtyServiceError::InvalidRequest(
            "hookのエージェントIDが正しくありません。",
        ));
    }
    Ok(())
}

fn validate_capability(capability: &str) -> Result<(), PtyServiceError> {
    if !(MIN_CAPABILITY_BYTES..=MAX_CAPABILITY_BYTES).contains(&capability.len())
        || !capability
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(PtyServiceError::InvalidRequest(
            "hook capabilityの形式が正しくありません。",
        ));
    }
    Ok(())
}

fn validate_endpoint(endpoint: &str) -> Result<(), PtyServiceError> {
    if endpoint.len() > MAX_URL_BYTES {
        return Err(PtyServiceError::InvalidRequest(
            "hook endpoint URLが長すぎます。",
        ));
    }
    let parsed = reqwest::Url::parse(endpoint).map_err(|_| {
        PtyServiceError::InvalidRequest("hook endpoint URLの形式が正しくありません。")
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.host_str().is_none()
    {
        return Err(PtyServiceError::InvalidRequest(
            "hook endpoint URLは認証情報・query・fragmentを含まないHTTP(S) URLにしてください。",
        ));
    }
    Ok(())
}

fn validate_runtime_root(root: &Path) -> Result<(), PtyServiceError> {
    if !root.is_absolute() || !root.is_dir() || root.to_str().is_none() {
        return Err(PtyServiceError::InvalidRequest(
            "hook runtime rootは既存の絶対ディレクトリにしてください。",
        ));
    }
    Ok(())
}

fn constant_time_eq(expected: &str, actual: &str) -> bool {
    let expected = expected.as_bytes();
    let actual = actual.as_bytes();
    let max_len = expected.len().max(actual.len());
    let mut difference = expected.len() ^ actual.len();
    for index in 0..max_len {
        difference |= usize::from(
            expected.get(index).copied().unwrap_or(0) ^ actual.get(index).copied().unwrap_or(0),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{AgentHookEvent, AgentHookRequest};
    use serde_json::Value;

    use super::{AgentHookCapabilities, AgentHookLaunch};

    fn launch(capability: &str) -> Result<AgentHookLaunch, super::PtyServiceError> {
        AgentHookLaunch::new(
            "http://127.0.0.1:5001/internal/hive-hook",
            "dev-1",
            capability,
            std::env::current_dir().map_err(super::PtyServiceError::Io)?,
        )
    }

    fn request(capability: &str) -> AgentHookRequest {
        AgentHookRequest {
            agent_id: String::from("dev-1"),
            capability: String::from(capability),
            event_id: String::from("evt-1"),
            event: AgentHookEvent::PreToolUse,
            tool_name: Some(String::from("Bash")),
            payload: Value::Null,
        }
    }

    #[test]
    fn capability_is_absent_until_installed() {
        let capabilities = AgentHookCapabilities::new();
        assert!(matches!(
            capabilities.verify_request(&request("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")),
            Ok(false)
        ));
    }

    #[test]
    fn rotation_invalidates_old_capability_and_accepts_new_one()
    -> Result<(), super::PtyServiceError> {
        let capabilities = AgentHookCapabilities::new();
        let old = launch("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
        let new = launch("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")?;
        assert!(capabilities.rotate(&old).is_ok());
        assert!(matches!(
            capabilities.verify_request(&request(old.capability())),
            Ok(true)
        ));
        assert!(capabilities.rotate(&new).is_ok());
        assert!(matches!(
            capabilities.verify_request(&request(old.capability())),
            Ok(false)
        ));
        assert!(matches!(
            capabilities.verify_request(&request(new.capability())),
            Ok(true)
        ));
        Ok(())
    }

    #[test]
    fn removing_old_lease_cannot_remove_rotated_capability() -> Result<(), super::PtyServiceError> {
        let capabilities = AgentHookCapabilities::new();
        let old = launch("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
        let new = launch("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")?;
        assert!(capabilities.rotate(&new).is_ok());
        assert!(
            capabilities
                .remove_if_matches(old.agent_id(), old.capability())
                .is_ok()
        );
        assert!(matches!(
            capabilities.verify_request(&request(new.capability())),
            Ok(true)
        ));
        Ok(())
    }

    #[test]
    fn debug_output_redacts_capability() -> Result<(), super::PtyServiceError> {
        let launch = launch("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?;
        let debug = format!("{launch:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains(launch.capability()));
        Ok(())
    }
}
