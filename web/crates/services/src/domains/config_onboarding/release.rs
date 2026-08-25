use std::fmt::{Display, Formatter};
use std::future::Future;
use std::time::Duration;

use md_web_contracts::domains::config_onboarding::{ReleaseRepository, UpdateStatus};
use serde::Deserialize;

const DEFAULT_RELEASE_REPOSITORY: &str = "fukugan-ai/munder-difflin";
const ORIGINAL_RELEASE_REPOSITORY: &str = "chaitanyagiri/munder-difflin";

/// Invalid or disallowed release-source configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseSourceError {
    InvalidSlug,
    OriginalRepositoryBlocked,
}

impl Display for ReleaseSourceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSlug => "MD_RELEASE_REPO must be owner/name",
            Self::OriginalRepositoryBlocked => {
                "the upstream repository is not a writable update source"
            }
        })
    }
}

impl std::error::Error for ReleaseSourceError {}

/// Failure while reading browser-safe metadata from the fork's release feed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseCheckError {
    ClientUnavailable,
    RequestFailed,
    InvalidResponse,
    InvalidVersion,
}

impl Display for ReleaseCheckError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ClientUnavailable => "release client is unavailable",
            Self::RequestFailed => "release metadata request failed",
            Self::InvalidResponse => "release metadata response is invalid",
            Self::InvalidVersion => "release version is invalid",
        })
    }
}

impl std::error::Error for ReleaseCheckError {}

/// Minimal release metadata accepted by the Web update UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseMetadata {
    pub version: String,
    pub release_url: String,
    pub notes: Option<String>,
}

/// Read-only boundary used to keep network access out of update decisions.
pub trait ReleaseLookup: Sync {
    fn latest(
        &self,
        repository: &ReleaseRepository,
    ) -> impl Future<Output = Result<ReleaseMetadata, ReleaseCheckError>> + Send;
}

/// GitHub API client restricted to public release reads.
#[derive(Clone, Debug)]
pub struct GitHubReleaseClient {
    client: reqwest::Client,
}

impl GitHubReleaseClient {
    /// Builds a finite-timeout client with no credential or write support.
    pub fn new() -> Result<Self, ReleaseCheckError> {
        let client = reqwest::Client::builder()
            .user_agent("munder-difflin-web-release-check")
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(8))
            .build()
            .map_err(|_| ReleaseCheckError::ClientUnavailable)?;
        Ok(Self { client })
    }
}

#[derive(Deserialize)]
struct GitHubReleaseResponse {
    tag_name: String,
    html_url: String,
    body: Option<String>,
}

impl ReleaseLookup for GitHubReleaseClient {
    async fn latest(
        &self,
        repository: &ReleaseRepository,
    ) -> Result<ReleaseMetadata, ReleaseCheckError> {
        let endpoint = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            repository.owner, repository.name
        );
        let response = self
            .client
            .get(endpoint)
            .send()
            .await
            .map_err(|_| ReleaseCheckError::RequestFailed)?
            .error_for_status()
            .map_err(|_| ReleaseCheckError::RequestFailed)?
            .json::<GitHubReleaseResponse>()
            .await
            .map_err(|_| ReleaseCheckError::InvalidResponse)?;
        let expected_prefix = format!(
            "https://github.com/{}/{}/releases/",
            repository.owner, repository.name
        );
        let release_url = if response.html_url.starts_with(&expected_prefix) {
            response.html_url
        } else {
            latest_release_url(repository)
        };
        Ok(ReleaseMetadata {
            version: response.tag_name,
            release_url,
            notes: response.body.map(|body| body.chars().take(4_000).collect()),
        })
    }
}

/// Resolves `MD_RELEASE_REPO`, defaulting to the user's fork and refusing upstream.
pub fn resolve_release_repository(
    configured: Option<&str>,
) -> Result<ReleaseRepository, ReleaseSourceError> {
    let slug = configured
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_RELEASE_REPOSITORY);
    if slug.eq_ignore_ascii_case(ORIGINAL_RELEASE_REPOSITORY) {
        return Err(ReleaseSourceError::OriginalRepositoryBlocked);
    }
    let mut parts = slug.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if owner.is_empty()
        || name.is_empty()
        || parts.next().is_some()
        || !valid_slug_part(owner)
        || !valid_slug_part(name)
    {
        return Err(ReleaseSourceError::InvalidSlug);
    }
    Ok(ReleaseRepository {
        owner: String::from(owner),
        name: String::from(name),
    })
}

/// Builds the public releases/latest page for a validated repository.
pub fn latest_release_url(repository: &ReleaseRepository) -> String {
    format!(
        "https://github.com/{}/{}/releases/latest",
        repository.owner, repository.name
    )
}

/// Compares a public release read with the running server version.
pub async fn check_for_update(
    lookup: &impl ReleaseLookup,
    repository: &ReleaseRepository,
    current_version: &str,
) -> Result<UpdateStatus, ReleaseCheckError> {
    let current = parse_version(current_version)?;
    let release = lookup.latest(repository).await?;
    let latest = parse_version(&release.version)?;
    if latest > current {
        Ok(UpdateStatus::Available {
            version: release.version,
            release_url: release.release_url,
            notes: release.notes,
        })
    } else {
        Ok(UpdateStatus::Current)
    }
}

fn parse_version(value: &str) -> Result<(u64, u64, u64), ReleaseCheckError> {
    let stable = value
        .trim()
        .strip_prefix('v')
        .unwrap_or(value.trim())
        .split_once('-')
        .map_or_else(
            || value.trim().strip_prefix('v').unwrap_or(value.trim()),
            |pair| pair.0,
        );
    let mut parts = stable.split('.');
    let major = parse_version_part(parts.next())?;
    let minor = parse_version_part(parts.next())?;
    let patch = parse_version_part(parts.next())?;
    if parts.next().is_some() {
        return Err(ReleaseCheckError::InvalidVersion);
    }
    Ok((major, minor, patch))
}

fn parse_version_part(value: Option<&str>) -> Result<u64, ReleaseCheckError> {
    value
        .filter(|part| !part.is_empty())
        .ok_or(ReleaseCheckError::InvalidVersion)?
        .parse()
        .map_err(|_| ReleaseCheckError::InvalidVersion)
}

fn valid_slug_part(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::config_onboarding::{ReleaseRepository, UpdateStatus};

    use super::{
        GitHubReleaseClient, ReleaseCheckError, ReleaseLookup, ReleaseMetadata, ReleaseSourceError,
        check_for_update, latest_release_url, resolve_release_repository,
    };

    struct FixedLookup(ReleaseMetadata);

    impl ReleaseLookup for FixedLookup {
        async fn latest(
            &self,
            _repository: &ReleaseRepository,
        ) -> Result<ReleaseMetadata, ReleaseCheckError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn default_source_is_users_fork() {
        let repository = resolve_release_repository(None);

        assert!(matches!(repository, Ok(value) if value.slug() == "fukugan-ai/munder-difflin"));
    }

    #[test]
    fn original_source_is_blocked() {
        let repository = resolve_release_repository(Some("chaitanyagiri/munder-difflin"));

        assert_eq!(
            repository,
            Err(ReleaseSourceError::OriginalRepositoryBlocked)
        );
    }

    #[test]
    fn latest_url_uses_validated_repository() {
        let repository = resolve_release_repository(Some("example/fork"));

        assert!(
            matches!(repository, Ok(value) if latest_release_url(&value) == "https://github.com/example/fork/releases/latest")
        );
    }

    #[test]
    fn github_client_builds_without_credentials() {
        assert!(GitHubReleaseClient::new().is_ok());
    }

    #[tokio::test]
    async fn newer_fork_release_is_reported_without_installing() {
        let repository = resolve_release_repository(Some("example/fork"))
            .unwrap_or_else(|error| panic!("test repository must be valid: {error}"));
        let lookup = FixedLookup(ReleaseMetadata {
            version: String::from("v0.2.0"),
            release_url: String::from("https://github.com/example/fork/releases/tag/v0.2.0"),
            notes: Some(String::from("changes")),
        });

        let status = check_for_update(&lookup, &repository, "0.1.0").await;

        assert!(
            matches!(status, Ok(UpdateStatus::Available { version, .. }) if version == "v0.2.0")
        );
    }

    #[tokio::test]
    async fn equal_release_is_current() {
        let repository = resolve_release_repository(Some("example/fork"))
            .unwrap_or_else(|error| panic!("test repository must be valid: {error}"));
        let lookup = FixedLookup(ReleaseMetadata {
            version: String::from("0.1.0"),
            release_url: String::from("https://github.com/example/fork/releases/tag/0.1.0"),
            notes: None,
        });

        let status = check_for_update(&lookup, &repository, "0.1.0").await;

        assert_eq!(status, Ok(UpdateStatus::Current));
    }
}
