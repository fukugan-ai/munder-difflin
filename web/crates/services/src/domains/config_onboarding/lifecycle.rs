use std::fmt::{Display, Formatter};

use md_web_contracts::domains::config_onboarding::{
    AppCapability, AppInfo, CapabilityAvailability, CapabilitySupport, CreateFloorRequest,
    CreateFloorResponse, FloorId, ShutdownRequest, ShutdownResult,
};

/// Failure creating a new browser floor namespace.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleError {
    FloorIdExhausted,
    InvalidBasePath,
}

impl Display for LifecycleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::FloorIdExhausted => "floor identifier space exhausted",
            Self::InvalidBasePath => "floor base path must start with /",
        })
    }
}

impl std::error::Error for LifecycleError {}

/// Process-owned monotonic floor allocator. Browser disconnects do not remove floors.
#[derive(Debug, Default)]
pub struct FloorRegistry {
    next_id: u64,
}

impl FloorRegistry {
    /// Creates an empty registry whose first floor is `floor-1`.
    pub const fn new() -> Self {
        Self { next_id: 1 }
    }

    /// Allocates a floor URL. The optional label is presentation-only and is not part of identity.
    pub fn create(
        &mut self,
        base_path: &str,
        _request: &CreateFloorRequest,
    ) -> Result<CreateFloorResponse, LifecycleError> {
        if !base_path.starts_with('/') {
            return Err(LifecycleError::InvalidBasePath);
        }
        let current = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(LifecycleError::FloorIdExhausted)?;
        let raw = format!("floor-{current}");
        let floor_id = FloorId::parse(raw).ok_or(LifecycleError::FloorIdExhausted)?;
        let prefix = base_path.trim_end_matches('/');
        Ok(CreateFloorResponse {
            path: format!("{prefix}/{}", floor_id.as_str()),
            floor_id,
        })
    }
}

/// Explicit capability table shown instead of silently emulating Electron APIs.
pub fn web_capabilities() -> Vec<CapabilitySupport> {
    vec![
        support(
            AppCapability::WindowBounds,
            CapabilityAvailability::NotApplicable,
            "ブラウザーがウィンドウ位置を管理します。",
        ),
        support(
            AppCapability::LoginItem,
            CapabilityAvailability::ExternalSetup,
            "サーバーの自動起動はsystemdなどで設定します。",
        ),
        support(
            AppCapability::NativeAutoUpdate,
            CapabilityAvailability::NotApplicable,
            "Web版はリリース確認とダウンロード案内だけを行います。",
        ),
        support(
            AppCapability::OsSettingsDeepLink,
            CapabilityAvailability::NotApplicable,
            "OS設定の専用リンクはWeb版では使いません。",
        ),
        support(
            AppCapability::NativeDesktopNotification,
            CapabilityAvailability::BrowserRestricted,
            "通知はブラウザー権限とsecure contextに依存します。",
        ),
        support(
            AppCapability::KeepDisplayAwake,
            CapabilityAvailability::NotApplicable,
            "サーバー処理はブラウザー画面のスリープと独立して動きます。",
        ),
        support(
            AppCapability::MultiFloor,
            CapabilityAvailability::NotApplicable,
            "floor別のrouteとruntime namespaceが未接続のため、現在は利用できません。",
        ),
        support(
            AppCapability::ExternalLinks,
            CapabilityAvailability::Available,
            "許可されたHTTPSリンクを別タブで開きます。",
        ),
    ]
}

/// Builds server identity without reading a client OS or Electron package.
pub fn app_info(version: &str, changelog_excerpt: &str) -> AppInfo {
    AppInfo {
        version: String::from(version),
        changelog_excerpt: String::from(changelog_excerpt),
        runtime_label: String::from("ローカルWeb版（Dioxus + Rust/WASM）"),
    }
}

/// Validates the terminal count observed by the confirmation screen.
/// Execution remains false until the process registry supplies one atomic teardown operation.
pub fn shutdown_decision(request: ShutdownRequest, running_terminals: u32) -> ShutdownResult {
    let accepted = request.graceful && request.expected_running_terminals == running_terminals;
    ShutdownResult {
        accepted,
        executed: false,
        running_terminals,
        detail_ja: if accepted {
            String::from(
                "停止条件は一致しましたが、一括停止executorは未接続のため実行していません。",
            )
        } else {
            String::from("稼働中ターミナル数が確認時点から変わったため、停止していません。")
        },
    }
}

fn support(
    capability: AppCapability,
    availability: CapabilityAvailability,
    detail_ja: &str,
) -> CapabilitySupport {
    CapabilitySupport {
        capability,
        availability,
        detail_ja: String::from(detail_ja),
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::config_onboarding::{
        AppCapability, CapabilityAvailability, CreateFloorRequest, ShutdownRequest,
    };

    use super::{FloorRegistry, LifecycleError, app_info, shutdown_decision, web_capabilities};

    #[test]
    fn first_floor_has_stable_path() {
        let mut registry = FloorRegistry::new();
        let response = registry.create("/floor", &CreateFloorRequest::default());

        assert!(matches!(response, Ok(value) if value.path == "/floor/floor-1"));
    }

    #[test]
    fn floor_rejects_relative_base_path() {
        let mut registry = FloorRegistry::new();
        let response = registry.create("floor", &CreateFloorRequest::default());

        assert_eq!(response, Err(LifecycleError::InvalidBasePath));
    }

    #[test]
    fn capability_table_marks_unrouted_multi_floor_na() {
        let capabilities = web_capabilities();

        assert!(capabilities.iter().any(|support| {
            support.capability == AppCapability::MultiFloor
                && support.availability == CapabilityAvailability::NotApplicable
        }));
    }

    #[test]
    fn app_info_names_web_runtime() {
        let info = app_info("0.1.0", "changes");

        assert!(info.runtime_label.contains("Web"));
    }

    #[test]
    fn stale_shutdown_confirmation_is_rejected() {
        let result = shutdown_decision(
            ShutdownRequest {
                expected_running_terminals: 1,
                graceful: true,
            },
            2,
        );

        assert!(!result.accepted);
        assert!(!result.executed);
    }
}
