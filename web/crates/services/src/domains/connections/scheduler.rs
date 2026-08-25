use md_web_contracts::domains::connections::{
    ConnectionEvent, ContextAction, ContextRule, ScheduledMission, WeeklySchedule,
};

use super::DomainState;

const DAY_MS: u64 = 86_400_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextUsageSample {
    pub agent_id: String,
    pub context_pct: u8,
    pub large_window: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AutomationBatch {
    pub missions: Vec<ScheduledMission>,
    pub events: Vec<ConnectionEvent>,
}

pub(super) fn poll(
    state: &mut DomainState,
    now_ms: u64,
    context_usage: &[ContextUsageSample],
) -> AutomationBatch {
    let mut batch = AutomationBatch::default();
    let mut missions_changed = false;
    for mission in &mut state.missions {
        if !mission.enabled {
            continue;
        }
        let due = mission.weekly.as_ref().map_or_else(
            || interval_due(mission.last_fired_at_ms, mission.interval_ms, now_ms),
            |weekly| weekly_due(mission.last_fired_at_ms, weekly, now_ms),
        );
        if mission.last_fired_at_ms.is_none() {
            mission.last_fired_at_ms = Some(now_ms);
            missions_changed = true;
            continue;
        }
        if due {
            mission.last_fired_at_ms = Some(now_ms);
            missions_changed = true;
            batch.missions.push(mission.clone());
            batch
                .events
                .push(ConnectionEvent::MissionDue(mission.clone()));
        }
    }
    if missions_changed {
        batch.events.push(ConnectionEvent::MissionsUpdated);
    }
    poll_context(
        ContextAction::Compact,
        &state.context.compact,
        &mut state.context_last_fired[0],
        context_usage,
        now_ms,
        &mut batch,
    );
    poll_context(
        ContextAction::Clear,
        &state.context.clear,
        &mut state.context_last_fired[1],
        context_usage,
        now_ms,
        &mut batch,
    );
    batch
}

fn poll_context(
    action: ContextAction,
    rule: &ContextRule,
    last_fired: &mut Option<u64>,
    usage: &[ContextUsageSample],
    now_ms: u64,
    batch: &mut AutomationBatch,
) {
    if !rule.enabled {
        return;
    }
    let Some(last) = *last_fired else {
        *last_fired = Some(now_ms);
        return;
    };
    if now_ms.saturating_sub(last) < rule.every_ms {
        return;
    }
    let threshold_met = usage.iter().any(|sample| {
        let threshold = if sample.large_window {
            rule.min_context_pct_large_window
        } else {
            rule.min_context_pct
        };
        sample.context_pct >= threshold
    });
    if threshold_met {
        *last_fired = Some(now_ms);
        batch.events.push(ConnectionEvent::ContextTriggerDue {
            action,
            rule: rule.clone(),
        });
    }
}

fn interval_due(last_fired: Option<u64>, interval_ms: u64, now_ms: u64) -> bool {
    last_fired.is_some_and(|last| now_ms.saturating_sub(last) >= interval_ms)
}

fn weekly_due(last_fired: Option<u64>, schedule: &WeeklySchedule, now_ms: u64) -> bool {
    let unix_day = now_ms / DAY_MS;
    // 1970-01-01 was Thursday; contracts use Sunday=0.
    let day_of_week = ((unix_day + 4) % 7) as u8;
    if !schedule.days.contains(&day_of_week) {
        return false;
    }
    let minute = ((now_ms % DAY_MS) / 60_000) as u16;
    if minute < schedule.minute {
        return false;
    }
    let day_start = unix_day * DAY_MS;
    last_fired.is_some_and(|last| last < day_start + u64::from(schedule.minute) * 60_000)
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::connections::{MissionKind, ScheduledMission};

    use super::{ContextUsageSample, poll};
    use crate::domains::connections::ConnectionsService;

    #[test]
    fn interval_fires_once_and_stamps_before_next_poll() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        service.replace_missions(vec![ScheduledMission {
            id: String::from("hourly"),
            label: String::from("Hourly"),
            interval_ms: 60_000,
            weekly: None,
            to: String::from("god"),
            body: String::from("status"),
            enabled: true,
            last_fired_at_ms: Some(1_000),
            kind: MissionKind::Dispatch,
            quiet_threshold_ms: None,
        }])?;
        let first = service.poll_automations(61_000, &[])?;
        let duplicate = service.poll_automations(61_000, &[])?;
        assert_eq!(first.missions.len(), 1);
        assert!(duplicate.missions.is_empty());
        assert_eq!(
            service.snapshot()?.missions[0].last_fired_at_ms,
            Some(61_000)
        );
        Ok(())
    }

    #[test]
    fn context_requires_cadence_and_pressure() -> Result<(), Box<dyn std::error::Error>> {
        let service = ConnectionsService::new();
        let usage = [ContextUsageSample {
            agent_id: String::from("god"),
            context_pct: 90,
            large_window: false,
        }];
        assert!(service.poll_automations(1, &usage)?.events.is_empty());
        let due = service.poll_automations(7_200_001, &usage)?;
        assert_eq!(due.events.len(), 1);
        Ok(())
    }

    #[test]
    fn direct_poll_helper_is_deterministic() {
        let mut service = ConnectionsService::new();
        let state = service
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let first = poll(state, 1, &[]);
        let second = poll(state, 1, &[]);
        assert!(first.missions.is_empty() && second.missions.is_empty());
    }
}
