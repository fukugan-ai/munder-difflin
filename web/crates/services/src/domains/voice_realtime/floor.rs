#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use md_web_contracts::domains::voice_realtime::FloorDelta;

const MIN_PUSH_GAP_MS: i64 = 12_000;
const ACTIVE_WINDOW_MS: i64 = 8_000;
const MAX_PUSH_CHARS: usize = 600;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FloorAgent {
    pub id: String,
    pub name: String,
    pub archived: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FloorTask {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FloorPty {
    pub agent_id: String,
    pub last_output_at_ms: i64,
}

#[derive(Clone, Debug, Default)]
pub struct FloorObserver {
    live: bool,
    primed: bool,
    last_push_at_ms: i64,
    agents: BTreeMap<String, (String, bool)>,
    tasks: BTreeMap<String, (String, String)>,
    active: BTreeMap<String, bool>,
    buffer: Vec<String>,
}

impl FloorObserver {
    pub fn set_session_live(&mut self, live: bool) {
        self.live = live;
        self.primed = false;
        self.buffer.clear();
    }

    pub fn observe(
        &mut self,
        agents: &[FloorAgent],
        tasks: &[FloorTask],
        ptys: &[FloorPty],
        now_ms: i64,
    ) -> Option<FloorDelta> {
        let next_agents = agents
            .iter()
            .map(|agent| (agent.id.clone(), (agent.name.clone(), agent.archived)))
            .collect::<BTreeMap<_, _>>();
        let next_tasks = tasks
            .iter()
            .map(|task| (task.id.clone(), (task.title.clone(), task.status.clone())))
            .collect::<BTreeMap<_, _>>();
        let next_active = ptys
            .iter()
            .map(|pty| {
                (
                    pty.agent_id.clone(),
                    now_ms.saturating_sub(pty.last_output_at_ms) < ACTIVE_WINDOW_MS,
                )
            })
            .collect::<BTreeMap<_, _>>();

        if self.primed && self.live {
            self.diff_agents(&next_agents);
            self.diff_tasks(&next_tasks);
            self.diff_activity(&next_active, &next_agents);
        }
        self.agents = next_agents;
        self.tasks = next_tasks;
        self.active = next_active;
        self.primed = true;

        if !self.live || self.buffer.is_empty() {
            if !self.live {
                self.buffer.clear();
            }
            return None;
        }
        if now_ms.saturating_sub(self.last_push_at_ms) < MIN_PUSH_GAP_MS {
            return None;
        }
        let text = self
            .buffer
            .join("; ")
            .chars()
            .take(MAX_PUSH_CHARS)
            .collect();
        self.buffer.clear();
        self.last_push_at_ms = now_ms;
        Some(FloorDelta {
            text,
            observed_at_ms: now_ms,
        })
    }

    fn diff_agents(&mut self, next: &BTreeMap<String, (String, bool)>) {
        for (id, (name, archived)) in next {
            match self.agents.get(id) {
                None => self.buffer.push(format!("{name} がフロアに参加")),
                Some((_, false)) if *archived => self.buffer.push(format!("{name} をアーカイブ")),
                Some((_, true)) if !archived => self.buffer.push(format!("{name} が復帰")),
                _ => {}
            }
        }
    }

    fn diff_tasks(&mut self, next: &BTreeMap<String, (String, String)>) {
        for (id, (title, status)) in next {
            match self.tasks.get(id) {
                None => self.buffer.push(format!("新しいタスク「{title}」")),
                Some((_, previous)) if previous != status => {
                    self.buffer
                        .push(format!("タスク「{title}」が {status} へ移動"));
                }
                _ => {}
            }
        }
    }

    fn diff_activity(
        &mut self,
        next: &BTreeMap<String, bool>,
        agents: &BTreeMap<String, (String, bool)>,
    ) {
        for (id, active) in next {
            let Some(previous) = self.active.get(id) else {
                continue;
            };
            let Some((name, archived)) = agents.get(id) else {
                continue;
            };
            if *archived || previous == active {
                continue;
            }
            self.buffer.push(if *active {
                format!("{name} が出力を開始")
            } else {
                format!("{name} が待機状態")
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FloorAgent, FloorObserver, FloorPty, FloorTask};

    fn agent(archived: bool) -> FloorAgent {
        FloorAgent {
            id: String::from("worker"),
            name: String::from("Worker"),
            archived,
        }
    }

    #[test]
    fn first_observation_only_primes() {
        let mut observer = FloorObserver::default();
        observer.set_session_live(true);

        assert!(
            observer
                .observe(&[agent(false)], &[], &[], 20_000)
                .is_none()
        );
    }

    #[test]
    fn archive_change_emits_after_gap() {
        let mut observer = FloorObserver::default();
        observer.set_session_live(true);
        let _prime = observer.observe(&[agent(false)], &[], &[], 20_000);
        let delta = observer.observe(&[agent(true)], &[], &[], 32_000);

        assert!(delta.is_some_and(|event| event.text.contains("アーカイブ")));
    }

    #[test]
    fn closed_session_drops_deltas() {
        let mut observer = FloorObserver::default();
        observer.set_session_live(false);
        let delta = observer.observe(
            &[agent(false)],
            &[FloorTask {
                id: String::from("task"),
                title: String::from("作業"),
                status: String::from("done"),
            }],
            &[FloorPty {
                agent_id: String::from("worker"),
                last_output_at_ms: 0,
            }],
            20_000,
        );

        assert!(delta.is_none());
    }
}
