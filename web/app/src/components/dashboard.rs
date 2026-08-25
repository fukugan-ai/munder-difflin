use dioxus::prelude::*;
use md_web_contracts::HealthSnapshot;

use super::status_badge::{PersistenceBadge, ServerStatusBadge};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HealthViewState {
    Loading,
    Ready(HealthSnapshot),
    Error(String),
}

impl HealthViewState {
    fn refresh_state(&self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready(_) => "success",
            Self::Error(_) => "error",
        }
    }
}

#[component]
pub(crate) fn Dashboard(health: HealthViewState, on_refresh: EventHandler<()>) -> Element {
    let refresh_state = health.refresh_state();
    let is_loading = matches!(health, HealthViewState::Loading);

    rsx! {
        main { class: "dashboard", id: "main-content",
            section { class: "office", aria_labelledby: "office-title",
                div { class: "office__heading",
                    div {
                        h1 { id: "office-title", "オフィス" }
                        p { "ローカルで動く、あなた専用のAIチームです。" }
                    }
                    button {
                        class: "ui-button ui-button--primary",
                        r#type: "button",
                        disabled: true,
                        "data-ui-state": "disabled",
                        title: "この機能は次の移植スライスで追加します",
                        "＋ エージェントを追加（準備中）"
                    }
                }

                div { class: "office-floor", aria_hidden: "true",
                    div { class: "office-floor__window" }
                    div { class: "office-floor__desk office-floor__desk--left" }
                    div { class: "office-floor__desk office-floor__desk--right" }
                    div { class: "office-floor__plant" }
                }

                div { class: "empty-office",
                    h2 { "エージェントがいません" }
                    p { "エージェントを追加すると、ここにAIの作業状況が表示されます。" }
                }
            }

            aside { class: "health-panel", aria_labelledby: "health-title",
                div { class: "panel-heading",
                    div {
                        h2 { id: "health-title", "ローカル環境" }
                        p { "Webサーバーと永続ストレージの現在の状態" }
                    }
                    button {
                        class: "ui-button ui-button--icon",
                        r#type: "button",
                        disabled: is_loading,
                        "data-ui-state": refresh_state,
                        aria_label: "状態を再読み込み",
                        title: "状態を再読み込み",
                        onclick: move |_| on_refresh.call(()),
                        if is_loading { "…" } else { "↻" }
                    }
                }

                match health {
                    HealthViewState::Loading => rsx! {
                        div { class: "health-loading", role: "status",
                            span { class: "health-loading__bar" }
                            span { "状態を確認しています…" }
                        }
                    },
                    HealthViewState::Error(message) => rsx! {
                        div { class: "health-error", role: "alert",
                            strong { "サーバーへ接続できません" }
                            p { {message} }
                        }
                    },
                    HealthViewState::Ready(snapshot) => rsx! {
                        dl { class: "health-list",
                            div { class: "health-list__row",
                                dt { "Webサーバー" }
                                dd { ServerStatusBadge { available: true } }
                            }
                            div { class: "health-list__row",
                                dt { "PostgreSQL" }
                                dd { PersistenceBadge { status: snapshot.persistence } }
                            }
                            div { class: "health-list__row",
                                dt { "アプリ版" }
                                dd { class: "health-list__value", {snapshot.app_version} }
                            }
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HealthViewState;

    #[test]
    fn loading_state_disables_refresh() {
        assert_eq!(HealthViewState::Loading.refresh_state(), "loading");
    }

    #[test]
    fn error_state_marks_refresh_as_error() {
        let state = HealthViewState::Error(String::from("offline"));

        assert_eq!(state.refresh_state(), "error");
    }
}
