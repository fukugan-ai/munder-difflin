use std::time::Instant;

use md_web_contracts::{HealthSnapshot, PersistenceStatus};

use crate::DomainRegistry;
use crate::health::snapshot;
use crate::persistence::probe_from_environment;

/// Process-lifetime state shared by the local web server.
pub struct AppState {
    started_at: Instant,
    app_version: &'static str,
    persistence: PersistenceStatus,
    domains: DomainRegistry,
}

impl AppState {
    /// Creates the state once and performs the bounded persistence probe.
    pub async fn initialize(app_version: &'static str) -> Self {
        Self {
            started_at: Instant::now(),
            app_version,
            persistence: probe_from_environment().await,
            domains: DomainRegistry::integrated(),
        }
    }

    /// Returns the current shared health contract without exposing server configuration.
    pub fn health_snapshot(&self) -> HealthSnapshot {
        snapshot(self.started_at, self.app_version, self.persistence)
    }

    /// Re-probes persistence while preserving process lifetime/version identity.
    /// This lets a database restored after startup move health out of degraded.
    pub async fn refresh_health_snapshot(&self) -> HealthSnapshot {
        self.health_snapshot_with_status(probe_from_environment().await)
    }

    fn health_snapshot_with_status(&self, persistence: PersistenceStatus) -> HealthSnapshot {
        snapshot(self.started_at, self.app_version, persistence)
    }

    /// Returns the compile-time registry of integrated Web domains.
    pub fn domains(&self) -> &DomainRegistry {
        &self.domains
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
            domains: DomainRegistry::integrated(),
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

    #[test]
    fn initial_state_contains_integrated_domains() {
        let state = AppState::with_status(
            "0.1.0",
            std::time::Instant::now(),
            PersistenceStatus::Closed,
        );

        assert_eq!(state.domains().registered().len(), 8);
    }

    #[test]
    fn refreshed_health_can_move_from_degraded_to_ready() {
        let state = AppState::with_status(
            "0.4.5",
            std::time::Instant::now(),
            PersistenceStatus::Degraded {
                code: PersistenceCode::Unreachable,
            },
        );
        assert!(matches!(
            state
                .health_snapshot_with_status(PersistenceStatus::Degraded {
                    code: PersistenceCode::Unreachable,
                })
                .persistence,
            PersistenceStatus::Degraded { .. }
        ));
        assert_eq!(
            state
                .health_snapshot_with_status(PersistenceStatus::Ready { writes: true })
                .persistence,
            PersistenceStatus::Ready { writes: true }
        );
    }
}
