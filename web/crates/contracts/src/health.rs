use serde::{Deserialize, Serialize};

/// Durable persistence state exposed by the local service.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PersistenceStatus {
    Closed,
    Ready { writes: bool },
    Degraded { code: PersistenceCode },
}

/// Stable reason for degraded persistence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistenceCode {
    MissingConfig,
    ConfigInvalid,
    Unreachable,
    SchemaMismatch,
    NamespaceLocked,
    WriteFailed,
}

/// Current application and persistence health.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HealthSnapshot {
    pub app_version: String,
    pub uptime_ms: u64,
    pub persistence: PersistenceStatus,
}

#[cfg(test)]
mod tests {
    use super::{HealthSnapshot, PersistenceCode, PersistenceStatus};

    #[test]
    fn health_snapshot_accepts_maximum_uptime() {
        let snapshot = HealthSnapshot {
            app_version: String::from("0.4.5"),
            uptime_ms: u64::MAX,
            persistence: PersistenceStatus::Closed,
        };

        assert_eq!(snapshot.uptime_ms, u64::MAX);
    }

    #[test]
    fn ready_status_reports_writes_enabled() {
        let status = PersistenceStatus::Ready { writes: true };

        assert_eq!(status, PersistenceStatus::Ready { writes: true });
    }

    #[test]
    fn degraded_status_retains_reason_code() {
        let status = PersistenceStatus::Degraded {
            code: PersistenceCode::SchemaMismatch,
        };

        assert_eq!(
            status,
            PersistenceStatus::Degraded {
                code: PersistenceCode::SchemaMismatch,
            }
        );
    }
}
