use std::time::{Duration, Instant};

use md_web_contracts::{HealthSnapshot, PersistenceStatus};

pub(crate) fn snapshot(
    started_at: Instant,
    app_version: &'static str,
    persistence: PersistenceStatus,
) -> HealthSnapshot {
    HealthSnapshot {
        app_version: String::from(app_version),
        uptime_ms: saturating_millis(started_at.elapsed()),
        persistence,
    }
}

const fn saturating_millis(duration: Duration) -> u64 {
    let millis = duration.as_millis();
    if millis > u64::MAX as u128 {
        u64::MAX
    } else {
        millis as u64
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use md_web_contracts::PersistenceStatus;

    use super::{saturating_millis, snapshot};

    #[test]
    fn snapshot_reports_elapsed_milliseconds() {
        let started_at = Instant::now() - Duration::from_millis(7);

        assert!(snapshot(started_at, "1", PersistenceStatus::Closed).uptime_ms >= 7);
    }

    #[test]
    fn snapshot_accepts_empty_version() {
        let result = snapshot(Instant::now(), "", PersistenceStatus::Closed);

        assert!(result.app_version.is_empty());
    }

    #[test]
    fn uptime_saturates_at_u64_max() {
        assert_eq!(saturating_millis(Duration::MAX), u64::MAX);
    }
}
