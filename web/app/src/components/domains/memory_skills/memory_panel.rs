use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{CommandOutcome, MemoryGraphSnapshot, MemoryStatus};

#[component]
pub(super) fn MemoryPanel(
    status: MemoryStatus,
    result: Option<CommandOutcome>,
    graph: MemoryGraphSnapshot,
    busy: bool,
    on_search: EventHandler<String>,
    on_wake_up: EventHandler<()>,
    on_mine: EventHandler<()>,
    on_reflect: EventHandler<()>,
) -> Element {
    let mut query = use_signal(String::new);
    let state = memory_state(&status);
    rsx! {
        div { class: "domain-stack", "data-testid": "memory-panel",
            div { class: "domain-summary",
                div {
                    h2 { "共有メモリ" }
                    p { "各エージェントの memory.md を横断し、意味から検索します。" }
                }
                span { class: format!("domain-state domain-state--{}", state.0), {state.1} }
            }
            form {
                class: "domain-search",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = query().trim().to_owned();
                    if !value.is_empty() { on_search.call(value); }
                },
                label { r#for: "memory-query", "記憶を検索" }
                div { class: "domain-search__row",
                    input {
                        id: "memory-query",
                        value: "{query}",
                        disabled: busy,
                        maxlength: 512,
                        placeholder: "決定、原因、手順を検索…",
                        oninput: move |event| query.set(event.value()),
                    }
                    button {
                        class: "domain-button domain-button--primary",
                        r#type: "submit",
                        disabled: busy || query().trim().is_empty(),
                        "data-ui-state": if busy { "loading" } else { "default" },
                        if busy { "検索中…" } else { "検索" }
                    }
                }
            }
            div { class: "domain-actions",
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy || !status.active,
                    onclick: move |_| on_wake_up.call(()),
                    "Wake-upダイジェスト"
                }
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy || !status.active,
                    onclick: move |_| on_mine.call(()),
                    "今すぐ索引更新"
                }
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_reflect.call(()),
                    "メモリを圧縮"
                }
            }
            if let Some(outcome) = result {
                div {
                    class: if outcome.ok { "domain-result" } else { "domain-result domain-result--error" },
                    role: if outcome.ok { "status" } else { "alert" },
                    pre { {outcome.output} }
                    if let Some(message) = outcome.error { p { {message} } }
                }
            }
            if !graph.nodes.is_empty() {
                section { class: "domain-section memory-graph", aria_labelledby: "memory-graph-title",
                    h3 { id: "memory-graph-title", "Memory Graph" }
                    div { class: "memory-graph__nodes",
                        for node in graph.nodes.iter().take(80) {
                            span {
                                class: "memory-graph__node",
                                title: "{node.modality} · weight {node.weight}",
                                {node.label.clone()}
                            }
                        }
                    }
                    ul { class: "memory-graph__edges",
                        for edge in graph.edges.iter().take(120) {
                            li { code { "{edge.source} → {edge.target}" } span { " {edge.relation}" } }
                        }
                    }
                }
            }
        }
    }
}

fn memory_state(status: &MemoryStatus) -> (&'static str, &'static str) {
    if !status.available {
        ("error", "未設定")
    } else if !status.enabled {
        ("disabled", "オフ")
    } else if status.initialized {
        ("success", "準備完了")
    } else {
        ("loading", "準備中")
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::memory_skills::{EmbeddingModel, MemoryStatus};

    use super::memory_state;

    #[test]
    fn unavailable_memory_is_an_error_state() {
        let status = MemoryStatus {
            available: false,
            enabled: true,
            active: false,
            initialized: false,
            model: EmbeddingModel::MiniLm,
        };

        assert_eq!(memory_state(&status).0, "error");
    }
}
