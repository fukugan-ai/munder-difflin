use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{
    ActivityEntry, AgentCostTotal, CatalogSkill, CommandHistoryEntry, CommandOutcome,
    KnowledgeDetail, KnowledgeDocument, KnowledgeHit, KnowledgeStatus, KnowledgeUploadRequest,
    LocalSkill, MemoryGraphSnapshot, MemoryStatus, ToolSpan,
};

use super::activity_panel::ActivityPanel;
use super::history_panel::HistoryPanel;
use super::knowledge_panel::KnowledgePanel;
use super::memory_panel::MemoryPanel;
use super::skills_panel::SkillsPanel;
use super::telemetry_panel::TelemetryPanel;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DomainTab {
    Memory,
    Knowledge,
    Skills,
    Activity,
    Traces,
    History,
}

#[component]
pub(crate) fn MemorySkillsWorkspace(
    memory_status: MemoryStatus,
    memory_result: Option<CommandOutcome>,
    #[props(default)] memory_graph: MemoryGraphSnapshot,
    knowledge_status: KnowledgeStatus,
    knowledge_documents: Vec<KnowledgeDocument>,
    knowledge_hits: Vec<KnowledgeHit>,
    #[props(default)] knowledge_detail: Option<KnowledgeDetail>,
    local_skills: Vec<LocalSkill>,
    catalog_skills: Vec<CatalogSkill>,
    activities: Vec<ActivityEntry>,
    spans: Vec<ToolSpan>,
    history: Vec<CommandHistoryEntry>,
    costs: Vec<AgentCostTotal>,
    busy: bool,
    error: Option<String>,
    on_memory_search: EventHandler<String>,
    #[props(default)] on_memory_wake_up: EventHandler<()>,
    on_memory_mine: EventHandler<()>,
    on_memory_reflect: EventHandler<()>,
    on_knowledge_search: EventHandler<String>,
    on_knowledge_upload: EventHandler<KnowledgeUploadRequest>,
    on_knowledge_remove: EventHandler<String>,
    #[props(default)] on_knowledge_get: EventHandler<String>,
    on_skill_refresh: EventHandler<()>,
    on_skill_install: EventHandler<CatalogSkill>,
    on_skill_uninstall: EventHandler<String>,
    on_activity_refresh: EventHandler<()>,
    on_history_search: EventHandler<String>,
) -> Element {
    let tab = use_signal(|| DomainTab::Memory);

    rsx! {
        section { class: "memory-skills", aria_labelledby: "memory-skills-title",
            div { class: "memory-skills__heading",
                div {
                    h1 { id: "memory-skills-title", "記憶と履歴" }
                    p { "チームの記憶、ナレッジ、スキル、実行履歴をここで確認します。" }
                }
                if busy {
                    span { class: "domain-state domain-state--loading", role: "status", "処理中…" }
                }
            }
            if let Some(message) = error {
                div { class: "domain-alert", role: "alert", {message} }
            }
            nav { class: "domain-tabs", aria_label: "記憶と履歴の表示切替",
                {tab_button("記憶", DomainTab::Memory, tab)}
                {tab_button("ナレッジ", DomainTab::Knowledge, tab)}
                {tab_button("スキル", DomainTab::Skills, tab)}
                {tab_button("活動", DomainTab::Activity, tab)}
                {tab_button("トレース", DomainTab::Traces, tab)}
                {tab_button("コマンド履歴", DomainTab::History, tab)}
            }
            div { class: "domain-panel",
                match tab() {
                    DomainTab::Memory => rsx! {
                        MemoryPanel {
                            status: memory_status,
                            result: memory_result,
                            graph: memory_graph,
                            busy,
                            on_search: on_memory_search,
                            on_wake_up: on_memory_wake_up,
                            on_mine: on_memory_mine,
                            on_reflect: on_memory_reflect,
                        }
                    },
                    DomainTab::Knowledge => rsx! {
                        KnowledgePanel {
                            status: knowledge_status,
                            documents: knowledge_documents,
                            hits: knowledge_hits,
                            detail: knowledge_detail,
                            busy,
                            on_search: on_knowledge_search,
                            on_upload: on_knowledge_upload,
                            on_remove: on_knowledge_remove,
                            on_get: on_knowledge_get,
                        }
                    },
                    DomainTab::Skills => rsx! {
                        SkillsPanel {
                            local: local_skills,
                            catalog: catalog_skills,
                            busy,
                            on_refresh: on_skill_refresh,
                            on_install: on_skill_install,
                            on_uninstall: on_skill_uninstall,
                        }
                    },
                    DomainTab::Activity => rsx! {
                        ActivityPanel { entries: activities, busy, on_refresh: on_activity_refresh }
                    },
                    DomainTab::Traces => rsx! { TelemetryPanel { spans, costs } },
                    DomainTab::History => rsx! {
                        HistoryPanel { entries: history, busy, on_search: on_history_search }
                    },
                }
            }
        }
    }
}

fn tab_button(label: &'static str, value: DomainTab, mut selected: Signal<DomainTab>) -> Element {
    let active = selected() == value;
    rsx! {
        button {
            class: if active { "domain-tab is-active" } else { "domain-tab" },
            r#type: "button",
            role: "tab",
            aria_selected: active,
            "data-ui-state": if active { "success" } else { "default" },
            onclick: move |_| selected.set(value),
            {label}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DomainTab;

    #[test]
    fn memory_is_a_distinct_workspace_tab() {
        assert_ne!(DomainTab::Memory, DomainTab::History);
    }
}
