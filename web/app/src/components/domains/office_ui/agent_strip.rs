use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::{AgentStatus, OfficeAgent, OfficeSnapshot};

#[component]
pub(super) fn AgentStrip(
    snapshot: OfficeSnapshot,
    #[props(default)] compact_rail: bool,
    on_add_agent: EventHandler<()>,
    on_select: EventHandler<String>,
    on_reorder: EventHandler<(String, String)>,
    on_rename: EventHandler<(String, String)>,
    on_note: EventHandler<(String, String)>,
    on_open_task: EventHandler<String>,
    on_restore_all: EventHandler<()>,
    on_dismiss_restore: EventHandler<String>,
) -> Element {
    let mut dragged_id = use_signal(|| None::<String>);
    let agents = displayed_agents(&snapshot, compact_rail);

    rsx! {
        section { class: "agent-strip", aria_label: "エージェント一覧",
            div { class: "agent-strip__scroll",
                for (agent_index, agent) in agents.iter().enumerate() {
                    Fragment { key: "{agent.id}",
                        if compact_rail && (agent_index == 0 || agents[agent_index - 1].project != agent.project) {
                            h3 { class: "agent-strip__project", "{agent.project}" }
                        }
                        AgentCard {
                        agent: agent.clone(),
                        selected: snapshot.selected_agent_id.as_deref() == Some(agent.id.as_str()),
                        doing_count: snapshot.tasks.iter().filter(|task| {
                            task.status == md_web_contracts::domains::office_ui::TaskStatus::Doing
                                && task.assignee.as_deref() == Some(agent.id.as_str())
                        }).count(),
                        doing_task_id: snapshot.tasks.iter().find(|task| {
                            task.status == md_web_contracts::domains::office_ui::TaskStatus::Doing
                                && task.assignee.as_deref() == Some(agent.id.as_str())
                        }).map(|task| task.id.clone()),
                        on_select,
                        on_rename,
                        on_note,
                        on_open_task,
                        on_drag_start: move |id| dragged_id.set(Some(id)),
                        on_drop: move |target_id| {
                            if let Some(source_id) = dragged_id.take()
                                && source_id != target_id
                            {
                                on_reorder.call((source_id, target_id));
                            }
                        },
                        }
                    }
                }
                button {
                    class: "office-button office-button--secondary agent-strip__add",
                    r#type: "button",
                    onclick: move |_| on_add_agent.call(()),
                    span { aria_hidden: "true", "+" }
                    "エージェントを追加"
                }
            }

            if !snapshot.restorable_agents.is_empty() {
                details { class: "restore-menu",
                    summary { class: "office-button office-button--primary",
                        "チームを復元 ({snapshot.restorable_agents.len()})"
                    }
                    div { class: "restore-menu__panel",
                        strong { "前回のセッション" }
                        for restorable in &snapshot.restorable_agents {
                            div { class: "restore-menu__row", key: "{restorable.id}",
                                span {
                                    b { {restorable.name.as_str()} }
                                    small { {restorable.description.as_str()} }
                                }
                                button {
                                    class: "office-icon-button",
                                    r#type: "button",
                                    title: "復元一覧から外す",
                                    aria_label: "{restorable.name}を復元一覧から外す",
                                    onclick: {
                                        let id = restorable.id.clone();
                                        move |_| on_dismiss_restore.call(id.clone())
                                    },
                                    "×"
                                }
                            }
                        }
                        button {
                            class: "office-button office-button--primary",
                            r#type: "button",
                            onclick: move |_| on_restore_all.call(()),
                            "すべて復元"
                        }
                    }
                }
            }
        }
    }
}

fn displayed_agents(snapshot: &OfficeSnapshot, compact_rail: bool) -> Vec<OfficeAgent> {
    let mut agents = snapshot.agents.clone();
    if compact_rail {
        agents.sort_by_cached_key(|agent| (agent.project.to_ascii_lowercase(), !agent.is_god));
    }
    agents
}

#[component]
fn AgentCard(
    agent: OfficeAgent,
    selected: bool,
    doing_count: usize,
    doing_task_id: Option<String>,
    on_select: EventHandler<String>,
    on_rename: EventHandler<(String, String)>,
    on_note: EventHandler<(String, String)>,
    on_open_task: EventHandler<String>,
    on_drag_start: EventHandler<String>,
    on_drop: EventHandler<String>,
) -> Element {
    let mut editing_name = use_signal(|| agent.name.clone());
    let mut editing_note = use_signal(|| agent.note.clone());
    let status = status_label(agent.status, agent.has_terminal_draft);
    let status_class = status_class(agent.status, agent.has_terminal_draft);
    let info = if agent.status != AgentStatus::Idle && !agent.action.is_empty() {
        agent.action.as_str()
    } else {
        agent.project.as_str()
    };
    let progress = agent.progress_eighths.min(8) * 100 / 8;

    rsx! {
        article {
            class: if selected { "agent-card is-selected" } else { "agent-card" },
            "data-agent-id": agent.id.as_str(),
            "data-accent": accent_name(agent.accent),
            "data-is-god": agent.is_god.to_string(),
            draggable: "true",
            tabindex: "0",
            aria_current: selected.then_some("true"),
            ondragstart: {
                let id = agent.id.clone();
                move |_| on_drag_start.call(id.clone())
            },
            ondragover: move |event| event.prevent_default(),
            ondrop: {
                let id = agent.id.clone();
                move |event| {
                    event.prevent_default();
                    on_drop.call(id.clone());
                }
            },
            onclick: {
                let id = agent.id.clone();
                move |_| on_select.call(id.clone())
            },
            onkeydown: {
                let id = agent.id.clone();
                move |event| {
                    if event.key() == Key::Enter || event.key() == Key::Character(String::from(" ")) {
                        on_select.call(id.clone());
                    }
                }
            },

            if doing_count > 0 {
                button {
                    class: "agent-card__task-note",
                    r#type: "button",
                    title: "作業中のタスクを開く",
                    onclick: {
                        let task_id = doing_task_id;
                        move |event| {
                            event.stop_propagation();
                            if let Some(id) = &task_id {
                                on_open_task.call(id.clone());
                            }
                        }
                    },
                    if doing_count == 1 { "✎" } else { "{doing_count}" }
                }
            }

            div { class: "agent-card__portrait", aria_hidden: "true",
                canvas {
                    width: "36",
                    height: "56",
                    "data-office-portrait": character_name(agent.character),
                }
            }
            div { class: "agent-card__body",
                header {
                    strong { title: agent.name.as_str(), {agent.name.to_uppercase()} }
                    if agent.is_god { span { class: "agent-card__boss", "ボス" } }
                    span { class: "agent-status {status_class}", {status} }
                }
                p { class: "agent-card__info", title: info, {info} }
                if agent.is_god {
                    p { class: "agent-card__god-tools", "Talk · Cost" }
                } else {
                    details {
                        class: "agent-card__editor",
                        onclick: move |event| event.stop_propagation(),
                        summary { title: "名前とメモを編集", "{agent.note.lines().next().unwrap_or_default()} ✎" }
                        div { class: "agent-card__editor-panel",
                            label {
                                "表示名"
                                input {
                                    value: "{editing_name}",
                                    maxlength: "80",
                                    oninput: move |event| editing_name.set(event.value()),
                                }
                            }
                            button {
                                class: "office-button office-button--secondary",
                                r#type: "button",
                                onclick: {
                                    let id = agent.id.clone();
                                    move |_| on_rename.call((id.clone(), editing_name.read().clone()))
                                },
                                "名前を保存"
                            }
                            label {
                                "非公開メモ"
                                textarea {
                                    rows: "3",
                                    maxlength: "2000",
                                    value: "{editing_note}",
                                    oninput: move |event| editing_note.set(event.value()),
                                }
                            }
                            button {
                                class: "office-button office-button--secondary",
                                r#type: "button",
                                onclick: {
                                    let id = agent.id.clone();
                                    move |_| on_note.call((id.clone(), editing_note.read().clone()))
                                },
                                "メモを保存"
                            }
                        }
                    }
                }
                div {
                    class: "agent-card__gauge",
                    title: context_title(&agent),
                    span { style: "width: {progress}%" }
                }
            }
        }
    }
}

fn status_label(status: AgentStatus, typing: bool) -> &'static str {
    if typing {
        return "入力中";
    }
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

fn status_class(status: AgentStatus, typing: bool) -> &'static str {
    if typing {
        return "is-typing";
    }
    match status {
        AgentStatus::Idle => "is-idle",
        AgentStatus::Thinking => "is-thinking",
        AgentStatus::Working => "is-working",
        AgentStatus::Waiting => "is-waiting",
        AgentStatus::Blocked => "is-blocked",
        AgentStatus::Success => "is-success",
        AgentStatus::Ghost => "is-ghost",
        AgentStatus::Compacting => "is-compacting",
        AgentStatus::Looping => "is-looping",
    }
}

fn accent_name(accent: md_web_contracts::domains::office_ui::Accent) -> &'static str {
    use md_web_contracts::domains::office_ui::Accent;
    match accent {
        Accent::Coral => "coral",
        Accent::Mint => "mint",
        Accent::Sky => "sky",
        Accent::Lemon => "lemon",
        Accent::Lilac => "lilac",
        Accent::Peach => "peach",
    }
}

fn character_name(
    character: md_web_contracts::domains::office_ui::OfficeCharacter,
) -> &'static str {
    use md_web_contracts::domains::office_ui::OfficeCharacter;

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

fn context_title(agent: &OfficeAgent) -> String {
    match (agent.context_tokens, agent.context_limit) {
        (Some(tokens), Some(limit)) if limit > 0 => {
            format!("コンテキスト: {tokens} / {limit} tokens")
        }
        _ => String::from("コンテキスト使用量"),
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::office_ui::{
        Accent, AgentStatus, OfficeAgent, OfficeCharacter, OfficeSnapshot,
    };

    use super::{displayed_agents, status_label};

    #[test]
    fn terminal_draft_overrides_idle_label() {
        assert_eq!(status_label(AgentStatus::Idle, true), "入力中");
    }

    #[test]
    fn blocked_agent_uses_human_facing_label() {
        assert_eq!(status_label(AgentStatus::Blocked, false), "要確認");
    }

    #[test]
    fn focus_rail_groups_projects_stably() {
        let agent = |id: &str, project: &str| OfficeAgent {
            id: String::from(id),
            name: String::from(id),
            character: OfficeCharacter::Jim,
            accent: Accent::Sky,
            status: AgentStatus::Idle,
            project: String::from(project),
            action: String::new(),
            note: String::new(),
            last_prompt: String::new(),
            carrying: None,
            progress_eighths: 0,
            context_tokens: None,
            context_limit: None,
            has_terminal_draft: false,
            is_god: false,
        };
        let snapshot = OfficeSnapshot {
            agents: vec![agent("a", "zeta"), agent("b", "alpha"), agent("c", "zeta")],
            ..OfficeSnapshot::default()
        };

        let ids: Vec<_> = displayed_agents(&snapshot, true)
            .into_iter()
            .map(|agent| agent.id)
            .collect();
        assert_eq!(ids, ["b", "a", "c"]);
    }
}
