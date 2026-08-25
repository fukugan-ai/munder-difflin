use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::ActivityEntry;

#[component]
pub(super) fn ActivityPanel(
    entries: Vec<ActivityEntry>,
    busy: bool,
    on_refresh: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "domain-stack", "data-testid": "activity-panel",
            div { class: "domain-summary",
                div {
                    h2 { "アクティビティ" }
                    p { "起動、メッセージ、アーカイブなどのメタデータ履歴です。" }
                }
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_refresh.call(()),
                    "再読込"
                }
            }
            if entries.is_empty() {
                p { class: "domain-empty", "記録されたアクティビティはありません。" }
            } else {
                ol { class: "activity-list",
                    for entry in entries.into_iter().rev() {
                        li {
                            span { class: "activity-list__kind", {entry.kind} }
                            div {
                                strong { {entry.summary} }
                                time { datetime: "{entry.timestamp_ms}", {format_timestamp(entry.timestamp_ms)} }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn format_timestamp(timestamp_ms: i64) -> String {
    if timestamp_ms <= 0 {
        return String::from("時刻不明");
    }
    format!("{} ms", timestamp_ms)
}

#[cfg(test)]
mod tests {
    use super::format_timestamp;

    #[test]
    fn zero_timestamp_is_unknown() {
        assert_eq!(format_timestamp(0), "時刻不明");
    }
}
