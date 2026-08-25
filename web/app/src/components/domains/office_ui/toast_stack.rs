use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::CompletionNotice;

#[component]
pub(super) fn CompletionToasts(
    notices: Vec<CompletionNotice>,
    on_dismiss: EventHandler<(String, i64)>,
) -> Element {
    rsx! {
        if !notices.is_empty() {
            section {
                class: "office-toasts",
                aria_label: "完了通知",
                aria_live: "polite",
                for notice in &notices {
                    article {
                        class: "office-toast",
                        role: "status",
                        key: "{notice.correlation_id}:{notice.completed_at_ms}",
                        header {
                            span { aria_hidden: "true", "◆" }
                            strong { "Aria · 完了" }
                            button {
                                class: "office-icon-button",
                                r#type: "button",
                                aria_label: "通知を閉じる",
                                title: "通知を閉じる",
                                onclick: {
                                    let correlation_id = notice.correlation_id.clone();
                                    let completed_at_ms = notice.completed_at_ms;
                                    move |_| on_dismiss.call((correlation_id.clone(), completed_at_ms))
                                },
                                "×"
                            }
                        }
                        p { {notice.summary.as_str()} }
                        if let Some(objective) = &notice.objective {
                            small { {objective.as_str()} }
                        }
                    }
                }
            }
        }
    }
}
