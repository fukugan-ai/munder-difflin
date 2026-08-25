use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{AgentCostTotal, ToolSpan};

#[component]
pub(super) fn TelemetryPanel(spans: Vec<ToolSpan>, costs: Vec<AgentCostTotal>) -> Element {
    let origin = spans
        .iter()
        .map(|span| span.timestamp_ms)
        .min()
        .unwrap_or(0);
    let total = spans
        .iter()
        .map(|span| {
            u64::try_from(span.timestamp_ms.saturating_sub(origin))
                .unwrap_or(0)
                .saturating_add(span.duration_ms)
        })
        .max()
        .unwrap_or(1);
    rsx! {
        div { class: "domain-stack", "data-testid": "telemetry-panel",
            div { class: "domain-summary",
                div {
                    h2 { "ツールトレース" }
                    p { "直近のツール呼び出しと、PostgreSQLに記録された累積コストです。" }
                }
            }
            if !costs.is_empty() {
                dl { class: "cost-list",
                    for cost in costs {
                        div {
                            dt { {cost.agent_id} }
                            dd { {format_cost(cost.usd)} }
                        }
                    }
                }
            }
            if spans.is_empty() {
                p { class: "domain-empty", "ツール呼び出しはまだありません。" }
            } else {
                ul { class: "trace-list",
                    for span in spans.into_iter().take(60) {
                        TraceRow { span, origin, total }
                    }
                }
            }
        }
    }
}

#[component]
fn TraceRow(span: ToolSpan, origin: i64, total: u64) -> Element {
    let tool_title = span.tool.clone();
    let offset = u64::try_from(span.timestamp_ms.saturating_sub(origin)).unwrap_or(0);
    rsx! {
        li {
            span { class: "trace-list__tool", title: "{tool_title}", {span.tool} }
            div { class: "trace-list__track", aria_hidden: "true",
                span {
                    class: if span.success { "trace-list__bar is-success" } else { "trace-list__bar is-error" },
                    style: "inset-inline-start: {bar_width(offset, total)}%; width: {bar_width(span.duration_ms, total)}%",
                }
            }
            span { class: "trace-list__duration", {format_duration(span.duration_ms)} }
            span { class: if span.success { "trace-list__result is-success" } else { "trace-list__result is-error" },
                if span.success { "成功" } else { "失敗" }
            }
        }
    }
}

fn bar_width(duration_ms: u64, maximum: u64) -> u64 {
    duration_ms
        .saturating_mul(100)
        .checked_div(maximum.max(1))
        .unwrap_or(0)
        .clamp(2, 100)
}

fn format_duration(duration_ms: u64) -> String {
    if duration_ms >= 1_000 {
        format!("{:.1}s", duration_ms as f64 / 1_000.0)
    } else {
        format!("{duration_ms}ms")
    }
}

fn format_cost(usd: f64) -> String {
    format!("${:.4}", usd.max(0.0))
}

#[cfg(test)]
mod tests {
    use super::{bar_width, format_cost, format_duration};

    #[test]
    fn smallest_trace_remains_visible() {
        assert_eq!(bar_width(0, 100), 2);
    }

    #[test]
    fn seconds_use_one_decimal_place() {
        assert_eq!(format_duration(1_500), "1.5s");
    }

    #[test]
    fn negative_cost_is_not_rendered() {
        assert_eq!(format_cost(-1.0), "$0.0000");
    }
}
