#![forbid(unsafe_code)]

mod bridge;
mod domain;

pub use bridge::{
    VoiceBridgeEvents, VoiceBridgeScript, browser_configure_freeflow_shortcut,
    browser_connect_realtime, browser_disconnect_realtime, browser_send_realtime_notification,
    browser_send_realtime_tool_result, browser_set_input_device, browser_set_output_device,
    browser_start_freeflow, browser_stop_freeflow, browser_voice_capabilities,
};
pub use domain::VoiceRealtimeDomain;

use dioxus::prelude::*;
use md_web_contracts::domains::voice_realtime::{
    FreeflowConfig, FreeflowSnapshot, FreeflowStatus, RealtimeSessionSnapshot, RealtimeStatus,
};

#[component]
pub fn VoiceRealtimePanel(
    freeflow: FreeflowSnapshot,
    freeflow_config: FreeflowConfig,
    realtime: RealtimeSessionSnapshot,
    has_openai_key: bool,
    on_freeflow_toggle: EventHandler<()>,
    on_realtime_toggle: EventHandler<()>,
    on_open_voice_settings: EventHandler<()>,
) -> Element {
    let freeflow_busy = freeflow.status != FreeflowStatus::Idle;
    let realtime_busy = matches!(
        realtime.status,
        RealtimeStatus::Connecting | RealtimeStatus::Working
    );
    let realtime_live = realtime.status != RealtimeStatus::Off;
    let mic_available = realtime.secure_context;
    let freeflow_disabled = !freeflow_config.enabled
        || !freeflow_config.has_groq_key
        || !mic_available
        || freeflow.status == FreeflowStatus::Transcribing;
    let realtime_disabled = !has_openai_key || !mic_available || realtime_busy;

    rsx! {
        section {
            class: "voice-panel",
            aria_labelledby: "voice-panel-title",
            div { class: "voice-panel__heading",
                div {
                    h2 { id: "voice-panel-title", "音声" }
                    p { "下書き音声入力とMichaelとのリアルタイム会話" }
                }
                button {
                    class: "voice-button voice-button--quiet",
                    r#type: "button",
                    onclick: move |_| on_open_voice_settings.call(()),
                    "設定"
                }
            }

            if !mic_available {
                div { class: "voice-notice voice-notice--warning", role: "alert",
                    strong { "HTTPS接続が必要です" }
                    p {
                        "別PCからのマイク利用は安全なHTTPS接続でのみ有効です。現在のHTTP接続では音声機能を開始できません。"
                    }
                }
            }

            div { class: "voice-panel__grid",
                article { class: "voice-card", aria_labelledby: "freeflow-title",
                    div { class: "voice-card__body",
                        div { class: "voice-card__title-row",
                            div {
                                h3 { id: "freeflow-title", "Free Flow" }
                                p { "話した内容を選択中エージェントの下書きへ追加します。送信前に確認できます。" }
                            }
                            VoiceStatusBadge {
                                label: freeflow_label(freeflow.status),
                                state: freeflow_state(freeflow.status),
                            }
                        }
                        dl { class: "voice-meta",
                            div {
                                dt { "モデル" }
                                dd { {freeflow_config.model.as_str()} }
                            }
                            div {
                                dt { "Groqキー" }
                                dd { if freeflow_config.has_groq_key { "設定済み" } else { "未設定" } }
                            }
                        }
                        if let Some(error) = freeflow.error.as_deref() {
                            p { class: "voice-inline-error", role: "alert", {error} }
                        }
                    }
                    div { class: "voice-card__actions",
                        button {
                            class: "voice-button voice-button--primary",
                            r#type: "button",
                            disabled: freeflow_disabled,
                            "data-ui-state": if freeflow_busy { "loading" } else if freeflow_disabled { "disabled" } else { "default" },
                            aria_pressed: freeflow.status == FreeflowStatus::Recording,
                            onclick: move |_| on_freeflow_toggle.call(()),
                            match freeflow.status {
                                FreeflowStatus::Idle => "録音を開始",
                                FreeflowStatus::Recording => "録音を停止",
                                FreeflowStatus::Transcribing => "文字起こし中…",
                            }
                        }
                        span { class: "voice-card__hint", "Optionキー長押しでも録音できます" }
                    }
                }

                article { class: "voice-card", aria_labelledby: "realtime-title",
                    div { class: "voice-card__body",
                        div { class: "voice-card__title-row",
                            div {
                                h3 { id: "realtime-title", "Realtime Michael" }
                                p { "音声でフロアを確認し、エージェントやタスクを操作します。" }
                            }
                            VoiceStatusBadge {
                                label: realtime_label(realtime.status),
                                state: realtime_state(realtime.status),
                            }
                        }
                        dl { class: "voice-meta",
                            div {
                                dt { "モデル" }
                                dd { {realtime.model.as_deref().unwrap_or("未接続")} }
                            }
                            div {
                                dt { "セッション料金" }
                                dd { {format!("${:.4}", realtime.cost.usd)} }
                            }
                            div {
                                dt { "OpenAIキー" }
                                dd { if has_openai_key { "設定済み" } else { "未設定" } }
                            }
                        }
                        if realtime.cost.over_cap {
                            p { class: "voice-inline-error", role: "alert", "設定した料金上限へ到達しました。" }
                        }
                        if let Some(error) = realtime.error.as_deref() {
                            p { class: "voice-inline-error", role: "alert", {error} }
                        }
                    }
                    div { class: "voice-card__actions",
                        button {
                            class: "voice-button voice-button--primary",
                            r#type: "button",
                            disabled: realtime_disabled && !realtime_live,
                            "data-ui-state": if realtime_busy { "loading" } else if realtime_disabled && !realtime_live { "disabled" } else { "default" },
                            aria_pressed: realtime_live,
                            onclick: move |_| on_realtime_toggle.call(()),
                            if realtime_live { "会話を終了" } else if realtime.status == RealtimeStatus::Connecting { "接続中…" } else { "Michaelと話す" }
                        }
                        span { class: "voice-card__hint", aria_live: "polite",
                            {realtime_activity_label(&realtime)}
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn VoiceStatusBadge(label: &'static str, state: &'static str) -> Element {
    rsx! {
        span { class: "voice-status", "data-state": state, role: "status", {label} }
    }
}

pub fn freeflow_label(status: FreeflowStatus) -> &'static str {
    match status {
        FreeflowStatus::Idle => "待機中",
        FreeflowStatus::Recording => "録音中",
        FreeflowStatus::Transcribing => "文字起こし中",
    }
}

pub fn realtime_label(status: RealtimeStatus) -> &'static str {
    match status {
        RealtimeStatus::Off => "オフ",
        RealtimeStatus::Connecting => "接続中",
        RealtimeStatus::Listening => "聞いています",
        RealtimeStatus::Responding => "応答中",
        RealtimeStatus::Working => "作業中",
    }
}

fn freeflow_state(status: FreeflowStatus) -> &'static str {
    match status {
        FreeflowStatus::Idle => "idle",
        FreeflowStatus::Recording => "active",
        FreeflowStatus::Transcribing => "loading",
    }
}

fn realtime_state(status: RealtimeStatus) -> &'static str {
    match status {
        RealtimeStatus::Off => "idle",
        RealtimeStatus::Connecting => "loading",
        RealtimeStatus::Listening => "active",
        RealtimeStatus::Responding => "success",
        RealtimeStatus::Working => "loading",
    }
}

fn realtime_activity_label(snapshot: &RealtimeSessionSnapshot) -> &'static str {
    if snapshot.muted {
        "ツール実行中のためマイクを一時停止しています"
    } else {
        match snapshot.status {
            RealtimeStatus::Listening => "話しかけてください",
            RealtimeStatus::Responding => "Michaelが応答しています",
            RealtimeStatus::Working => "フロアを確認しています",
            _ => "マイクとスピーカーは設定から選択できます",
        }
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::voice_realtime::{
        FreeflowStatus, RealtimeSessionSnapshot, RealtimeStatus,
    };

    use super::{freeflow_label, realtime_activity_label, realtime_label};

    #[test]
    fn recording_label_is_japanese() {
        assert_eq!(freeflow_label(FreeflowStatus::Recording), "録音中");
    }

    #[test]
    fn working_label_is_japanese() {
        assert_eq!(realtime_label(RealtimeStatus::Working), "作業中");
    }

    #[test]
    fn muted_state_explains_tool_pause() {
        let snapshot = RealtimeSessionSnapshot {
            muted: true,
            ..RealtimeSessionSnapshot::default()
        };

        assert!(realtime_activity_label(&snapshot).contains("マイク"));
    }
}
