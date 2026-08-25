use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::{
    AgentStatus, OfficeAgent, OfficeAgentTelemetry, OfficeCharacter,
};

#[component]
pub(super) fn AgentDetailHost(
    agent: Option<OfficeAgent>,
    content: Element,
    auto_mode: bool,
    focus_mode: bool,
    telemetry: Option<OfficeAgentTelemetry>,
    on_open_ide: EventHandler<String>,
    on_open_terminal: EventHandler<String>,
    on_close_agent: EventHandler<String>,
) -> Element {
    let Some(agent) = agent else {
        return rsx! {
            div { class: "agent-detail-host agent-detail-host--empty",
                p { "エージェントを選択してください。" }
            }
        };
    };
    let title = if agent.is_god {
        String::from("COMMAND CENTER")
    } else {
        agent.name.to_uppercase()
    };
    let status = status_label(agent.status);

    rsx! {
        section {
            class: "agent-detail-host",
            "data-agent-detail-id": agent.id.as_str(),
            "data-focus-detail": focus_mode.to_string(),
            header { class: "agent-detail-host__header",
                canvas {
                    class: "agent-detail-host__portrait",
                    width: "36",
                    height: "56",
                    aria_hidden: "true",
                    "data-office-portrait": character_name(agent.character),
                }
                div { class: "agent-detail-host__identity",
                    strong { {title} }
                    span { class: "agent-status", "{status}" }
                    small { {if agent.is_god { "Ariaがフロアを統括" } else { agent.project.as_str() }} }
                    if let Some(metrics) = telemetry {
                        small {
                            class: "agent-detail-host__telemetry",
                            {format!(
                                "${:.4} · in {} / out {}",
                                metrics.cost_usd_micros as f64 / 1_000_000.0,
                                metrics.input_tokens,
                                metrics.output_tokens,
                            )}
                        }
                    }
                }
                nav { class: "agent-detail-host__actions", aria_label: "エージェント操作",
                    if agent.is_god {
                        button { class: "office-button office-button--secondary", r#type: "button", disabled: true, "自動 {auto_mode}" }
                    }
                    button {
                        class: "office-button office-button--secondary",
                        r#type: "button",
                        onclick: {
                            let id = agent.id.clone();
                            move |_| on_open_ide.call(id.clone())
                        },
                        "IDE"
                    }
                    if !agent.is_god {
                        button {
                            class: "office-button office-button--secondary",
                            r#type: "button",
                            onclick: {
                                let id = agent.id.clone();
                                move |_| on_open_terminal.call(id.clone())
                            },
                            "開く"
                        }
                        button {
                            class: "office-button agent-detail-host__close",
                            r#type: "button",
                            aria_label: "{agent.name}を終了",
                            onclick: {
                                let id = agent.id.clone();
                                move |_| on_close_agent.call(id.clone())
                            },
                            "×"
                        }
                    }
                }
            }
            div {
                key: "{agent.id}",
                class: "agent-detail-host__content",
                "data-agent-content-id": agent.id.as_str(),
                {content}
            }
        }
    }
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "待機",
        AgentStatus::Thinking => "思考中",
        AgentStatus::Working => "作業中",
        AgentStatus::Waiting => "待機中",
        AgentStatus::Blocked => "要確認",
        AgentStatus::Success => "完了",
        AgentStatus::Ghost => "不在",
        AgentStatus::Compacting => "圧縮中",
        AgentStatus::Looping => "ループ",
    }
}

fn character_name(character: OfficeCharacter) -> &'static str {
    match character {
        OfficeCharacter::Michael => "michael",
        OfficeCharacter::Dwight => "dwight",
        OfficeCharacter::Pam => "pam",
        OfficeCharacter::Jim => "jim",
        OfficeCharacter::Stanley => "stanley",
        OfficeCharacter::Phyllis => "phyllis",
        OfficeCharacter::Angela => "angela",
        OfficeCharacter::Kevin => "kevin",
        OfficeCharacter::Oscar => "oscar",
        OfficeCharacter::Meredith => "meredith",
        OfficeCharacter::Creed => "creed",
        OfficeCharacter::Andy => "andy",
        OfficeCharacter::Ryan => "ryan",
        OfficeCharacter::Kelly => "kelly",
        OfficeCharacter::Toby => "toby",
        OfficeCharacter::Darryl => "darryl",
    }
}
