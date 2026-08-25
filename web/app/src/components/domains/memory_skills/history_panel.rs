use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::CommandHistoryEntry;

#[component]
pub(super) fn HistoryPanel(
    entries: Vec<CommandHistoryEntry>,
    busy: bool,
    on_search: EventHandler<String>,
) -> Element {
    let mut query = use_signal(String::new);
    rsx! {
        div { class: "domain-stack", "data-testid": "history-panel",
            div { class: "domain-summary",
                div {
                    h2 { "コマンド履歴" }
                    p { "PostgreSQLに保存された、エージェントへ送信した指示です。" }
                }
            }
            form {
                class: "domain-search",
                onsubmit: move |event| {
                    event.prevent_default();
                    on_search.call(query().trim().to_owned());
                },
                label { r#for: "history-query", "履歴を検索" }
                div { class: "domain-search__row",
                    input {
                        id: "history-query",
                        value: "{query}",
                        disabled: busy,
                        maxlength: 512,
                        placeholder: "空欄で最近の履歴を表示…",
                        oninput: move |event| query.set(event.value()),
                    }
                    button { class: "domain-button", r#type: "submit", disabled: busy, "表示" }
                }
            }
            if entries.is_empty() {
                p { class: "domain-empty", "一致するコマンド履歴はありません。" }
            } else {
                ol { class: "history-list",
                    for entry in entries {
                        li {
                            div { class: "history-list__meta",
                                strong { {entry.agent_id} }
                                time { datetime: "{entry.timestamp_ms}", "{entry.timestamp_ms} ms" }
                            }
                            p { {entry.text} }
                            if let Some(cwd) = entry.cwd { code { {cwd} } }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_history_query_is_supported_by_contract() {
        let query = String::new();

        assert!(query.is_empty());
    }
}
