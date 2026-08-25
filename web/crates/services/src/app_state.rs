use std::time::Instant;

use md_web_contracts::{HealthSnapshot, PersistenceStatus};

use crate::health::snapshot;
use crate::persistence::probe_from_environment;

/// Process-lifetime state shared by the local web server.
pub struct AppState {
    started_at: Instant,
    app_version: &'static str,
    persistence: PersistenceStatus,
}

impl AppState {
    /// Creates the state once and performs the bounded persistence probe.
    pub async fn initialize(app_version: &'static str) -> Self {
        Self {
            started_at: Instant::now(),
            app_version,
            persistence: probe_from_environment().await,
        }
    }

    /// Returns the current shared health contract without exposing server configuration.
    pub fn health_snapshot(&self) -> HealthSnapshot {
        snapshot(self.started_at, self.app_version, self.persistence)
    }

    #[cfg(test)]
    pub(crate) fn with_status(
        app_version: &'static str,
        started_at: Instant,
        persistence: PersistenceStatus,
    ) -> Self {
        Self {
            started_at,
            app_version,
            persistence,
        }
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::{PersistenceCode, PersistenceStatus};

    use super::AppState;

    #[tokio::test]
    async fn initialize_without_config_is_degraded() {
        let state = AppState::initialize("0.1.0").await;

        assert!(matches!(
            state.health_snapshot().persistence,
            PersistenceStatus::Degraded {
                code: PersistenceCode::MissingConfig
            }
        ));
    }

    #[test]
    fn health_snapshot_preserves_empty_version() {
        let state = AppState::with_status("", std::time::Instant::now(), PersistenceStatus::Closed);

        assert!(state.health_snapshot().app_version.is_empty());
    }
}
