use serde::{Deserialize, Serialize};

/// Desktop capabilities that require an explicit Web equivalent or N/A marker.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCapability {
    WindowBounds,
    LoginItem,
    NativeAutoUpdate,
    OsSettingsDeepLink,
    NativeDesktopNotification,
    KeepDisplayAwake,
    MultiFloor,
    ExternalLinks,
}

/// Whether a capability exists in the Web runtime.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    Available,
    BrowserRestricted,
    ExternalSetup,
    NotApplicable,
}

/// Human-readable runtime capability report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilitySupport {
    pub capability: AppCapability,
    pub availability: CapabilityAvailability,
    pub detail_ja: String,
}

/// Server build identity shown in Settings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppInfo {
    pub version: String,
    pub changelog_excerpt: String,
    pub runtime_label: String,
}

/// Validated opaque floor/session identifier.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FloorId(String);

impl FloorId {
    /// Accepts only the server-generated `floor-<digits>` representation.
    pub fn parse(value: String) -> Option<Self> {
        let suffix = value.strip_prefix("floor-")?;
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(Self(value))
    }

    /// Borrows the stable URL-safe identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Creates a separately-namespaced browser floor.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateFloorRequest {
    pub label: Option<String>,
}

/// URL returned to the browser for opening in another tab or PC.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateFloorResponse {
    pub floor_id: FloorId,
    pub path: String,
}

/// Explicit shutdown request; closing a browser tab never produces this request.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShutdownRequest {
    pub expected_running_terminals: u32,
    pub graceful: bool,
}

/// Result of a server-owned graceful shutdown protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShutdownResult {
    pub accepted: bool,
    /// Always false until a server-owned terminal teardown executor is wired.
    pub executed: bool,
    pub running_terminals: u32,
    pub detail_ja: String,
}

/// Exact namespace reset confirmation. Phrase must be `RESET <namespace>`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResetNamespaceRequest {
    pub namespace: String,
    pub confirmation: String,
}

/// Bounded transactional reset receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResetResult {
    pub reset: bool,
    pub next_path: String,
    pub detail_ja: String,
    pub deleted_rows: u64,
}

#[cfg(test)]
mod tests {
    use super::FloorId;

    #[test]
    fn floor_id_rejects_empty_suffix() {
        assert_eq!(FloorId::parse(String::from("floor-")), None);
    }

    #[test]
    fn floor_id_accepts_numeric_suffix() {
        let floor = FloorId::parse(String::from("floor-42"));

        assert_eq!(floor.as_ref().map(FloorId::as_str), Some("floor-42"));
    }
}
