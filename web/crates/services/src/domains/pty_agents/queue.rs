use std::collections::{BTreeMap, VecDeque};

use md_web_contracts::domains::pty_agents::{
    AgentStatus, QueuedTerminalMessage, TerminalActivityStatus, TerminalPresence, TerminalReadiness,
};

use super::error::PtyServiceError;

const MAX_SEND_ATTEMPTS: u8 = 3;
const QUIESCE_IDLE_MS: u64 = 12_000;
pub const DELIVERY_BOOT_GRACE_MS: u64 = 35_000;
pub const DELIVERY_QUIET_MS: u64 = 4_500;
pub const DELIVERY_COOLDOWN_MS: u64 = 4_500;

pub fn evaluate_terminal_readiness(
    has_initial_output: bool,
    boot_elapsed_ms: u64,
    quiet_elapsed_ms: u64,
    cooldown_elapsed_ms: u64,
    presence: TerminalPresence,
) -> TerminalReadiness {
    let boot_grace_remaining_ms = DELIVERY_BOOT_GRACE_MS.saturating_sub(boot_elapsed_ms);
    let quiet_remaining_ms = DELIVERY_QUIET_MS.saturating_sub(quiet_elapsed_ms);
    let cooldown_remaining_ms = DELIVERY_COOLDOWN_MS.saturating_sub(cooldown_elapsed_ms);
    let status = if presence.blocks_automation() {
        TerminalActivityStatus::UserOwned
    } else if boot_grace_remaining_ms > 0 || !has_initial_output {
        TerminalActivityStatus::Booting
    } else if quiet_remaining_ms > 0 || cooldown_remaining_ms > 0 {
        TerminalActivityStatus::Busy
    } else {
        TerminalActivityStatus::Ready
    };
    TerminalReadiness {
        has_initial_output,
        boot_grace_remaining_ms,
        quiet_remaining_ms,
        cooldown_remaining_ms,
        presence,
        status,
    }
}
const MAX_MESSAGE_BYTES: usize = 256 * 1024;

/// Current facts used to decide whether automation may own an agent's prompt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeliveryGate {
    pub status: AgentStatus,
    pub quiet_ms: Option<u64>,
    pub has_initial_output: bool,
    pub presence: TerminalPresence,
    pub automation_safe: bool,
    pub auto_delivery_paused: bool,
    pub boot_grace_remaining_ms: u64,
    pub cooldown_remaining_ms: u64,
    pub inbox_nonempty: Option<bool>,
}

/// Side-effect-free decision for the front of one agent queue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryDecision {
    Empty,
    Wait,
    Drop { message_id: String },
    Send { message_id: String, text: String },
}

/// Per-agent FIFO with bounded retry and delivery-time precondition semantics.
#[derive(Default)]
pub struct TerminalQueue {
    queues: BTreeMap<String, VecDeque<QueuedTerminalMessage>>,
}

impl TerminalQueue {
    /// Creates an empty queue set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one message, rejecting empty identities and payloads.
    pub fn enqueue(&mut self, message: QueuedTerminalMessage) -> Result<(), PtyServiceError> {
        if message.id.trim().is_empty()
            || message.agent_id.trim().is_empty()
            || message.text.trim().is_empty()
            || message.text.len() > MAX_MESSAGE_BYTES
            || message
                .instruction
                .as_deref()
                .is_some_and(|instruction| instruction.len() > MAX_MESSAGE_BYTES)
        {
            return Err(PtyServiceError::InvalidRequest(
                "送信先と上限内のメッセージを指定してください。",
            ));
        }
        self.queues
            .entry(message.agent_id.clone())
            .or_default()
            .push_back(message);
        Ok(())
    }

    /// Decides the current head without mutating it.
    pub fn decide(&self, agent_id: &str, gate: DeliveryGate) -> DeliveryDecision {
        let Some(message) = self.queues.get(agent_id).and_then(|queue| queue.front()) else {
            return DeliveryDecision::Empty;
        };
        let idle = gate.status == AgentStatus::Idle;
        let constrained_idle = gate.status == AgentStatus::Looping
            && gate.quiet_ms.is_some_and(|quiet| quiet >= QUIESCE_IDLE_MS);
        if (!idle && !constrained_idle)
            || !gate.has_initial_output
            || gate.quiet_ms.is_none_or(|quiet| quiet < DELIVERY_QUIET_MS)
            || gate.presence.blocks_automation()
            || !gate.automation_safe
            || gate.boot_grace_remaining_ms > 0
            || gate.cooldown_remaining_ms > 0
            || (gate.auto_delivery_paused && !message.manual)
        {
            return DeliveryDecision::Wait;
        }
        if message.precondition.is_some() && gate.inbox_nonempty == Some(false) {
            return DeliveryDecision::Drop {
                message_id: message.id.clone(),
            };
        }
        DeliveryDecision::Send {
            message_id: message.id.clone(),
            text: message
                .instruction
                .clone()
                .unwrap_or_else(|| message.text.clone()),
        }
    }

    /// Removes the head only when the browser acknowledges the same message id.
    pub fn acknowledge(&mut self, agent_id: &str, message_id: &str) -> bool {
        let Some(queue) = self.queues.get_mut(agent_id) else {
            return false;
        };
        if queue.front().is_none_or(|message| message.id != message_id) {
            return false;
        }
        queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(agent_id);
        }
        true
    }

    /// Records a failed two-write delivery. The third failure drops the head loudly at the caller.
    pub fn record_failure(
        &mut self,
        agent_id: &str,
        message_id: &str,
    ) -> Result<bool, PtyServiceError> {
        let queue = self
            .queues
            .get_mut(agent_id)
            .ok_or(PtyServiceError::NotFound)?;
        let message = queue.front_mut().ok_or(PtyServiceError::NotFound)?;
        if message.id != message_id {
            return Err(PtyServiceError::Conflict);
        }
        message.failed_attempts = message.failed_attempts.saturating_add(1);
        if message.failed_attempts < MAX_SEND_ATTEMPTS {
            return Ok(false);
        }
        queue.pop_front();
        if queue.is_empty() {
            self.queues.remove(agent_id);
        }
        Ok(true)
    }

    /// Clears all queued input for one archived agent.
    pub fn clear_agent(&mut self, agent_id: &str) -> usize {
        self.queues.remove(agent_id).map_or(0, |queue| queue.len())
    }

    /// Returns the number of pending messages for one agent.
    pub fn len(&self, agent_id: &str) -> usize {
        self.queues.get(agent_id).map_or(0, VecDeque::len)
    }

    /// Returns a selected nonempty head without consuming it or any Hive steer.
    pub fn selected_head_id(&self, agent_id: &str) -> Option<&str> {
        self.queues
            .get(agent_id)
            .and_then(|queue| queue.front())
            .map(|message| message.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{
        AgentStatus, DeliveryPrecondition, QueuedTerminalMessage, TerminalActivityStatus,
        TerminalPresence,
    };

    use super::{
        DELIVERY_BOOT_GRACE_MS, DELIVERY_COOLDOWN_MS, DELIVERY_QUIET_MS, DeliveryDecision,
        DeliveryGate, TerminalQueue, evaluate_terminal_readiness,
    };

    fn message() -> QueuedTerminalMessage {
        QueuedTerminalMessage {
            id: String::from("q-1"),
            agent_id: String::from("dev-1"),
            text: String::from("Read the inbox"),
            instruction: None,
            queued_at_ms: 0,
            manual: false,
            precondition: None,
            failed_attempts: 0,
        }
    }

    fn gate() -> DeliveryGate {
        DeliveryGate {
            status: AgentStatus::Idle,
            quiet_ms: Some(20_000),
            has_initial_output: true,
            presence: TerminalPresence::default(),
            automation_safe: true,
            auto_delivery_paused: false,
            boot_grace_remaining_ms: 0,
            cooldown_remaining_ms: 0,
            inbox_nonempty: Some(true),
        }
    }

    #[test]
    fn new_queue_is_empty() {
        assert_eq!(TerminalQueue::new().len("dev-1"), 0);
    }

    #[test]
    fn readiness_uses_exact_boot_quiet_and_cooldown_boundaries() {
        let booting = evaluate_terminal_readiness(
            true,
            DELIVERY_BOOT_GRACE_MS - 1,
            DELIVERY_QUIET_MS,
            DELIVERY_COOLDOWN_MS,
            TerminalPresence::default(),
        );
        assert_eq!(booting.boot_grace_remaining_ms, 1);
        let busy = evaluate_terminal_readiness(
            true,
            DELIVERY_BOOT_GRACE_MS,
            DELIVERY_QUIET_MS - 1,
            DELIVERY_COOLDOWN_MS - 1,
            TerminalPresence::default(),
        );
        assert_eq!(busy.status, TerminalActivityStatus::Busy);
        let ready = evaluate_terminal_readiness(
            true,
            DELIVERY_BOOT_GRACE_MS,
            DELIVERY_QUIET_MS,
            DELIVERY_COOLDOWN_MS,
            TerminalPresence::default(),
        );
        assert_eq!(ready.status, TerminalActivityStatus::Ready);
    }

    #[test]
    fn steer_selection_requires_nonempty_queue_head() {
        let mut queue = TerminalQueue::new();
        assert!(queue.selected_head_id("dev-1").is_none());
        assert!(queue.enqueue(message()).is_ok());
        assert_eq!(queue.selected_head_id("dev-1"), Some("q-1"));
    }

    #[test]
    fn enqueue_rejects_empty_text() {
        let mut queue = TerminalQueue::new();
        let mut empty = message();
        empty.text.clear();
        assert!(queue.enqueue(empty).is_err());
    }

    #[test]
    fn idle_agent_can_receive_front_message() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        assert!(matches!(
            queue.decide("dev-1", gate()),
            DeliveryDecision::Send { .. }
        ));
    }

    #[test]
    fn user_draft_blocks_delivery() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        let mut facts = gate();
        facts.automation_safe = false;
        assert_eq!(queue.decide("dev-1", facts), DeliveryDecision::Wait);
    }

    #[test]
    fn initial_output_and_real_quiet_window_are_required() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        let mut facts = gate();
        facts.has_initial_output = false;
        assert_eq!(queue.decide("dev-1", facts), DeliveryDecision::Wait);
        facts.has_initial_output = true;
        facts.quiet_ms = Some(4_499);
        assert_eq!(queue.decide("dev-1", facts), DeliveryDecision::Wait);
        facts.quiet_ms = Some(4_500);
        assert!(matches!(
            queue.decide("dev-1", facts),
            DeliveryDecision::Send { .. }
        ));
    }

    #[test]
    fn draft_picker_and_composition_each_block_delivery() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        for presence in [
            TerminalPresence {
                draft_nonempty: true,
                ..TerminalPresence::default()
            },
            TerminalPresence {
                picker_open: true,
                ..TerminalPresence::default()
            },
            TerminalPresence {
                composing: true,
                ..TerminalPresence::default()
            },
        ] {
            let mut facts = gate();
            facts.presence = presence;
            assert_eq!(queue.decide("dev-1", facts), DeliveryDecision::Wait);
        }
    }

    #[test]
    fn boot_grace_and_cooldown_block_delivery() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        let mut booting = gate();
        booting.boot_grace_remaining_ms = 1;
        assert_eq!(queue.decide("dev-1", booting), DeliveryDecision::Wait);
        let mut cooling_down = gate();
        cooling_down.cooldown_remaining_ms = 1;
        assert_eq!(queue.decide("dev-1", cooling_down), DeliveryDecision::Wait);
    }

    #[test]
    fn working_agent_and_quiescence_shortfall_block_delivery() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        let mut working = gate();
        working.status = AgentStatus::Working;
        assert_eq!(queue.decide("dev-1", working), DeliveryDecision::Wait);
        let mut looping = gate();
        looping.status = AgentStatus::Looping;
        looping.quiet_ms = Some(11_999);
        assert_eq!(queue.decide("dev-1", looping), DeliveryDecision::Wait);
        looping.quiet_ms = Some(12_000);
        assert!(matches!(
            queue.decide("dev-1", looping),
            DeliveryDecision::Send { .. }
        ));
    }

    #[test]
    fn paused_automation_blocks_only_non_manual_messages() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        let mut paused = gate();
        paused.auto_delivery_paused = true;
        assert_eq!(queue.decide("dev-1", paused), DeliveryDecision::Wait);

        let mut manual_queue = TerminalQueue::new();
        let mut manual = message();
        manual.manual = true;
        assert!(manual_queue.enqueue(manual).is_ok());
        assert!(matches!(
            manual_queue.decide("dev-1", paused),
            DeliveryDecision::Send { .. }
        ));
    }

    #[test]
    fn stale_inbox_nudge_is_dropped() {
        let mut queue = TerminalQueue::new();
        let mut nudge = message();
        nudge.precondition = Some(DeliveryPrecondition::InboxNonempty);
        assert!(queue.enqueue(nudge).is_ok());
        let mut facts = gate();
        facts.inbox_nonempty = Some(false);
        assert!(matches!(
            queue.decide("dev-1", facts),
            DeliveryDecision::Drop { .. }
        ));
    }

    #[test]
    fn acknowledge_requires_front_message_id() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        assert!(!queue.acknowledge("dev-1", "q-other"));
        assert_eq!(queue.len("dev-1"), 1);
    }

    #[test]
    fn fifo_preserves_message_order_across_acknowledgements() {
        let mut queue = TerminalQueue::new();
        let first = message();
        let mut second = message();
        second.id = String::from("q-2");
        second.text = String::from("Second instruction");
        assert!(queue.enqueue(first).is_ok());
        assert!(queue.enqueue(second).is_ok());
        assert!(matches!(
            queue.decide("dev-1", gate()),
            DeliveryDecision::Send { message_id, .. } if message_id == "q-1"
        ));
        assert!(queue.acknowledge("dev-1", "q-1"));
        assert!(matches!(
            queue.decide("dev-1", gate()),
            DeliveryDecision::Send { message_id, text } if message_id == "q-2" && text == "Second instruction"
        ));
    }

    #[test]
    fn third_failure_drops_message() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        assert!(matches!(queue.record_failure("dev-1", "q-1"), Ok(false)));
        assert!(matches!(queue.record_failure("dev-1", "q-1"), Ok(false)));
        assert!(matches!(queue.record_failure("dev-1", "q-1"), Ok(true)));
    }

    #[test]
    fn clear_agent_reports_removed_count() {
        let mut queue = TerminalQueue::new();
        assert!(queue.enqueue(message()).is_ok());
        assert_eq!(queue.clear_agent("dev-1"), 1);
    }
}
