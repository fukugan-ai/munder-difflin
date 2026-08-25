use std::sync::Arc;

use md_web_contracts::{HiveDomainEvent, HiveMessage, HiveRegistry};

use super::{EventHub, EventHubError, HiveStore, HiveStoreError};

const HOP_CAP: u8 = 12;

/// Routing failure at the local coordination boundary.
#[derive(Debug)]
pub enum RouteError {
    HopCap,
    InvalidMessage(serde_json::Error),
    Store(HiveStoreError),
    Event(EventHubError),
}

/// Result of one route attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteOutcome {
    pub delivered: Vec<String>,
    pub unknown: Vec<String>,
}

/// Routes normalized hive messages into local agent inboxes.
pub struct HiveRouter {
    store: Arc<HiveStore>,
    events: Arc<EventHub>,
}

impl HiveRouter {
    /// Creates a router over the canonical store and event stream.
    #[must_use]
    pub fn new(store: Arc<HiveStore>, events: Arc<EventHub>) -> Self {
        Self { store, events }
    }

    /// Delivers one message and emits its resolved route to live clients.
    pub fn route(&self, message: &HiveMessage, ts_ms: i64) -> Result<RouteOutcome, RouteError> {
        if message.hops > HOP_CAP {
            return Err(RouteError::HopCap);
        }
        let registry = self.store.registry().map_err(RouteError::Store)?;
        let targets = resolve_targets(message, &registry);
        let value = serde_json::to_value(message).map_err(RouteError::InvalidMessage)?;
        let mut delivered = Vec::with_capacity(targets.len());
        let mut unknown = Vec::new();
        for target in targets {
            match self.store.deliver_message(&target, &message.id, &value) {
                Ok(()) => delivered.push(target),
                Err(HiveStoreError::TaskNotFound) => unknown.push(target),
                Err(error) => return Err(RouteError::Store(error)),
            }
        }
        self.events
            .publish(
                ts_ms,
                HiveDomainEvent::MessageRouted {
                    message: message.clone(),
                    targets: delivered.clone(),
                },
            )
            .map_err(RouteError::Event)?;
        Ok(RouteOutcome { delivered, unknown })
    }
}

fn resolve_targets(message: &HiveMessage, registry: &HiveRegistry) -> Vec<String> {
    let god_id = registry.god_id.as_deref().unwrap_or("god");
    if message.to == "broadcast" {
        return registry
            .agents
            .values()
            .filter(|agent| !agent.archived && agent.id != message.from)
            .map(|agent| agent.id.clone())
            .collect();
    }
    let target = if message.to == "human" || message.to == "god" {
        god_id
    } else {
        &message.to
    };
    if target == message.from {
        Vec::new()
    } else {
        vec![String::from(target)]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use md_web_contracts::{HiveAgent, HiveMessage, HiveRegistry, MessageAct};

    use super::resolve_targets;

    fn message(to: &str) -> HiveMessage {
        HiveMessage {
            id: String::from("m-1"),
            conversation: String::from("c-1"),
            in_reply_to: None,
            from: String::from("a"),
            to: String::from(to),
            act: MessageAct::Inform,
            subject: String::new(),
            body: String::new(),
            hops: 0,
            requires_reply: false,
            needs_human: false,
            created_at: String::from("2026-08-25T00:00:00Z"),
        }
    }

    fn agent(id: &str, archived: bool) -> HiveAgent {
        HiveAgent {
            id: String::from(id),
            name: String::from(id),
            status: String::from("idle"),
            role: String::new(),
            provider: String::from("codex"),
            archived,
            on_hold: false,
            inbox_backlog: 0,
        }
    }

    #[test]
    fn human_routes_to_god() {
        let registry = HiveRegistry {
            god_id: Some(String::from("michael")),
            agents: BTreeMap::new(),
        };

        assert_eq!(
            resolve_targets(&message("human"), &registry),
            [String::from("michael")]
        );
    }

    #[test]
    fn broadcast_excludes_sender_and_archived_agents() {
        let registry = HiveRegistry {
            god_id: None,
            agents: BTreeMap::from([
                (String::from("a"), agent("a", false)),
                (String::from("b"), agent("b", false)),
                (String::from("c"), agent("c", true)),
            ]),
        };

        assert_eq!(
            resolve_targets(&message("broadcast"), &registry),
            [String::from("b")]
        );
    }
}
