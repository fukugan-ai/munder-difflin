use std::collections::BTreeSet;

use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{
    BaseSkillCatalogEntry, BaseSkillCatalogSnapshot, BaseSkillSelectionRequest,
    BaseSkillSourceKind, TeamSkillAssignments,
};

#[component]
pub(crate) fn BaseSkillsOnboardingPanel(
    catalog: BaseSkillCatalogSnapshot,
    assignments: TeamSkillAssignments,
    busy: bool,
    error: Option<String>,
    on_refresh: EventHandler<()>,
    on_install: EventHandler<BaseSkillSelectionRequest>,
    on_save_assignments: EventHandler<TeamSkillAssignments>,
) -> Element {
    let selected = use_signal(BTreeSet::<String>::new);
    let mut confirmed = use_signal(|| false);
    let selected_count = selected.read().len();
    rsx! {
        section { class: "domain-stack base-skills", aria_labelledby: "base-skills-title",
            div { class: "domain-summary",
                div {
                    h2 { id: "base-skills-title", "ベーススキルを選ぶ" }
                    p { "出典と互換性を確認し、このサーバーへ入れるものだけを選択します。" }
                }
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_refresh.call(()),
                    "公式・設定済みsourceを更新"
                }
            }
            if let Some(message) = error {
                div { class: "domain-alert", role: "alert", {message} }
            }
            ul { class: "domain-list", aria_label: "ベーススキルsource",
                for source in catalog.sources {
                    li { class: "domain-list__item",
                        strong { {source.name} }
                        p { class: "domain-meta", "{source.repository} · {source.reference}" }
                        p { class: "domain-meta",
                            "種別: {source_kind_label(source.kind)} · 認証: {authentication_label(source.kind, source.authentication_configured)}"
                        }
                        span { class: "domain-state domain-state--success",
                            if source.official { "公式" } else { "設定済みGitHub" }
                        }
                    }
                }
            }
            div { class: "base-skills__catalog",
                for skill in catalog.skills {
                    BaseSkillRow { skill, busy, selected }
                }
            }
            label { class: "base-skills__confirmation",
                input {
                    r#type: "checkbox",
                    checked: confirmed(),
                    disabled: busy || selected_count == 0,
                    onchange: move |event| confirmed.set(event.checked()),
                }
                "選択したスキルをこのサーバーへインストールすることを確認しました"
            }
            button {
                class: "domain-button domain-button--primary",
                r#type: "button",
                disabled: busy || selected_count == 0 || !confirmed(),
                onclick: move |_| on_install.call(BaseSkillSelectionRequest {
                    skill_ids: selected.read().iter().cloned().collect(),
                    confirmed: confirmed(),
                }),
                "{selected_count}件をインストール"
            }
            section { class: "domain-section", aria_labelledby: "team-skills-title",
                h3 { id: "team-skills-title", "最小ソフトウェアチーム" }
                p { "専門担当はtaskが該当するときだけ追加します。各agentには割当済みスキルだけを渡します。" }
                ul { class: "domain-list",
                    for assignment in assignments.assignments.iter() {
                        li { class: "domain-list__item",
                            strong { "{assignment.display_name} · {assignment.role:?}" }
                            p { {assignment.skill_ids.join(" · ")} }
                            if let Some(condition) = &assignment.task_condition {
                                span { class: "domain-meta", "条件: {condition}" }
                            }
                        }
                    }
                }
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_save_assignments.call(assignments.clone()),
                    "この割当を保存"
                }
            }
        }
    }
}

fn source_kind_label(kind: BaseSkillSourceKind) -> &'static str {
    match kind {
        BaseSkillSourceKind::OpenAiProjectSkillsApi => "OpenAI Project Skills API",
        BaseSkillSourceKind::OpenAiPluginMarketplace => "OpenAI GitHub marketplace",
        BaseSkillSourceKind::AnthropicAgentSkills => "Anthropic Agent Skills",
        BaseSkillSourceKind::AnthropicPluginMarketplace => "Anthropic plugin marketplace",
        BaseSkillSourceKind::GitHubRepository => "追加GitHub source",
    }
}

fn authentication_label(kind: BaseSkillSourceKind, configured: bool) -> &'static str {
    if configured {
        "server側で設定済み"
    } else if kind == BaseSkillSourceKind::OpenAiProjectSkillsApi {
        "未設定"
    } else {
        "publicまたは未設定"
    }
}

#[component]
fn BaseSkillRow(
    skill: BaseSkillCatalogEntry,
    busy: bool,
    mut selected: Signal<BTreeSet<String>>,
) -> Element {
    let id = skill.id.clone();
    let checked = selected.read().contains(&id);
    let license = skill.license.as_deref().unwrap_or("未記載").to_owned();
    let compatibility = compatibility_label(&skill);
    rsx! {
        label { class: "domain-list__item base-skills__row",
            input {
                r#type: "checkbox",
                checked,
                disabled: busy || skill.installed,
                onchange: move |event| {
                    if event.checked() { selected.write().insert(id.clone()); }
                    else { selected.write().remove(&id); }
                },
            }
            div {
                strong { {skill.name} }
                p { {skill.description} }
                p { class: "domain-meta", "{skill.provenance} @ {short_version(&skill.version)}" }
                p { class: "domain-meta",
                    "互換: {compatibility} · License: {license}"
                }
            }
        }
    }
}

#[allow(
    dead_code,
    reason = "Dioxus removes unused onboarding component bodies before shared route integration"
)]
fn short_version(version: &str) -> &str {
    version.get(..12).unwrap_or(version)
}

#[allow(
    dead_code,
    reason = "Dioxus removes unused onboarding component bodies before shared route integration"
)]
fn compatibility_label(skill: &BaseSkillCatalogEntry) -> String {
    skill
        .compatibility
        .iter()
        .map(|value| format!("{value:?}"))
        .collect::<Vec<_>>()
        .join(" / ")
}

#[cfg(test)]
mod tests {
    use super::short_version;

    #[test]
    fn provenance_version_is_shortened_without_panicking() {
        assert_eq!(short_version("1234567890abcdef"), "1234567890ab");
        assert_eq!(short_version("dev"), "dev");
    }
}
