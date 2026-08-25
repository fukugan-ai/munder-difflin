use serde::{Deserialize, Serialize};

/// Repository queried for Web-app release metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReleaseRepository {
    pub owner: String,
    pub name: String,
}

impl ReleaseRepository {
    /// Returns the canonical `owner/name` slug.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Browser-safe update pipeline. Native Electron installation is intentionally absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    Idle,
    Checking,
    Current,
    Available {
        version: String,
        release_url: String,
        notes: Option<String>,
    },
    Error {
        message_ja: String,
    },
}

/// Actions available in a browser-hosted build.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAction {
    None,
    Check,
    OpenRelease,
}

impl UpdateStatus {
    /// Maps status to the only safe browser action for that state.
    pub const fn action(&self) -> UpdateAction {
        match self {
            Self::Idle | Self::Current | Self::Error { .. } => UpdateAction::Check,
            Self::Checking => UpdateAction::None,
            Self::Available { .. } => UpdateAction::OpenRelease,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReleaseRepository, UpdateAction, UpdateStatus};

    #[test]
    fn available_update_opens_release_instead_of_native_install() {
        let status = UpdateStatus::Available {
            version: String::from("1.0.0"),
            release_url: String::from("https://github.com/fukugan-ai/munder-difflin/releases/1"),
            notes: None,
        };

        assert_eq!(status.action(), UpdateAction::OpenRelease);
    }

    #[test]
    fn repository_slug_preserves_fork_identity() {
        let repository = ReleaseRepository {
            owner: String::from("fukugan-ai"),
            name: String::from("munder-difflin"),
        };

        assert_eq!(repository.slug(), "fukugan-ai/munder-difflin");
    }
}
