use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use md_web_contracts::{HiveDomainEvent, HiveEventEnvelope};

/// Failure to access the in-process replay buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventHubError {
    InvalidCapacity,
    SequenceExhausted,
    LockPoisoned,
}

/// Replay response. `gap` means the requested cursor predates retained history.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayBatch {
    pub gap: bool,
    pub events: Vec<HiveEventEnvelope>,
}

/// Bounded ordered event history shared by SSE/WebSocket adapters.
pub struct EventHub {
    capacity: usize,
    next_sequence: AtomicU64,
    events: Mutex<VecDeque<HiveEventEnvelope>>,
}

impl EventHub {
    /// Creates a replay buffer with a fixed positive capacity.
    pub fn new(capacity: usize) -> Result<Self, EventHubError> {
        if capacity == 0 {
            return Err(EventHubError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            next_sequence: AtomicU64::new(1),
            events: Mutex::new(VecDeque::with_capacity(capacity)),
        })
    }

    /// Appends one event and evicts the oldest retained item when full.
    pub fn publish(
        &self,
        ts_ms: i64,
        event: HiveDomainEvent,
    ) -> Result<HiveEventEnvelope, EventHubError> {
        let seq = self
            .next_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| EventHubError::SequenceExhausted)?;
        let envelope = HiveEventEnvelope { seq, ts_ms, event };
        let mut events = self
            .events
            .lock()
            .map_err(|_| EventHubError::LockPoisoned)?;
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(envelope.clone());
        Ok(envelope)
    }

    /// Returns retained events newer than `after`, preserving publish order.
    pub fn replay_after(&self, after: u64) -> Result<ReplayBatch, EventHubError> {
        let events = self
            .events
            .lock()
            .map_err(|_| EventHubError::LockPoisoned)?;
        let gap = events
            .front()
            .is_some_and(|oldest| after.saturating_add(1) < oldest.seq);
        let replay = events
            .iter()
            .filter(|item| item.seq > after)
            .cloned()
            .collect();
        Ok(ReplayBatch {
            gap,
            events: replay,
        })
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::HiveDomainEvent;

    use super::{EventHub, EventHubError};

    fn deleted(id: &str) -> HiveDomainEvent {
        HiveDomainEvent::TaskDeleted {
            task_id: String::from(id),
        }
    }

    #[test]
    fn rejects_zero_capacity() {
        assert!(matches!(
            EventHub::new(0),
            Err(EventHubError::InvalidCapacity)
        ));
    }

    #[test]
    fn publish_assigns_monotonic_sequence() -> Result<(), EventHubError> {
        let hub = EventHub::new(2)?;
        let first = hub.publish(1, deleted("a"))?;
        let second = hub.publish(2, deleted("b"))?;

        assert_eq!((first.seq, second.seq), (1, 2));
        Ok(())
    }

    #[test]
    fn replay_reports_eviction_gap() -> Result<(), EventHubError> {
        let hub = EventHub::new(1)?;
        hub.publish(1, deleted("a"))?;
        hub.publish(2, deleted("b"))?;
        let replay = hub.replay_after(0)?;

        assert!(replay.gap);
        Ok(())
    }
}
