use serde::{Deserialize, Serialize};

/// Host operating-system family used only for install guidance.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostPlatform {
    Linux,
    MacOs,
    Windows,
    Other,
}

/// Role an external executable plays in the local server.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Prerequisite,
    Memory,
    Engine,
}

/// Server-host tool availability safe to render in a LAN browser.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolStatus {
    pub id: String,
    pub label: String,
    pub kind: ToolKind,
    pub why_ja: String,
    pub essential: bool,
    pub found: bool,
    pub path: Option<String>,
    pub detail_ja: Option<String>,
    pub install_command: Option<String>,
    pub docs_url: Option<String>,
    /// Always `server_host`; a remote browser's own PATH is never probed.
    pub observed_on: String,
}

impl ToolStatus {
    /// True when the app's recommended baseline is incomplete.
    pub const fn blocks_recommended_setup(&self) -> bool {
        self.essential && !self.found
    }
}

#[cfg(test)]
mod tests {
    use super::{ToolKind, ToolStatus};

    #[test]
    fn optional_missing_engine_does_not_block_setup() {
        let tool = ToolStatus {
            id: String::from("engine:codex"),
            label: String::from("Codex"),
            kind: ToolKind::Engine,
            why_ja: String::new(),
            essential: false,
            found: false,
            path: None,
            detail_ja: None,
            install_command: None,
            docs_url: None,
            observed_on: String::from("server_host"),
        };

        assert!(!tool.blocks_recommended_setup());
    }
}
