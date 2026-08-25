use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::{
    CompletionNotice, OfficeAgent, OfficeAgentSpawnRequest, OfficeSnapshot, ThemePreference,
};

use super::add_agent_modal::AddAgentModal;
use super::agent_strip::AgentStrip;
use super::detail_host::AgentDetailHost;
use super::floor_island::OfficeFloorIsland;
use super::toast_stack::CompletionToasts;

const OFFICE_UI_CSS: Asset = asset!("/assets/domains/office_ui.css");
const BRAND_LOGO: Asset = asset!("/src/components/domains/office_ui/assets/logo.png");

#[component]
pub(crate) fn OfficeUi(
    snapshot: OfficeSnapshot,
    notices: Vec<CompletionNotice>,
    auto_mode: bool,
    #[props(default)] focus_mode: bool,
    app_version: String,
    detail_panel: Element,
    on_add_agent: EventHandler<()>,
    #[props(default)] add_agent_open: bool,
    #[props(default)] add_agent_spawning: bool,
    on_close_add_agent: EventHandler<()>,
    on_spawn_agent: EventHandler<OfficeAgentSpawnRequest>,
    on_open_ide: EventHandler<String>,
    on_open_terminal: EventHandler<String>,
    on_close_agent: EventHandler<String>,
    on_select: EventHandler<String>,
    on_reorder: EventHandler<(String, String)>,
    on_rename: EventHandler<(String, String)>,
    on_note: EventHandler<(String, String)>,
    on_open_task: EventHandler<String>,
    on_open_tasks: EventHandler<()>,
    on_open_human_questions: EventHandler<()>,
    on_request_close: EventHandler<()>,
    on_restore_all: EventHandler<()>,
    on_dismiss_restore: EventHandler<String>,
    on_theme: EventHandler<ThemePreference>,
    on_open_settings: EventHandler<()>,
    on_toggle_focus: EventHandler<()>,
    on_dismiss_notice: EventHandler<(String, i64)>,
) -> Element {
    let next_theme = next_theme(snapshot.theme_preference);
    let has_agents = !snapshot.agents.is_empty();
    let selected_agent = resolve_selected_agent(&snapshot);
    let resolved_selected_agent_id = selected_agent.as_ref().map(|agent| agent.id.clone());
    let selected_telemetry = resolved_selected_agent_id.as_ref().and_then(|selected| {
        snapshot
            .telemetry
            .iter()
            .find(|telemetry| &telemetry.agent_id == selected)
            .cloned()
    });
    let mut view_snapshot = snapshot.clone();
    view_snapshot.selected_agent_id = resolved_selected_agent_id.clone();

    rsx! {
        document::Link { rel: "stylesheet", href: OFFICE_UI_CSS }
        div {
            class: "office-domain",
            tabindex: "-1",
            "data-theme": theme_attribute(snapshot.theme_preference),
            "data-focus-mode": focus_mode.to_string(),
            "data-selected-agent-id": resolved_selected_agent_id,
            onkeydown: move |event| {
                if should_exit_focus(focus_mode, &event.key()) {
                    event.prevent_default();
                    on_toggle_focus.call(());
                }
            },
            CompletionToasts { notices, on_dismiss: on_dismiss_notice }
            if add_agent_open {
                AddAgentModal {
                    spawning: add_agent_spawning,
                    on_cancel: on_close_add_agent,
                    on_spawn: on_spawn_agent,
                }
            }

            header { class: "office-titlebar",
                span { class: "office-window-lights", aria_hidden: "true",
                    i {}
                    i {}
                    i {}
                }
                div { class: "office-brand", aria_label: "Munder Difflin",
                    img { class: "office-brand__mark", src: BRAND_LOGO, alt: "" }
                    if focus_mode { strong { "MUNDER DIFFLIN · FULLSCREEN" } }
                }
                if !focus_mode {
                    span { class: "office-version", "v{app_version}" }
                    span { class: "office-auto-mode",
                        if auto_mode { "自動モード：オン" } else { "自動モード：オフ" }
                    }
                }
                nav { class: "office-titlebar__actions", aria_label: "表示と設定",
                    button {
                        class: "office-icon-button",
                        r#type: "button",
                        title: "表示テーマを切り替える",
                        aria_label: "表示テーマを切り替える",
                        onclick: move |_| on_theme.call(next_theme),
                        span { aria_hidden: "true", {theme_glyph(snapshot.theme_preference)} }
                    }
                    button {
                        class: "office-icon-button",
                        r#type: "button",
                        title: "設定",
                        aria_label: "設定を開く",
                        onclick: move |_| on_open_settings.call(()),
                        span { aria_hidden: "true", "⚙" }
                    }
                    button {
                        class: "office-icon-button",
                        r#type: "button",
                        title: "集中モード",
                        aria_label: "集中モードを切り替える",
                        onclick: move |_| on_toggle_focus.call(()),
                        span { aria_hidden: "true", "⛶" }
                    }
                }
            }

            if focus_mode {
                main { class: "office-focus-workspace", id: "main-content",
                    AgentStrip {
                        snapshot: view_snapshot,
                        compact_rail: true,
                        on_add_agent,
                        on_select,
                        on_reorder,
                        on_rename,
                        on_note,
                        on_open_task,
                        on_restore_all,
                        on_dismiss_restore,
                    }
                    section { class: "office-focus-detail", aria_label: "選択中のエージェント",
                        AgentDetailHost {
                            agent: selected_agent,
                            content: detail_panel,
                            auto_mode,
                            focus_mode: true,
                            telemetry: selected_telemetry,
                            on_open_ide,
                            on_open_terminal,
                            on_close_agent,
                        }
                    }
                }
            } else {
                main { class: "office-workspace", id: "main-content",
                    section { class: "office-stage", aria_label: "オフィス",
                        OfficeFloorIsland {
                            snapshot: view_snapshot.clone(),
                            on_select,
                            on_open_tasks,
                            on_open_human_questions,
                            on_request_close,
                        }
                        if !has_agents {
                            div { class: "office-empty",
                                h1 { "エージェントがいません" }
                                p { "エージェントを追加すると、ここにAIの作業状況がリアルタイムで表示されます。" }
                                button {
                                    class: "office-button office-button--primary",
                                    r#type: "button",
                                    onclick: move |_| on_add_agent.call(()),
                                    "+ エージェントを追加"
                                }
                            }
                        }
                    }
                    aside { class: "office-detail", aria_label: "選択中のエージェント",
                        AgentDetailHost {
                            agent: selected_agent,
                            content: detail_panel,
                            auto_mode,
                            focus_mode: false,
                            telemetry: selected_telemetry,
                            on_open_ide,
                            on_open_terminal,
                            on_close_agent,
                        }
                    }
                }
                AgentStrip {
                    snapshot: view_snapshot,
                    on_add_agent,
                    on_select,
                    on_reorder,
                    on_rename,
                    on_note,
                    on_open_task,
                    on_restore_all,
                    on_dismiss_restore,
                }
            }
        }
    }
}

fn resolve_selected_agent(snapshot: &OfficeSnapshot) -> Option<OfficeAgent> {
    snapshot
        .selected_agent_id
        .as_ref()
        .and_then(|selected| snapshot.agents.iter().find(|agent| &agent.id == selected))
        .or_else(|| snapshot.agents.iter().find(|agent| agent.is_god))
        .or_else(|| snapshot.agents.first())
        .cloned()
}

fn should_exit_focus(focus_mode: bool, key: &Key) -> bool {
    focus_mode && key == &Key::Escape
}

fn next_theme(preference: ThemePreference) -> ThemePreference {
    match preference {
        ThemePreference::System => ThemePreference::Dark,
        ThemePreference::Dark => ThemePreference::Light,
        ThemePreference::Light => ThemePreference::System,
    }
}

fn theme_attribute(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::System => "system",
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
    }
}

fn theme_glyph(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::System => "◐",
        ThemePreference::Dark => "☀",
        ThemePreference::Light => "☾",
    }
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::Key;
    use md_web_contracts::domains::office_ui::{
        Accent, AgentStatus, OfficeAgent, OfficeCharacter, OfficeSnapshot, ThemePreference,
    };

    use super::{next_theme, resolve_selected_agent, should_exit_focus, theme_attribute};

    fn agent(id: &str, is_god: bool) -> OfficeAgent {
        OfficeAgent {
            id: String::from(id),
            name: if is_god {
                String::from("Aria")
            } else {
                String::from(id)
            },
            character: OfficeCharacter::Michael,
            accent: Accent::Lemon,
            status: AgentStatus::Idle,
            project: String::from("munder-difflin"),
            action: String::new(),
            note: String::new(),
            last_prompt: String::new(),
            carrying: None,
            progress_eighths: 0,
            context_tokens: None,
            context_limit: None,
            has_terminal_draft: false,
            is_god,
        }
    }

    #[test]
    fn theme_cycle_returns_to_system() {
        let cycled = next_theme(next_theme(next_theme(ThemePreference::System)));

        assert_eq!(cycled, ThemePreference::System);
    }

    #[test]
    fn dark_theme_has_stable_attribute() {
        assert_eq!(theme_attribute(ThemePreference::Dark), "dark");
    }

    #[test]
    fn selected_detail_resolves_live_id_then_aria_then_first() {
        let mut snapshot = OfficeSnapshot {
            agents: vec![agent("worker", false), agent("god", true)],
            selected_agent_id: Some(String::from("worker")),
            ..OfficeSnapshot::default()
        };
        assert_eq!(
            resolve_selected_agent(&snapshot).map(|value| value.id),
            Some(String::from("worker"))
        );

        snapshot.selected_agent_id = Some(String::from("stale"));
        assert_eq!(
            resolve_selected_agent(&snapshot).map(|value| value.id),
            Some(String::from("god"))
        );

        snapshot.agents.retain(|value| !value.is_god);
        assert_eq!(
            resolve_selected_agent(&snapshot).map(|value| value.id),
            Some(String::from("worker"))
        );
    }

    #[test]
    fn escape_only_exits_while_focus_mode_is_active() {
        assert!(should_exit_focus(true, &Key::Escape));
        assert!(!should_exit_focus(false, &Key::Escape));
        assert!(!should_exit_focus(true, &Key::Enter));
    }
}
