use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

use md_web_contracts::AgentControlSnapshot;

const MAX_PENDING_STEERS: usize = 20;
const MAX_STEER_BYTES: usize = 10_000;

/// Failure to access control state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlError {
    LockPoisoned,
}

#[derive(Default)]
struct AgentControl {
    paused: bool,
    halted: bool,
    auto_delivery_paused: bool,
    gated_tools: BTreeSet<String>,
    steer_queue: VecDeque<String>,
}

/// Process-local controls applied by the agent hook adapter.
#[derive(Default)]
pub struct ControlRegistry {
    controls: Mutex<BTreeMap<String, AgentControl>>,
}

impl ControlRegistry {
    /// Pauses or unpauses all tool use for one agent.
    pub fn pause(&self, agent_id: &str, on: bool) -> Result<AgentControlSnapshot, ControlError> {
        self.update(agent_id, |control| control.paused = on)
    }

    /// Holds or resumes automatic queued-message delivery.
    pub fn pause_auto_delivery(
        &self,
        agent_id: &str,
        on: bool,
    ) -> Result<AgentControlSnapshot, ControlError> {
        self.update(agent_id, |control| control.auto_delivery_paused = on)
    }

    /// Enables or disables a named tool gate.
    pub fn gate_tool(
        &self,
        agent_id: &str,
        tool: &str,
        on: bool,
    ) -> Result<AgentControlSnapshot, ControlError> {
        self.update(agent_id, |control| {
            if on {
                control.gated_tools.insert(String::from(tool));
            } else {
                control.gated_tools.remove(tool);
            }
        })
    }

    /// Queues guidance for the next hook boundary, dropping the oldest at capacity.
    pub fn steer(&self, agent_id: &str, text: &str) -> Result<AgentControlSnapshot, ControlError> {
        let trimmed = text.trim();
        self.update(agent_id, |control| {
            if trimmed.is_empty() {
                return;
            }
            if control.steer_queue.len() == MAX_PENDING_STEERS {
                control.steer_queue.pop_front();
            }
            let boundary = trimmed
                .char_indices()
                .map(|(index, _)| index)
                .take_while(|index| *index <= MAX_STEER_BYTES)
                .last()
                .unwrap_or(0);
            let end = if trimmed.len() <= MAX_STEER_BYTES {
                trimmed.len()
            } else {
                boundary
            };
            control.steer_queue.push_back(String::from(&trimmed[..end]));
        })
    }

    /// Requests a clean stop at the next hook boundary.
    pub fn halt(&self, agent_id: &str) -> Result<AgentControlSnapshot, ControlError> {
        self.update(agent_id, |control| control.halted = true)
    }

    /// Clears pause and halt while keeping explicit tool gates.
    pub fn resume(&self, agent_id: &str) -> Result<AgentControlSnapshot, ControlError> {
        self.update(agent_id, |control| {
            control.paused = false;
            control.halted = false;
        })
    }

    /// Reads the latest controls for one agent.
    pub fn snapshot(&self, agent_id: &str) -> Result<AgentControlSnapshot, ControlError> {
        let mut controls = self
            .controls
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?;
        let control = controls.entry(String::from(agent_id)).or_default();
        Ok(snapshot(control))
    }

    /// Removes and returns the oldest pending steer note.
    pub fn take_steer(&self, agent_id: &str) -> Result<Option<String>, ControlError> {
        let mut controls = self
            .controls
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?;
        Ok(controls
            .get_mut(agent_id)
            .and_then(|control| control.steer_queue.pop_front()))
    }

    fn update(
        &self,
        agent_id: &str,
        operation: impl FnOnce(&mut AgentControl),
    ) -> Result<AgentControlSnapshot, ControlError> {
        let mut controls = self
            .controls
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?;
        let control = controls.entry(String::from(agent_id)).or_default();
        operation(control);
        Ok(snapshot(control))
    }
}

fn snapshot(control: &AgentControl) -> AgentControlSnapshot {
    AgentControlSnapshot {
        paused: control.paused,
        halted: control.halted,
        auto_delivery_paused: control.auto_delivery_paused,
        gated_tools: control.gated_tools.iter().cloned().collect(),
        pending_steers: control.steer_queue.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlError, ControlRegistry, MAX_PENDING_STEERS};

    #[test]
    fn pause_changes_snapshot() -> Result<(), ControlError> {
        let registry = ControlRegistry::default();

        assert!(registry.pause("agent", true)?.paused);
        Ok(())
    }

    #[test]
    fn resume_keeps_tool_gates() -> Result<(), ControlError> {
        let registry = ControlRegistry::default();
        registry.pause("agent", true)?;
        registry.gate_tool("agent", "Bash", true)?;

        assert_eq!(
            registry.resume("agent")?.gated_tools,
            [String::from("Bash")]
        );
        Ok(())
    }

    #[test]
    fn steer_queue_drops_oldest_at_capacity() -> Result<(), ControlError> {
        let registry = ControlRegistry::default();
        for index in 0..=MAX_PENDING_STEERS {
            registry.steer("agent", &index.to_string())?;
        }

        assert_eq!(registry.take_steer("agent")?, Some(String::from("1")));
        Ok(())
    }

    #[test]
    fn empty_steer_is_ignored() -> Result<(), ControlError> {
        let registry = ControlRegistry::default();

        assert_eq!(registry.steer("agent", "   ")?.pending_steers, 0);
        Ok(())
    }
}
