use serde::{Deserialize, Serialize};

use crate::{DomainInvalidated, HealthSnapshot, PersistenceStatus};

/// Event emitted by the local Dioxus service.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AppEvent {
    HealthUpdated(HealthSnapshot),
    PersistenceChanged(PersistenceStatus),
    DomainInvalidated(DomainInvalidated),
}

/// Ordered event stream item shared with the browser client.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EventEnvelope {
    pub seq: u64,
    pub ts_ms: i64,
    pub event: AppEvent,
}

#[cfg(test)]
mod tests {
    use super::{AppEvent, EventEnvelope};
    use crate::PersistenceStatus;

    #[test]
    fn event_envelope_accepts_zero_sequence() {
        let envelope = EventEnvelope {
            seq: 0,
            ts_ms: 0,
            event: AppEvent::PersistenceChanged(PersistenceStatus::Closed),
        };

        assert_eq!(envelope.seq, 0);
    }

    #[test]
    fn event_envelope_accepts_maximum_sequence() {
        let envelope = EventEnvelope {
            seq: u64::MAX,
            ts_ms: i64::MAX,
            event: AppEvent::PersistenceChanged(PersistenceStatus::Closed),
        };

        assert_eq!(envelope.seq, u64::MAX);
    }

    #[test]
    fn event_envelope_accepts_negative_timestamp() {
        let envelope = EventEnvelope {
            seq: 1,
            ts_ms: i64::MIN,
            event: AppEvent::PersistenceChanged(PersistenceStatus::Closed),
        };

        assert_eq!(envelope.ts_ms, i64::MIN);
    }
}
