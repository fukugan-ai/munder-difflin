use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::{
    Accent, AgentStatus, OfficeAgent, OfficeSnapshot, OfficeTheme, OfficeUiAction, TaskStatus,
};

const OFFICE_ISLAND_JS: Asset =
    asset!("/src/components/domains/office_ui/assets/office_floor_island.js");
const PIXI_JS: Asset = asset!("/src/components/domains/office_ui/assets/pixi.min.js");
const PORTRAIT_ART_JS: Asset = asset!("/src/components/domains/office_ui/assets/portrait_art.js");
const DARRYL_ART_JS: Asset = asset!("/src/components/domains/office_ui/assets/darryl_art.js");
const OFFICE_MAP: Asset = asset!("/src/components/domains/office_ui/assets/maps/office.tmj");
const BROOKLYN99_MAP: Asset =
    asset!("/src/components/domains/office_ui/assets/maps/brooklyn99.tmj");
const OFFICE_TILESET: Asset =
    asset!("/src/components/domains/office_ui/assets/tilesets/office-tileset.png");
const OFFICE_FLOORS_WALLS: Asset =
    asset!("/src/components/domains/office_ui/assets/tilesets/a5-office-floors-walls.png");
const OFFICE_INTERIORS: Asset =
    asset!("/src/components/domains/office_ui/assets/tilesets/interiors.png");

#[component]
pub(super) fn OfficeFloorIsland(
    snapshot: OfficeSnapshot,
    on_select: EventHandler<String>,
    on_open_tasks: EventHandler<()>,
    on_open_human_questions: EventHandler<()>,
    on_request_close: EventHandler<()>,
) -> Element {
    let current = &snapshot;
    let theme = theme_name(current.theme);
    let selected = current.selected_agent_id.as_deref().unwrap_or_default();
    let keyboard_agent_id = current
        .selected_agent_id
        .clone()
        .or_else(|| current.agents.first().map(|agent| agent.id.clone()));

    rsx! {
        document::Script { r#type: "module", src: OFFICE_ISLAND_JS }
        OfficeIslandActionBridge {
            on_select,
            on_open_tasks,
            on_open_human_questions,
            on_request_close,
        }
        OfficeHandoffBridge { handoffs: current.handoffs.clone() }
        div {
            class: "office-island",
            role: "application",
            aria_label: "AIチームのオフィスフロア",
            aria_describedby: "office-island-instructions",
            aria_keyshortcuts: "Enter T A Q",
            tabindex: "0",
            onkeydown: move |event| {
                match event.key() {
                    Key::Enter => {
                        event.prevent_default();
                        if let Some(agent_id) = &keyboard_agent_id {
                            on_select.call(agent_id.clone());
                        }
                    }
                    Key::Character(ref value) if value == " " => {
                        event.prevent_default();
                        if let Some(agent_id) = &keyboard_agent_id {
                            on_select.call(agent_id.clone());
                        }
                    }
                    Key::Character(ref value) if value.eq_ignore_ascii_case("t") => {
                        event.prevent_default();
                        on_open_tasks.call(());
                    }
                    Key::Character(ref value) if value.eq_ignore_ascii_case("a") => {
                        event.prevent_default();
                        on_open_human_questions.call(());
                    }
                    Key::Character(ref value) if value.eq_ignore_ascii_case("q") => {
                        event.prevent_default();
                        on_request_close.call(());
                    }
                    _ => {}
                }
            },
            "data-office-island": "true",
            "data-revision": current.revision.to_string(),
            "data-theme-id": theme,
            "data-office-map": OFFICE_MAP,
            "data-brooklyn99-map": BROOKLYN99_MAP,
            "data-office-tileset": OFFICE_TILESET,
            "data-office-floors-walls": OFFICE_FLOORS_WALLS,
            "data-office-interiors": OFFICE_INTERIORS,
            "data-selected-agent": selected,
            "data-paused": current.paused.to_string(),
            "data-portrait-art-src": PORTRAIT_ART_JS,
            "data-darryl-art-src": DARRYL_ART_JS,
            "data-pixi-src": PIXI_JS,
            p {
                id: "office-island-instructions",
                class: "office-island__instructions",
                "Enterでエージェントを選択、Tでタスク、Aで質問、Qで終了操作"
            }
            div { class: "office-island__sources", aria_hidden: "true",
                for agent in &current.agents {
                    div {
                        "data-office-agent": "true",
                        "data-id": agent.id.as_str(),
                        "data-name": agent.name.as_str(),
                        "data-character": character_name(agent),
                        "data-accent": accent_name(agent.accent),
                        "data-status": status_name(agent.status),
                        "data-action": agent.action.as_str(),
                        "data-last-prompt": agent.last_prompt.as_str(),
                        "data-carrying": agent.carrying.as_deref().unwrap_or_default(),
                        "data-is-god": agent.is_god.to_string(),
                    }
                }
                for task in &current.tasks {
                    div {
                        "data-office-task": "true",
                        "data-id": task.id.as_str(),
                        "data-status": task_status_name(task.status),
                        "data-assignee": task.assignee.as_deref().unwrap_or_default(),
                        "data-human-question": task.has_unanswered_human_qa.to_string(),
                    }
                }
            }
            canvas {
                class: "office-island__fallback",
                width: "960",
                height: "560",
                aria_hidden: "true",
            }
            p { class: "office-island__status", role: "status", "オフィスを準備しています…" }
        }
    }
}

#[component]
fn OfficeHandoffBridge(
    handoffs: Vec<md_web_contracts::domains::office_ui::HiveHandoff>,
) -> Element {
    use_effect(move || {
        let Ok(payload) = serde_json::to_string(&handoffs) else {
            return;
        };
        spawn(async move {
            let _ = document::eval(&format!(
                r#"
                const host = document.querySelector("[data-office-island]");
                if (!host) return;
                const handoffs = {payload}.sort((a, b) => a.sequence - b.sequence);
                const seen = host.__officeSeenHandoffIds || new Set();
                let last = Number(host.dataset.lastHandoffSequence || 0);
                for (const handoff of handoffs) {{
                  if (seen.has(handoff.event_id)) continue;
                  host.dispatchEvent(new CustomEvent("office-handoff", {{
                    detail: handoff,
                  }}));
                  seen.add(handoff.event_id);
                  last = Math.max(last, handoff.sequence);
                }}
                while (seen.size > 128) seen.delete(seen.values().next().value);
                host.__officeSeenHandoffIds = seen;
                host.dataset.lastHandoffSequence = String(last);
                return last;
                "#,
            ))
            .join::<u64>()
            .await;
        });
    });
    rsx! {}
}

#[component]
fn OfficeIslandActionBridge(
    on_select: EventHandler<String>,
    on_open_tasks: EventHandler<()>,
    on_open_human_questions: EventHandler<()>,
    on_request_close: EventHandler<()>,
) -> Element {
    use_effect(move || {
        spawn(async move {
            let mut evaluator = document::eval(
                r#"
                globalThis.__munderOfficeActionCleanup?.();
                const host = document.querySelector("[data-office-island]");
                if (!host) return;
                const forward = (event) => dioxus.send(event.detail);
                host.addEventListener("office-ui-action", forward);
                host.dataset.actionBridge = "ready";
                globalThis.__munderOfficeActionCleanup = () => {
                  host.removeEventListener("office-ui-action", forward);
                  delete host.dataset.actionBridge;
                };
                await new Promise(() => {});
                "#,
            );
            while let Ok(action) = evaluator.recv::<OfficeUiAction>().await {
                dispatch_action(
                    action,
                    on_select,
                    on_open_tasks,
                    on_open_human_questions,
                    on_request_close,
                );
            }
        });
    });
    rsx! {}
}

fn dispatch_action(
    action: OfficeUiAction,
    on_select: EventHandler<String>,
    on_open_tasks: EventHandler<()>,
    on_open_human_questions: EventHandler<()>,
    on_request_close: EventHandler<()>,
) {
    match action {
        OfficeUiAction::SelectAgent { agent_id } => on_select.call(agent_id),
        OfficeUiAction::OpenTasks => on_open_tasks.call(()),
        OfficeUiAction::OpenHumanQuestions => on_open_human_questions.call(()),
        OfficeUiAction::RequestClose => on_request_close.call(()),
    }
}

fn theme_name(theme: OfficeTheme) -> &'static str {
    match theme {
        OfficeTheme::Office => "office",
        OfficeTheme::Brooklyn99 => "brooklyn99",
        OfficeTheme::Friends => "friends",
        OfficeTheme::SiliconValley => "silicon_valley",
        OfficeTheme::Got => "got",
        OfficeTheme::Hogwarts => "hogwarts",
    }
}

fn accent_name(accent: Accent) -> &'static str {
    match accent {
        Accent::Coral => "coral",
        Accent::Mint => "mint",
        Accent::Sky => "sky",
        Accent::Lemon => "lemon",
        Accent::Lilac => "lilac",
        Accent::Peach => "peach",
    }
}

fn status_name(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Thinking => "thinking",
        AgentStatus::Working => "working",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Blocked => "blocked",
        AgentStatus::Success => "success",
        AgentStatus::Ghost => "ghost",
        AgentStatus::Compacting => "compacting",
        AgentStatus::Looping => "looping",
    }
}

fn task_status_name(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "todo",
        TaskStatus::Doing => "doing",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
    }
}

fn character_name(agent: &OfficeAgent) -> &'static str {
    use md_web_contracts::domains::office_ui::OfficeCharacter;

    match agent.character {
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

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::office_ui::{
        AgentStatus, OfficeAgent, OfficeCharacter, OfficeTheme, TaskStatus,
    };

    use super::{character_name, status_name, task_status_name, theme_name};

    #[test]
    fn office_theme_has_stable_island_value() {
        assert_eq!(theme_name(OfficeTheme::Office), "office");
    }

    #[test]
    fn blocked_status_has_stable_island_value() {
        assert_eq!(status_name(AgentStatus::Blocked), "blocked");
    }

    #[test]
    fn doing_task_has_stable_island_value() {
        assert_eq!(task_status_name(TaskStatus::Doing), "doing");
    }

    #[test]
    fn darryl_has_a_distinct_island_art_key() {
        let agent = OfficeAgent {
            id: String::from("darryl"),
            name: String::from("Darryl"),
            character: OfficeCharacter::Darryl,
            accent: Default::default(),
            status: Default::default(),
            project: String::new(),
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

        assert_eq!(character_name(&agent), "darryl");
    }
}
