use std::collections::BTreeMap;

/// Exact guarded worker wake prompt used by both browser and server delivery paths.
pub const WORKER_WAKE_NUDGE: &str = "You have new hive inbox message(s) — read your inbox, act on them now, and move handled ones to inbox/.done/. Act autonomously; only message god if you genuinely need a decision.";
pub const WORKER_WAKE_IDLE_MS: u64 = 12_000;
pub const WORKER_WAKE_BOOT_GRACE_MS: u64 = 35_000;
pub const WORKER_WAKE_COOLDOWN_MS: u64 = 60_000;
pub const WORKER_WAKE_HITL_REARM_MS: u64 = 5 * 60_000;

/// Live facts required for a server-side worker wake decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerWakeFacts<'a> {
    pub agent_id: &'a str,
    pub pty_id: Option<&'a str>,
    pub orchestrator: bool,
    pub last_output_at_ms: i64,
    pub inbox_count: usize,
    pub auto_delivery_paused: bool,
    pub paused: bool,
    pub halted: bool,
}

/// Stateful cooldown and HITL guard for background inbox wakeups.
#[derive(Default)]
pub struct WorkerWakeWatchdog {
    spawned_at: BTreeMap<String, i64>,
    last_nudge_at: BTreeMap<String, i64>,
    last_human_need_at: BTreeMap<String, i64>,
}

impl WorkerWakeWatchdog {
    /// Creates an empty watchdog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts boot grace for a newly spawned process.
    pub fn note_spawn(&mut self, pty_id: &str, at_ms: i64) {
        self.spawned_at.insert(String::from(pty_id), at_ms);
    }

    /// Re-arms the human-input guard after an approval or confirmation hook.
    pub fn note_human_need(&mut self, agent_id: &str, at_ms: i64) {
        self.last_human_need_at
            .insert(String::from(agent_id), at_ms);
    }

    /// Drops all per-agent cooldown state after process teardown.
    pub fn forget(&mut self, agent_id: &str, pty_id: Option<&str>) {
        self.last_nudge_at.remove(agent_id);
        self.last_human_need_at.remove(agent_id);
        if let Some(pty_id) = pty_id {
            self.spawned_at.remove(pty_id);
        }
    }

    /// Returns stable registry-order worker IDs that may receive a wake prompt now.
    pub fn decide(&mut self, facts: &[WorkerWakeFacts<'_>], now_ms: i64) -> Vec<String> {
        let mut wake = Vec::new();
        for fact in facts {
            let Some(pty_id) = fact.pty_id else {
                continue;
            };
            if fact.orchestrator
                || fact.inbox_count == 0
                || fact.auto_delivery_paused
                || fact.paused
                || fact.halted
                || fact.last_output_at_ms <= 0
                || elapsed_ms(now_ms, fact.last_output_at_ms) < WORKER_WAKE_IDLE_MS
                || self
                    .spawned_at
                    .get(pty_id)
                    .is_some_and(|spawned| elapsed_ms(now_ms, *spawned) < WORKER_WAKE_BOOT_GRACE_MS)
                || self
                    .last_human_need_at
                    .get(fact.agent_id)
                    .is_some_and(|last| elapsed_ms(now_ms, *last) < WORKER_WAKE_HITL_REARM_MS)
                || self
                    .last_nudge_at
                    .get(fact.agent_id)
                    .is_some_and(|last| elapsed_ms(now_ms, *last) < WORKER_WAKE_COOLDOWN_MS)
            {
                continue;
            }
            self.last_nudge_at
                .insert(String::from(fact.agent_id), now_ms);
            wake.push(String::from(fact.agent_id));
        }
        wake
    }
}

fn elapsed_ms(now_ms: i64, then_ms: i64) -> u64 {
    now_ms.saturating_sub(then_ms).try_into().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{WorkerWakeFacts, WorkerWakeWatchdog};

    fn facts() -> WorkerWakeFacts<'static> {
        WorkerWakeFacts {
            agent_id: "dev-1",
            pty_id: Some("pty-dev-1"),
            orchestrator: false,
            last_output_at_ms: 1,
            inbox_count: 1,
            auto_delivery_paused: false,
            paused: false,
            halted: false,
        }
    }

    #[test]
    fn new_watchdog_wakes_idle_worker() {
        let mut watchdog = WorkerWakeWatchdog::new();
        assert_eq!(
            watchdog.decide(&[facts()], 20_000),
            vec![String::from("dev-1")]
        );
    }

    #[test]
    fn boot_grace_blocks_wake() {
        let mut watchdog = WorkerWakeWatchdog::new();
        watchdog.note_spawn("pty-dev-1", 10_000);
        assert!(watchdog.decide(&[facts()], 20_000).is_empty());
    }

    #[test]
    fn human_need_blocks_wake() {
        let mut watchdog = WorkerWakeWatchdog::new();
        watchdog.note_human_need("dev-1", 10_000);
        assert!(watchdog.decide(&[facts()], 20_000).is_empty());
    }

    #[test]
    fn forget_clears_cooldown() {
        let mut watchdog = WorkerWakeWatchdog::new();
        assert_eq!(watchdog.decide(&[facts()], 20_000).len(), 1);
        watchdog.forget("dev-1", Some("pty-dev-1"));
        assert_eq!(watchdog.decide(&[facts()], 20_001).len(), 1);
    }
}
