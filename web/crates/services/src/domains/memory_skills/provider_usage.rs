use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use md_web_contracts::domains::memory_skills::{
    AgentUsageSample, ProviderUsageEvent, ProviderUsageKind, ToolSpan, UsageCounterMode,
};

use super::DomainError;

const MAX_TRANSCRIPT_EVENT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TOOL_SPANS: usize = 128;
const MAX_DEDUP_EVENTS: usize = 4_096;

pub struct ProviderTranscriptEvent<'a> {
    pub provider: ProviderUsageKind,
    pub source_event_id: &'a str,
    pub agent_id: &'a str,
    pub session_id: &'a str,
    pub timestamp_ms: i64,
    pub payload_json: &'a str,
}

pub fn sanitize_provider_transcript(
    input: &ProviderTranscriptEvent<'_>,
) -> Result<ProviderUsageEvent, DomainError> {
    if input.payload_json.len() > MAX_TRANSCRIPT_EVENT_BYTES
        || input.agent_id.trim().is_empty()
        || input.session_id.trim().is_empty()
        || input.source_event_id.trim().is_empty()
    {
        return Err(DomainError::InvalidInput("invalid provider usage event"));
    }
    let payload: serde_json::Value = serde_json::from_str(input.payload_json)?;
    let usage_roots = usage_roots(input.provider, &payload);
    let sum_tokens = |keys: &[&str]| {
        usage_roots
            .iter()
            .map(|root| token(root, keys))
            .fold(0_u64, u64::saturating_add)
    };
    let input_tokens = sum_tokens(&["input_tokens", "inputTokens", "promptTokenCount"]);
    let output_tokens = sum_tokens(&["output_tokens", "outputTokens", "candidatesTokenCount"]);
    let cache_read_tokens = sum_tokens(&[
        "cache_read_tokens",
        "cacheReadInputTokens",
        "cached_input_tokens",
        "cachedContentTokenCount",
    ]);
    let cache_creation_tokens = sum_tokens(&["cache_creation_tokens", "cacheCreationInputTokens"]);
    let provider_costs: Vec<f64> = usage_roots
        .iter()
        .filter_map(|root| {
            number(
                root,
                &["cost_usd", "costUSD", "total_cost_usd", "totalCostUsd"],
            )
        })
        .collect();
    let usd = (!provider_costs.is_empty())
        .then(|| provider_costs.into_iter().sum())
        .or_else(|| {
            number(
                &payload,
                &["cost_usd", "costUSD", "total_cost_usd", "totalCostUsd"],
            )
        })
        .unwrap_or(0.0);
    if input_tokens == 0 && output_tokens == 0 && cache_read_tokens == 0 {
        return Err(DomainError::InvalidInput(
            "provider usage event has no token usage",
        ));
    }
    if !usd.is_finite() || usd < 0.0 {
        return Err(DomainError::InvalidInput("provider cost is invalid"));
    }
    let model = usage_roots
        .iter()
        .find_map(|root| string(root, &["model", "model_name", "modelName"]))
        .or_else(|| string(&payload, &["model", "model_name", "modelName"]))
        .unwrap_or_else(|| provider_label(input.provider).to_owned());
    let context_window_tokens = usage_roots
        .iter()
        .find_map(|root| {
            token_optional(
                root,
                &[
                    "context_window_tokens",
                    "contextWindow",
                    "contextWindowTokens",
                ],
            )
        })
        .or_else(|| {
            token_optional(
                &payload,
                &[
                    "context_window_tokens",
                    "contextWindow",
                    "contextWindowTokens",
                ],
            )
        });
    let event_id = stable_event_uuid(
        provider_label(input.provider),
        input.session_id,
        input.source_event_id,
    );
    Ok(ProviderUsageEvent {
        event_id: event_id.clone(),
        provider: input.provider,
        counter_mode: if input.provider == ProviderUsageKind::Claude
            || boolean(&payload, &["cumulative"]).unwrap_or(false)
        {
            UsageCounterMode::Cumulative
        } else {
            UsageCounterMode::Delta
        },
        usage: AgentUsageSample {
            agent_id: input.agent_id.trim().to_owned(),
            session_id: input.session_id.trim().to_owned(),
            timestamp_ms: input.timestamp_ms,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            model,
            usd,
        },
        context_window_tokens,
        tool_spans: tool_spans(&payload, input, event_id),
    })
}

#[derive(Default)]
pub struct ProviderUsageAccumulator {
    state: Mutex<AccumulatorState>,
}

#[derive(Default)]
struct AccumulatorState {
    totals: BTreeMap<String, AgentUsageSample>,
    events: BTreeMap<String, ProviderUsageEvent>,
    order: VecDeque<String>,
}

impl ProviderUsageAccumulator {
    pub fn accumulate(&self, event: &ProviderUsageEvent) -> ProviderUsageEvent {
        let Ok(mut state) = self.state.lock() else {
            return event.clone();
        };
        if let Some(existing) = state.events.get(&event.event_id) {
            return existing.clone();
        }
        let mut accumulated = event.clone();
        if event.counter_mode == UsageCounterMode::Delta {
            let key = format!(
                "{}\0{}\0{:?}",
                event.usage.agent_id, event.usage.session_id, event.provider
            );
            if let Some(current) = state.totals.get(&key) {
                accumulated.usage.input_tokens = current
                    .input_tokens
                    .saturating_add(event.usage.input_tokens);
                accumulated.usage.output_tokens = current
                    .output_tokens
                    .saturating_add(event.usage.output_tokens);
                accumulated.usage.cache_read_tokens = current
                    .cache_read_tokens
                    .saturating_add(event.usage.cache_read_tokens);
                accumulated.usage.cache_creation_tokens = current
                    .cache_creation_tokens
                    .saturating_add(event.usage.cache_creation_tokens);
                accumulated.usage.usd = current.usd + event.usage.usd;
            }
            state.totals.insert(key, accumulated.usage.clone());
        }
        state
            .events
            .insert(event.event_id.clone(), accumulated.clone());
        state.order.push_back(event.event_id.clone());
        if state.order.len() > MAX_DEDUP_EVENTS
            && let Some(expired) = state.order.pop_front()
        {
            state.events.remove(&expired);
        }
        accumulated
    }

    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = AccumulatorState::default();
        }
    }
}

pub fn context_percentage(event: &ProviderUsageEvent) -> Option<u8> {
    let window = event.context_window_tokens.filter(|window| *window > 0)?;
    let used = event
        .usage
        .input_tokens
        .saturating_add(event.usage.cache_read_tokens)
        .saturating_add(event.usage.cache_creation_tokens);
    Some(u8::try_from(used.saturating_mul(100).saturating_div(window).min(100)).unwrap_or(100))
}

fn usage_roots(
    provider: ProviderUsageKind,
    payload: &serde_json::Value,
) -> Vec<&serde_json::Value> {
    if provider == ProviderUsageKind::Claude
        && let Some(usage) = payload
            .get("modelUsage")
            .and_then(serde_json::Value::as_object)
        && !usage.is_empty()
    {
        return usage.values().collect();
    }
    vec![match provider {
        ProviderUsageKind::Gemini => payload
            .get("usageMetadata")
            .or_else(|| payload.get("usage"))
            .or_else(|| payload.get("stats"))
            .unwrap_or(payload),
        ProviderUsageKind::Claude | ProviderUsageKind::Codex => payload
            .get("usage")
            .or_else(|| payload.get("result").and_then(|result| result.get("usage")))
            .unwrap_or(payload),
    }]
}

fn tool_spans(
    payload: &serde_json::Value,
    input: &ProviderTranscriptEvent<'_>,
    event_id: String,
) -> Vec<ToolSpan> {
    ["tool_uses", "tool_calls", "tools"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(serde_json::Value::as_array))
        .into_iter()
        .flatten()
        .take(MAX_TOOL_SPANS)
        .enumerate()
        .filter_map(|(index, tool)| {
            let name = string(tool, &["name", "tool", "tool_name"])?;
            Some(ToolSpan {
                agent_id: input.agent_id.trim().to_owned(),
                session_id: input.session_id.trim().to_owned(),
                timestamp_ms: integer(tool, &["timestamp_ms", "started_at_ms"])
                    .and_then(|value| i64::try_from(value).ok())
                    .unwrap_or(input.timestamp_ms),
                tool: name,
                success: boolean(tool, &["success", "ok"]).unwrap_or_else(|| {
                    !string(tool, &["error", "error_type"]).is_some_and(|value| !value.is_empty())
                }),
                duration_ms: integer(tool, &["duration_ms", "durationMs"]).unwrap_or(0),
                decision: Some(format!("provider-event:{event_id}:{index}")),
                error: string(tool, &["error_type", "error_code"]),
            })
        })
        .collect()
}

fn token(value: &serde_json::Value, keys: &[&str]) -> u64 {
    token_optional(value, keys).unwrap_or(0)
}

fn token_optional(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    integer(value, keys)
}

fn integer(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
}

fn number(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
}

fn string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(serde_json::Value::as_str)
        .map(|value| value.chars().take(256).collect())
}

fn boolean(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(serde_json::Value::as_bool)
}

fn provider_label(provider: ProviderUsageKind) -> &'static str {
    match provider {
        ProviderUsageKind::Claude => "claude",
        ProviderUsageKind::Codex => "codex",
        ProviderUsageKind::Gemini => "gemini",
    }
}

fn stable_event_uuid(provider: &str, session_id: &str, source_event_id: &str) -> String {
    let mut high = 0xcbf2_9ce4_8422_2325_u64;
    let mut low = 0x8422_2325_cbf2_9ce4_u64;
    for byte in provider
        .bytes()
        .chain([0])
        .chain(session_id.bytes())
        .chain([0])
        .chain(source_event_id.bytes())
    {
        high = (high ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
        low = (low ^ u64::from(byte.rotate_left(1))).wrapping_mul(0x100_0000_01b3);
    }
    let mut bytes = ((u128::from(high) << 64) | u128::from(low)).to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::memory_skills::ProviderUsageKind;

    use super::{
        ProviderTranscriptEvent, ProviderUsageAccumulator, context_percentage,
        sanitize_provider_transcript,
    };

    fn event(provider: ProviderUsageKind, payload_json: &str) -> ProviderTranscriptEvent<'_> {
        ProviderTranscriptEvent {
            provider,
            source_event_id: "turn-42",
            agent_id: "agent-1",
            session_id: "session-1",
            timestamp_ms: 100,
            payload_json,
        }
    }

    #[test]
    fn claude_usage_and_tool_span_are_sanitized() {
        let parsed = sanitize_provider_transcript(&event(
            ProviderUsageKind::Claude,
            r#"{"modelUsage":{"claude-sonnet":{"inputTokens":800,"outputTokens":20,"cacheReadInputTokens":100,"costUSD":0.12,"model":"claude-sonnet","contextWindow":1000}},"tool_uses":[{"name":"Read","duration_ms":7,"success":true}],"prompt":"secret"}"#,
        ));
        let Ok(parsed) = parsed else {
            panic!("Claude usage did not parse")
        };
        assert_eq!(parsed.usage.usd, 0.12);
        assert_eq!(context_percentage(&parsed), Some(90));
        assert_eq!(parsed.tool_spans.len(), 1);
        assert!(!format!("{parsed:?}").contains("secret"));
    }

    #[test]
    fn codex_and_gemini_events_produce_nonzero_cost() {
        let codex = sanitize_provider_transcript(&event(
            ProviderUsageKind::Codex,
            r#"{"usage":{"input_tokens":10,"output_tokens":4,"cached_input_tokens":2,"cost_usd":0.03},"model":"gpt-5"}"#,
        ));
        let gemini = sanitize_provider_transcript(&event(
            ProviderUsageKind::Gemini,
            r#"{"usageMetadata":{"promptTokenCount":11,"candidatesTokenCount":5,"cachedContentTokenCount":3},"model":"gemini-2.5-pro","cost_usd":0.02}"#,
        ));
        let (Ok(codex), Ok(gemini)) = (codex, gemini) else {
            panic!("provider usage did not parse")
        };
        assert!(codex.usage.usd > 0.0);
        assert!(gemini.usage.usd > 0.0);
    }

    #[test]
    fn stable_source_event_produces_same_id_for_retry() {
        let first = sanitize_provider_transcript(&event(
            ProviderUsageKind::Codex,
            r#"{"usage":{"input_tokens":1,"output_tokens":1,"cost_usd":0.01}}"#,
        ));
        let second = sanitize_provider_transcript(&event(
            ProviderUsageKind::Codex,
            r#"{"usage":{"input_tokens":1,"output_tokens":1,"cost_usd":0.01}}"#,
        ));
        let (Ok(first), Ok(second)) = (first, second) else {
            panic!("provider usage did not parse")
        };
        assert_eq!(first.event_id, second.event_id);
    }

    #[test]
    fn delta_events_become_cumulative_and_retry_is_not_added_twice() {
        let accumulator = ProviderUsageAccumulator::default();
        let first = sanitize_provider_transcript(&event(
            ProviderUsageKind::Codex,
            r#"{"usage":{"input_tokens":3,"output_tokens":1,"cost_usd":0.02}}"#,
        ));
        let Ok(first) = first else {
            panic!("first provider usage did not parse")
        };
        let retried = accumulator.accumulate(&first);
        let duplicate = accumulator.accumulate(&first);
        assert_eq!(retried.usage.usd, duplicate.usage.usd);

        let mut second_input = event(
            ProviderUsageKind::Codex,
            r#"{"usage":{"input_tokens":2,"output_tokens":2,"cost_usd":0.03}}"#,
        );
        second_input.source_event_id = "turn-43";
        let second = sanitize_provider_transcript(&second_input);
        let Ok(second) = second else {
            panic!("second provider usage did not parse")
        };
        let cumulative = accumulator.accumulate(&second);
        assert_eq!(cumulative.usage.input_tokens, 5);
        assert_eq!(cumulative.usage.output_tokens, 3);
        assert!((cumulative.usage.usd - 0.05).abs() < f64::EPSILON);
    }
}
