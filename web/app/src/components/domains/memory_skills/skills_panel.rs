use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{CatalogSkill, LocalSkill};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SkillMode {
    Installed,
    Catalog,
}

#[component]
pub(super) fn SkillsPanel(
    local: Vec<LocalSkill>,
    catalog: Vec<CatalogSkill>,
    busy: bool,
    on_refresh: EventHandler<()>,
    on_install: EventHandler<CatalogSkill>,
    on_uninstall: EventHandler<String>,
) -> Element {
    let mut mode = use_signal(|| SkillMode::Installed);
    let mut query = use_signal(String::new);
    let needle = query().trim().to_lowercase();
    rsx! {
        div { class: "domain-stack", "data-testid": "skills-panel",
            div { class: "domain-summary",
                div {
                    h2 { "スキル" }
                    p { "エージェントが利用できる指示パッケージを管理します。" }
                }
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_refresh.call(()),
                    "再読込"
                }
            }
            div { class: "domain-toolbar",
                div { class: "domain-segment", role: "tablist", aria_label: "スキル表示",
                    button {
                        class: if mode() == SkillMode::Installed { "is-active" } else { "" },
                        r#type: "button",
                        role: "tab",
                        aria_selected: mode() == SkillMode::Installed,
                        onclick: move |_| mode.set(SkillMode::Installed),
                        "インストール済み（{local.len()}）"
                    }
                    button {
                        class: if mode() == SkillMode::Catalog { "is-active" } else { "" },
                        r#type: "button",
                        role: "tab",
                        aria_selected: mode() == SkillMode::Catalog,
                        onclick: move |_| mode.set(SkillMode::Catalog),
                        "探す（{catalog.len()}）"
                    }
                }
                label { class: "domain-filter",
                    span { "絞り込み" }
                    input {
                        value: "{query}",
                        placeholder: "名前または説明…",
                        oninput: move |event| query.set(event.value()),
                    }
                }
            }
            ul { class: "domain-list",
                if mode() == SkillMode::Installed {
                    for skill in local.into_iter().filter(|skill| matches_query(&skill.name, &skill.description, &needle)) {
                        InstalledSkillRow { skill, busy, on_uninstall }
                    }
                } else {
                    for skill in catalog.into_iter().filter(|skill| matches_query(&skill.name, &skill.description, &needle)).take(300) {
                        CatalogSkillRow { skill, busy, on_install }
                    }
                }
            }
        }
    }
}

#[component]
fn InstalledSkillRow(skill: LocalSkill, busy: bool, on_uninstall: EventHandler<String>) -> Element {
    let managed_id = skill.managed_id.clone();
    rsx! {
        li { class: "domain-list__item domain-list__item--action",
            div {
                strong { {skill.name} }
                p { {skill.description} }
                span { class: "domain-meta", "{skill.provider:?} · {skill.scope:?}" }
            }
            button {
                class: "domain-button domain-button--danger",
                r#type: "button",
                disabled: busy,
                onclick: move |_| on_uninstall.call(managed_id.clone()),
                "削除"
            }
        }
    }
}

#[component]
fn CatalogSkillRow(
    skill: CatalogSkill,
    busy: bool,
    on_install: EventHandler<CatalogSkill>,
) -> Element {
    let install_skill = skill.clone();
    rsx! {
        li { class: "domain-list__item domain-list__item--action",
            div {
                strong { {skill.name} }
                p { {skill.description} }
                span { class: "domain-meta", "{skill.owner} · {skill.category}" }
            }
            button {
                class: "domain-button domain-button--primary",
                r#type: "button",
                disabled: busy,
                onclick: move |_| on_install.call(install_skill.clone()),
                "追加"
            }
        }
    }
}

fn matches_query(name: &str, description: &str, needle: &str) -> bool {
    needle.is_empty()
        || name.to_lowercase().contains(needle)
        || description.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::matches_query;

    #[test]
    fn filter_matches_description() {
        assert!(matches_query("Docs", "Create PDF files", "pdf"));
    }
}
