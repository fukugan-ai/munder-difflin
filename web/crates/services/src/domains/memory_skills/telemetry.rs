use std::collections::{BTreeMap, VecDeque};
use std::sync::RwLock;

use md_web_contracts::domains::memory_skills::{
    AgentUsageSample, ProviderUsageEvent, TelemetrySnapshot, ToolSpan, ToolWaterfall,
    ToolWaterfallRow,
};

const DEFAULT_RING_CAPACITY: usize = 256;

#[derive(Default)]
struct TelemetryState {
    usage: BTreeMap<String, AgentUsageSample>,
    spans: BTreeMap<String, VecDeque<ToolSpan>>,
}

pub struct TelemetryStore {
    ring_capacity: usize,
    state: RwLock<TelemetryState>,
}

impl TelemetryStore {
    pub fn new(ring_capacity: usize) -> Self {
        Self {
            ring_capacity: ring_capacity.max(1),
            state: RwLock::new(TelemetryState::default()),
        }
    }

    pub fn record_usage(&self, sample: AgentUsageSample) {
        if !sample.usd.is_finite() || sample.usd < 0.0 || sample.session_id.is_empty() {
            return;
        }
        if let Ok(mut state) = self.state.write() {
            let replace = state
                .usage
                .get(&sample.agent_id)
                .is_none_or(|current| sample.timestamp_ms >= current.timestamp_ms);
            if replace {
                state.usage.insert(sample.agent_id.clone(), sample);
            }
        }
    }

    pub fn record_span(&self, span: ToolSpan) {
        if span.agent_id.is_empty() || span.session_id.is_empty() || span.tool.is_empty() {
            return;
        }
        if let Ok(mut state) = self.state.write() {
            let ring = state.spans.entry(span.agent_id.clone()).or_default();
            if ring.len() == self.ring_capacity {
                ring.pop_front();
            }
            ring.push_back(span);
        }
    }

    /// Applies post-ledger projections only for a newly inserted provider event.
    /// The PostgreSQL `(namespace,event_id)` conflict result is the idempotency
    /// authority supplied by the caller.
    pub fn record_provider_event(&self, event: &ProviderUsageEvent, inserted: bool) -> usize {
        if !inserted {
            return 0;
        }
        self.record_usage(event.usage.clone());
        for span in &event.tool_spans {
            self.record_span(span.clone());
        }
        event.tool_spans.len()
    }

    pub fn spans(&self, agent_id: &str) -> Vec<ToolSpan> {
        self.state
            .read()
            .ok()
            .and_then(|state| {
                state
                    .spans
                    .get(agent_id)
                    .map(|ring| ring.iter().cloned().collect())
            })
            .unwrap_or_default()
    }

    pub fn snapshot(&self) -> TelemetrySnapshot {
        let Ok(state) = self.state.read() else {
            return TelemetrySnapshot {
                usage: Vec::new(),
                spans: BTreeMap::new(),
            };
        };
        TelemetrySnapshot {
            usage: state.usage.values().cloned().collect(),
            spans: state
                .spans
                .iter()
                .map(|(agent, ring)| (agent.clone(), ring.iter().cloned().collect()))
                .collect(),
        }
    }

    pub fn clear(&self) {
        if let Ok(mut state) = self.state.write() {
            *state = TelemetryState::default();
        }
    }

    pub fn waterfall(&self, agent_id: Option<&str>) -> ToolWaterfall {
        let snapshot = self.snapshot();
        let mut spans: Vec<ToolSpan> = snapshot
            .spans
            .into_iter()
            .filter(|(agent, _)| agent_id.is_none_or(|selected| selected == agent))
            .flat_map(|(_, spans)| spans)
            .collect();
        spans.sort_by_key(|span| span.timestamp_ms);
        let origin_ms = spans.first().map_or(0, |span| span.timestamp_ms);
        let rows: Vec<ToolWaterfallRow> = spans
            .into_iter()
            .map(|span| ToolWaterfallRow {
                agent_id: span.agent_id,
                tool: span.tool,
                offset_ms: u64::try_from(span.timestamp_ms.saturating_sub(origin_ms)).unwrap_or(0),
                duration_ms: span.duration_ms,
                success: span.success,
            })
            .collect();
        let duration_ms = rows
            .iter()
            .map(|row| row.offset_ms.saturating_add(row.duration_ms))
            .max()
            .unwrap_or(0);
        ToolWaterfall {
            origin_ms,
            duration_ms,
            rows,
        }
    }
}

impl Default for TelemetryStore {
    fn default() -> Self {
        Self::new(DEFAULT_RING_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::memory_skills::{
        AgentUsageSample, ProviderUsageEvent, ProviderUsageKind, ToolSpan, UsageCounterMode,
    };

    use super::TelemetryStore;

    fn span(timestamp_ms: i64) -> ToolSpan {
        ToolSpan {
            agent_id: String::from("a"),
            session_id: String::from("s"),
            timestamp_ms,
            tool: String::from("Read"),
            success: true,
            duration_ms: 1,
            decision: None,
            error: None,
        }
    }

    #[test]
    fn span_ring_discards_oldest_item() {
        let store = TelemetryStore::new(2);
        store.record_span(span(1));
        store.record_span(span(2));
        store.record_span(span(3));

        assert_eq!(
            store.spans("a").first().map(|item| item.timestamp_ms),
            Some(2)
        );
    }

    #[test]
    fn invalid_usage_cost_is_rejected() {
        let store = TelemetryStore::default();
        store.record_usage(AgentUsageSample {
            agent_id: String::from("a"),
            session_id: String::from("s"),
            timestamp_ms: 1,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            model: String::new(),
            usd: f64::NAN,
        });

        assert!(store.snapshot().usage.is_empty());
    }

    #[test]
    fn waterfall_preserves_real_offsets() {
        let store = TelemetryStore::default();
        store.record_span(span(100));
        store.record_span(span(140));
        let waterfall = store.waterfall(Some("a"));
        assert_eq!(waterfall.rows[1].offset_ms, 40);
    }

    #[test]
    fn clear_removes_usage_and_spans() {
        let store = TelemetryStore::default();
        store.record_span(span(100));
        store.record_usage(AgentUsageSample {
            agent_id: String::from("a"),
            session_id: String::from("s"),
            timestamp_ms: 100,
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_creation_tokens: 4,
            model: String::from("model"),
            usd: 0.01,
        });

        store.clear();

        assert!(store.snapshot().usage.is_empty());
        assert!(store.spans("a").is_empty());
    }

    #[test]
    fn duplicate_provider_event_does_not_duplicate_tool_span() {
        let store = TelemetryStore::default();
        let event = ProviderUsageEvent {
            event_id: String::from("00000000-0000-5000-8000-000000000001"),
            provider: ProviderUsageKind::Codex,
            counter_mode: UsageCounterMode::Delta,
            usage: AgentUsageSample {
                agent_id: String::from("a"),
                session_id: String::from("s"),
                timestamp_ms: 1,
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                model: String::from("gpt"),
                usd: 0.01,
            },
            context_window_tokens: Some(100),
            tool_spans: vec![span(1)],
        };

        assert_eq!(store.record_provider_event(&event, true), 1);
        assert_eq!(store.record_provider_event(&event, false), 0);
        assert_eq!(store.spans("a").len(), 1);
        assert_eq!(store.snapshot().usage[0].usd, 0.01);
    }
}
