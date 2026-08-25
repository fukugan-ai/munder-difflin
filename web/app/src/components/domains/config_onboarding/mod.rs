use dioxus::prelude::*;
use md_web_contracts::domains::config_onboarding::{
    AppCapability, Audience, CapabilityAvailability, CapabilitySupport, ChangeHomeRequest,
    ConfigPatch, CreateFloorRequest, FinishOnboardingRequest, FinishOnboardingResult,
    OnboardingPhase, OnboardingStep, ProviderKeyId, ProviderKeyWrite, PublicConfig,
    ReleaseRepository, SetAgentTokenCapRequest, ToolKind, ToolStatus, UpdateAction, UpdateStatus,
    WriteOnlyProviderKey,
};
use md_web_contracts::domains::memory_skills::LocalSkill;

const CONFIG_ONBOARDING_CSS: Asset = asset!("/assets/domains/config_onboarding.css");
const ARIA_STANDARD_SKILLS: &[&str] = &[
    "aria-orchestration",
    "graph-engineering",
    "project-documentation",
];
const IMPLEMENTER_STANDARD_SKILLS: &[&str] = &["local-development", "web-project-standards"];
const VERIFIER_STANDARD_SKILLS: &[&str] = &["perfectionist-reviewer"];

/// Complete configuration-domain surface. Integration may render its sections in
/// separate routes without changing the component contracts.
#[component]
pub(crate) fn ConfigOnboardingPanel(
    config: PublicConfig,
    tools: Vec<ToolStatus>,
    update: UpdateStatus,
    capabilities: Vec<CapabilitySupport>,
    release_repository: ReleaseRepository,
    on_patch: EventHandler<ConfigPatch>,
    on_finish: EventHandler<FinishOnboardingRequest>,
    #[props(default)] finish_pending: bool,
    #[props(default)] finish_result: Option<FinishOnboardingResult>,
    #[props(default)] finish_error: Option<String>,
    #[props(default)] base_skills: Vec<LocalSkill>,
    on_refresh_tools: EventHandler<()>,
    on_check_update: EventHandler<()>,
    on_open_release: EventHandler<String>,
    on_create_floor: EventHandler<CreateFloorRequest>,
    #[props(default)] on_provider_key: EventHandler<ProviderKeyWrite>,
    #[props(default)] on_change_home: EventHandler<ChangeHomeRequest>,
    #[props(default)] on_set_agent_token_cap: EventHandler<SetAgentTokenCapRequest>,
) -> Element {
    rsx! {
        document::Link { rel: "stylesheet", href: CONFIG_ONBOARDING_CSS }
        section { class: "config-domain", id: "config-onboarding", aria_label: "設定と初期設定",
            if config.onboarding_ready() {
                SettingsPanel {
                    config,
                    tools,
                    update,
                    capabilities,
                    release_repository,
                    on_patch,
                    on_refresh_tools,
                    on_check_update,
                    on_open_release,
                    on_create_floor,
                    on_provider_key,
                    on_change_home,
                    on_set_agent_token_cap,
                }
            } else {
                OnboardingPanel {
                    config,
                    tools,
                    on_finish,
                    on_refresh_tools,
                    finish_pending,
                    finish_result,
                    finish_error,
                    base_skills,
                }
            }
        }
    }
}

#[component]
fn OnboardingPanel(
    config: PublicConfig,
    tools: Vec<ToolStatus>,
    on_finish: EventHandler<FinishOnboardingRequest>,
    on_refresh_tools: EventHandler<()>,
    finish_pending: bool,
    finish_result: Option<FinishOnboardingResult>,
    finish_error: Option<String>,
    base_skills: Vec<LocalSkill>,
) -> Element {
    let initial_step = if matches!(
        config.onboarding_phase,
        OnboardingPhase::RepairRequired | OnboardingPhase::TeamStarting
    ) {
        OnboardingStep::Team
    } else {
        OnboardingStep::default()
    };
    let mut step = use_signal(|| initial_step);
    let mut audience = use_signal(|| config.audience);
    let mut home = use_signal(|| config.harness_home.as_deref().unwrap_or("~").to_string());
    let mut repositories = use_signal(|| config.registered_repos.join("\n"));
    let mut workspace_cwd = use_signal(|| {
        config
            .workspace_cwd
            .clone()
            .or_else(|| config.registered_repos.first().cloned())
            .unwrap_or_default()
    });
    let mut provider = use_signal(|| config.god_provider.clone());
    let mut model = use_signal(|| {
        config
            .god_model
            .clone()
            .or_else(|| {
                config
                    .god_provider
                    .recommended_aria_model()
                    .map(String::from)
            })
            .unwrap_or_default()
    });
    let mut base_skill_managed_id = use_signal(|| {
        base_skills
            .first()
            .map(|skill| skill.managed_id.clone())
            .unwrap_or_default()
    });
    let mut telemetry = use_signal(|| config.telemetry_enabled);
    use_effect(move || {
        if let Some(completed) = completed_step(finish_result.is_some()) {
            step.set(completed);
        }
    });
    let current = *step.read();
    let known_engine_missing = tools
        .iter()
        .any(|tool| tool.id == format!("engine:{}", provider.read().as_str()) && !tool.found);
    let home_is_blank = home.read().trim().is_empty();
    let model_is_blank = model.read().trim().is_empty();
    let repository_list = repository_lines(&repositories.read());
    let repositories_are_blank = repository_list.is_empty();
    let workspace_invalid = workspace_cwd.read().trim().is_empty()
        || !repository_list
            .iter()
            .any(|repository| repository == workspace_cwd.read().trim());
    let unresolved_standards = unresolved_team_standards(&base_skills);
    let next_disabled = matches!(current, OnboardingStep::Home) && home_is_blank
        || matches!(current, OnboardingStep::Orchestrator)
            && (known_engine_missing || model_is_blank)
        || matches!(current, OnboardingStep::Repositories)
            && (repositories_are_blank || workspace_invalid)
        || matches!(current, OnboardingStep::Team)
            && (base_skill_managed_id.read().is_empty() || !unresolved_standards.is_empty())
        || finish_pending;

    rsx! {
        div { class: "onboarding-card", "data-testid": "onboarding-wizard",
            header { class: "config-domain__heading",
                div {
                    h1 { "Munder Difflinを準備" }
                    p { "サーバーPCの作業場所と、最初のオーケストレーターを設定します。" }
                }
                span { class: "step-count", "{step_number(current)} / 8" }
            }

            if matches!(
                config.onboarding_phase,
                OnboardingPhase::RepairRequired | OnboardingPhase::TeamStarting
            ) {
                div { class: "inline-alert", role: "status",
                    strong { "初期チームを修復します" }
                    span { "保存済み設定を確認し、Aria・Implementer・Verifierの起動を再試行してください。" }
                }
            }

            div { class: "onboarding-card__body",
                match current {
                    OnboardingStep::Persona => rsx! {
                        fieldset { class: "choice-grid",
                            legend { "説明の詳しさ" }
                            button {
                                class: "choice-card",
                                class: if *audience.read() == Audience::Technical { "is-selected" },
                                r#type: "button",
                                aria_pressed: (*audience.read() == Audience::Technical).to_string(),
                                onclick: move |_| audience.set(Audience::Technical),
                                strong { "技術用語を使う" }
                                span { "CLI、モデル、worktreeなどをそのまま表示します。" }
                            }
                            button {
                                class: "choice-card",
                                class: if *audience.read() == Audience::NonTechnical { "is-selected" },
                                r#type: "button",
                                aria_pressed: (*audience.read() == Audience::NonTechnical).to_string(),
                                onclick: move |_| audience.set(Audience::NonTechnical),
                                strong { "分かりやすく説明" }
                                span { "略語を避けて、操作の意味を先に説明します。" }
                            }
                        }
                    },
                    OnboardingStep::Welcome => rsx! {
                        div { class: "explain-block",
                            h2 { "ローカル単独利用" }
                            p { "エージェント、Git、ファイル、設定はDioxus serverが動くPCで処理します。別PCのブラウザーは画面だけを受け取ります。" }
                            p { "ブラウザーを閉じても、明示的に停止するまでサーバー上の処理は継続します。" }
                        }
                    },
                    OnboardingStep::Home => rsx! {
                        label { class: "field-stack",
                            span { "サーバー上の作業フォルダー" }
                            input {
                                r#type: "text",
                                value: "{home}",
                                placeholder: "/home/user/HarnessAgents",
                                aria_invalid: home_is_blank.to_string(),
                                oninput: move |event| home.set(event.value()),
                            }
                            small { "このパスはブラウザー側ではなく、サーバーPC上の絶対パスとして検証されます。" }
                        }
                    },
                    OnboardingStep::Orchestrator => rsx! {
                        div { class: "explain-block",
                            h2 { "Ariaのエンジン" }
                            fieldset { class: "choice-grid choice-grid--compact",
                                legend { "プロバイダー" }
                                for (label, choice) in aria_provider_choices() {
                                    button {
                                        class: "choice-card choice-card--compact",
                                        class: if *provider.read() == choice { "is-selected" },
                                        r#type: "button",
                                        aria_pressed: (*provider.read() == choice).to_string(),
                                        onclick: move |_| {
                                            let recommended = choice
                                                .recommended_aria_model()
                                                .unwrap_or_default();
                                            provider.set(choice.clone());
                                            model.set(String::from(recommended));
                                        },
                                        strong { {label} }
                                        span { {choice.aria_command().unwrap_or_default()} }
                                    }
                                }
                            }
                            label { class: "field-stack",
                                span { "モデル" }
                                input {
                                    value: "{model}",
                                    aria_invalid: model_is_blank.to_string(),
                                    oninput: move |event| model.set(event.value()),
                                }
                            }
                            if known_engine_missing {
                                div { class: "inline-alert", role: "alert",
                                    strong { "このCLIはサーバーPCで見つかりません。" }
                                    button {
                                        class: "co-button co-button--secondary",
                                        r#type: "button",
                                        onclick: move |_| on_refresh_tools.call(()),
                                        "もう一度確認"
                                    }
                                }
                            }
                        }
                    },
                    OnboardingStep::Repositories => rsx! {
                        div { class: "explain-block",
                            h2 { "リポジトリ" }
                            label { class: "field-stack",
                                span { "Gitワークスペース（1行に1つ）" }
                                textarea {
                                    rows: "6",
                                    value: "{repositories}",
                                    placeholder: "/home/user/source/project",
                                    oninput: move |event| repositories.set(event.value()),
                                }
                                small { "サーバー上の許可root内にある既存Gitリポジトリだけを登録します。" }
                            }
                            label { class: "field-stack",
                                span { "チームを起動するワークスペース" }
                                input {
                                    value: "{workspace_cwd}",
                                    aria_invalid: workspace_invalid.to_string(),
                                    placeholder: "/home/user/source/project",
                                    oninput: move |event| workspace_cwd.set(event.value()),
                                }
                                small { "上の登録済みGitリポジトリから1つを指定します。Harness homeとは別のパスです。" }
                            }
                        }
                    },
                    OnboardingStep::Team => rsx! {
                        div { class: "explain-block",
                            h2 { "最小チームとベースskill" }
                            if base_skills.is_empty() {
                                div { class: "inline-alert", role: "alert",
                                    strong { "解決済みskillがありません。" }
                                    span { "記憶とスキル画面で必須skillを利用可能にしてください。" }
                                }
                            } else {
                                label { class: "field-stack",
                                    span { "全員へ追加するベースskill" }
                                    select {
                                        value: "{base_skill_managed_id}",
                                        onchange: move |event| base_skill_managed_id.set(event.value()),
                                        for skill in &base_skills {
                                            option {
                                                value: "{skill.managed_id}",
                                                "{skill.name} · {skill.scope:?}"
                                            }
                                        }
                                    }
                                }
                            }
                            div { class: "team-preview", aria_label: "初期チーム",
                                TeamPreviewCard { name: "Aria", role: "オーケストレーター", skills: ARIA_STANDARD_SKILLS }
                                TeamPreviewCard { name: "Implementer", role: "実装", skills: IMPLEMENTER_STANDARD_SKILLS }
                                TeamPreviewCard { name: "Verifier", role: "検証", skills: VERIFIER_STANDARD_SKILLS }
                            }
                            if !unresolved_standards.is_empty() {
                                div { class: "inline-alert", role: "alert",
                                    strong { "必須skillが未解決です" }
                                    span { {unresolved_standards.join(", ")} }
                                }
                            }
                        }
                    },
                    OnboardingStep::Reliability => rsx! {
                        div { class: "toggle-row",
                            div {
                                strong { "匿名の利用統計" }
                                p { "プロンプト、コード、パス、出力を含めません。" }
                            }
                            button {
                                class: "co-switch",
                                r#type: "button",
                                role: "switch",
                                aria_checked: telemetry.read().to_string(),
                                onclick: move |_| {
                                    let next = !*telemetry.read();
                                    telemetry.set(next);
                                },
                                if *telemetry.read() { "オン" } else { "オフ" }
                            }
                        }
                    },
                    OnboardingStep::Done => rsx! {
                        div { class: "explain-block", role: "status",
                            h2 { "準備完了" }
                            p { "設定をPostgreSQLへ保存しました。Ariaを起動してオフィスを開けます。" }
                        }
                    },
                }
            }

            footer { class: "onboarding-card__actions",
                if let Some(error) = finish_error.as_ref() {
                    p { class: "onboarding-save-error", role: "alert", {error.clone()} }
                }
                button {
                    class: "co-button co-button--secondary",
                    r#type: "button",
                    disabled: current == OnboardingStep::Persona,
                    onclick: move |_| {
                        let previous = step.read().previous();
                        step.set(previous);
                    },
                    "戻る"
                }
                button {
                    class: "co-button co-button--primary",
                    r#type: "button",
                    disabled: next_disabled || current == OnboardingStep::Done,
                    "data-ui-state": if finish_pending { "loading" } else { "default" },
                    onclick: move |_| {
                        let active = *step.read();
                        if active == OnboardingStep::Reliability {
                            on_finish.call(FinishOnboardingRequest {
                                expected_revision: config.revision,
                                audience: *audience.read(),
                                harness_home: home.read().trim().to_string(),
                                registered_repos: repository_lines(&repositories.read()),
                                workspace_cwd: workspace_cwd.read().trim().to_string(),
                                auto_mode: config.auto_mode,
                                god_provider: provider.read().clone(),
                                god_model: Some(model.read().trim().to_string()),
                                base_skill_managed_id: base_skill_managed_id.read().clone(),
                                telemetry_enabled: *telemetry.read(),
                            });
                        } else {
                            step.set(active.next());
                        }
                    },
                    if finish_pending { "保存中…" } else if current == OnboardingStep::Reliability { "設定を保存" } else { "次へ" }
                }
            }
        }
    }
}

#[component]
fn SettingsPanel(
    config: PublicConfig,
    tools: Vec<ToolStatus>,
    update: UpdateStatus,
    capabilities: Vec<CapabilitySupport>,
    release_repository: ReleaseRepository,
    on_patch: EventHandler<ConfigPatch>,
    on_refresh_tools: EventHandler<()>,
    on_check_update: EventHandler<()>,
    on_open_release: EventHandler<String>,
    on_create_floor: EventHandler<CreateFloorRequest>,
    on_provider_key: EventHandler<ProviderKeyWrite>,
    on_change_home: EventHandler<ChangeHomeRequest>,
    on_set_agent_token_cap: EventHandler<SetAgentTokenCapRequest>,
) -> Element {
    let missing_count = tools
        .iter()
        .filter(|tool| tool.blocks_recommended_setup())
        .count();
    let revision = config.revision;
    let auto_update = config.auto_update;
    let notifications = config.notifications;
    let telemetry_enabled = config.telemetry_enabled;
    let multi_floor = config.multi_floor;
    let harness_home = config.harness_home.clone().unwrap_or_default();

    rsx! {
        header { class: "config-domain__heading",
            div {
                h1 { "設定" }
                p { "この画面の変更はサーバー上のPostgreSQLへ保存されます。" }
            }
            span { class: "revision-chip", "rev {revision}" }
        }

        div { class: "settings-grid",
            section { class: "settings-section", aria_labelledby: "general-title",
                h2 { id: "general-title", "一般" }
                ToggleSetting {
                    label: "更新の確認",
                    detail: "forkのGitHub Releasesだけを確認します。",
                    enabled: auto_update,
                    on_toggle: move |_| on_patch.call(ConfigPatch {
                        expected_revision: revision,
                        auto_update: Some(!auto_update),
                        ..ConfigPatch::default()
                    }),
                }
                ToggleSetting {
                    label: "ブラウザー通知",
                    detail: "LANのHTTPではブラウザー制約により使えない場合があります。",
                    enabled: notifications,
                    on_toggle: move |_| on_patch.call(ConfigPatch {
                        expected_revision: revision,
                        notifications: Some(!notifications),
                        ..ConfigPatch::default()
                    }),
                }
                ToggleSetting {
                    label: "匿名の利用統計",
                    detail: "プロンプト、コード、パス、出力は送りません。",
                    enabled: telemetry_enabled,
                    on_toggle: move |_| on_patch.call(ConfigPatch {
                        expected_revision: revision,
                        telemetry_enabled: Some(!telemetry_enabled),
                        ..ConfigPatch::default()
                    }),
                }
                ToggleSetting {
                    label: "複数フロア",
                    detail: "floor別route/runtime namespaceが未接続のため、Web版では利用できません。",
                    enabled: multi_floor,
                    disabled: true,
                    on_toggle: move |_| {},
                }
            }

            PrerequisitesPanel { tools, missing_count, on_refresh: on_refresh_tools }
            ByokPanel { present: config.secrets.provider_keys, on_write: on_provider_key }
            RuntimeConfigPanel {
                revision,
                harness_home,
                on_change_home,
                on_set_agent_token_cap,
            }
            UpdatePanel {
                update,
                repository: release_repository,
                on_check: on_check_update,
                on_open_release,
            }
            LifecyclePanel { capabilities, multi_floor, on_create_floor }
        }
    }
}

#[component]
fn RuntimeConfigPanel(
    revision: i64,
    harness_home: String,
    on_change_home: EventHandler<ChangeHomeRequest>,
    on_set_agent_token_cap: EventHandler<SetAgentTokenCapRequest>,
) -> Element {
    let mut home = use_signal(|| harness_home);
    let mut agent_id = use_signal(String::new);
    let mut token_cap = use_signal(String::new);
    rsx! {
        section { class: "settings-section", aria_labelledby: "runtime-config-title",
            h2 { id: "runtime-config-title", "実行時設定" }
            label { class: "field-stack", span { "Harness home" }
                input { value: "{home}", oninput: move |event| home.set(event.value()) }
            }
            button { class: "co-button co-button--secondary", r#type: "button",
                onclick: move |_| on_change_home.call(ChangeHomeRequest {
                    expected_revision: revision,
                    harness_home: home.read().clone(),
                }),
                "作業場所を変更"
            }
            label { class: "field-stack", span { "Agent ID" }
                input { value: "{agent_id}", oninput: move |event| agent_id.set(event.value()) }
            }
            label { class: "field-stack", span { "Token cap（空欄で解除）" }
                input { r#type: "number", min: "1", value: "{token_cap}",
                    oninput: move |event| token_cap.set(event.value()) }
            }
            button { class: "co-button co-button--secondary", r#type: "button",
                onclick: move |_| on_set_agent_token_cap.call(SetAgentTokenCapRequest {
                    expected_revision: revision,
                    agent_id: agent_id.read().clone(),
                    token_cap: token_cap.read().parse().ok(),
                }),
                "Token capを保存"
            }
        }
    }
}

#[component]
fn ByokPanel(present: Vec<String>, on_write: EventHandler<ProviderKeyWrite>) -> Element {
    let mut provider = use_signal(String::new);
    let mut key = use_signal(String::new);
    let present_label = present.join(", ");
    rsx! {
        section { class: "settings-section", aria_labelledby: "byok-title",
            h2 { id: "byok-title", "プロバイダーキー（BYOK）" }
            p { "キー本文はブラウザーへ再表示せず、保存後は有無だけを表示します。" }
            if !present.is_empty() {
                p { "設定済み: {present_label}" }
            }
            label { class: "field-stack", span { "プロバイダーID" }
                input { value: "{provider}", oninput: move |event| provider.set(event.value()) }
            }
            label { class: "field-stack", span { "APIキー" }
                input { r#type: "password", value: "{key}", autocomplete: "off",
                    oninput: move |event| key.set(event.value()) }
            }
            button { class: "co-button co-button--primary", r#type: "button",
                onclick: move |_| {
                    let provider_value = provider.read().clone();
                    let key_value = key.read().clone();
                    if let (Some(provider), Some(secret)) = (
                        ProviderKeyId::new(provider_value),
                        WriteOnlyProviderKey::new(key_value),
                    ) {
                        on_write.call(ProviderKeyWrite {
                            provider,
                            key: secret,
                        });
                        key.set(String::new());
                    }
                },
                "書き込み専用で保存"
            }
        }
    }
}

#[component]
fn ToggleSetting(
    label: &'static str,
    detail: &'static str,
    enabled: bool,
    #[props(default)] disabled: bool,
    on_toggle: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "toggle-row",
            div {
                strong { {label} }
                p { {detail} }
            }
            button {
                class: "co-switch",
                r#type: "button",
                role: "switch",
                aria_checked: enabled.to_string(),
                aria_disabled: disabled.to_string(),
                disabled,
                onclick: move |_| on_toggle.call(()),
                if enabled { "オン" } else { "オフ" }
            }
        }
    }
}

#[component]
fn PrerequisitesPanel(
    tools: Vec<ToolStatus>,
    missing_count: usize,
    on_refresh: EventHandler<()>,
) -> Element {
    rsx! {
        section { class: "settings-section settings-section--wide", aria_labelledby: "tools-title",
            div { class: "section-heading",
                div {
                    h2 { id: "tools-title", "前提ツール" }
                    p { "サーバーPCを確認しています。不足（推奨）：{missing_count}件" }
                }
                button {
                    class: "co-button co-button--secondary",
                    r#type: "button",
                    onclick: move |_| on_refresh.call(()),
                    "再確認"
                }
            }
            div { class: "tool-list",
                for tool in tools {
                    article { class: "tool-row", "data-kind": tool_kind(tool.kind),
                        div { class: "tool-row__main",
                            strong { {tool.label} }
                            span { class: "status-chip", "data-state": if tool.found { "success" } else if tool.essential { "error" } else { "idle" },
                                if tool.found { "準備済み" } else if tool.essential { "不足" } else { "未設定" }
                            }
                            p { {tool.why_ja} }
                            if let Some(path) = tool.path {
                                code { {path} }
                            }
                        }
                        if !tool.found {
                            if let Some(command) = tool.install_command {
                                code { class: "install-command", {command} }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn UpdatePanel(
    update: UpdateStatus,
    repository: ReleaseRepository,
    on_check: EventHandler<()>,
    on_open_release: EventHandler<String>,
) -> Element {
    let action = update.action();
    let (headline, detail, release_url) = update_copy(&update, &repository);

    rsx! {
        section { class: "settings-section", aria_labelledby: "update-title",
            h2 { id: "update-title", "アップデート" }
            strong { {headline} }
            p { {detail} }
            button {
                class: "co-button co-button--primary",
                r#type: "button",
                disabled: action == UpdateAction::None,
                "data-ui-state": if action == UpdateAction::None { "loading" } else { "default" },
                onclick: move |_| match action {
                    UpdateAction::Check => on_check.call(()),
                    UpdateAction::OpenRelease => {
                        if let Some(url) = release_url.as_ref() {
                            on_open_release.call(url.clone());
                        }
                    }
                    UpdateAction::None => {}
                },
                match action {
                    UpdateAction::Check => "更新を確認",
                    UpdateAction::OpenRelease => "リリースを開く",
                    UpdateAction::None => "確認中…",
                }
            }
        }
    }
}

#[component]
fn LifecyclePanel(
    capabilities: Vec<CapabilitySupport>,
    multi_floor: bool,
    on_create_floor: EventHandler<CreateFloorRequest>,
) -> Element {
    let multi_floor_available = capabilities.iter().any(|support| {
        support.capability == AppCapability::MultiFloor
            && support.availability == CapabilityAvailability::Available
    });
    rsx! {
        section { class: "settings-section", aria_labelledby: "lifecycle-title",
            h2 { id: "lifecycle-title", "Web版の動作" }
            ul { class: "capability-list",
                for support in capabilities {
                    li {
                        span { "{capability_label(support.capability)}" }
                        span { class: "capability-state", "data-state": availability_state(support.availability),
                            "{availability_label(support.availability)}"
                        }
                        small { {support.detail_ja} }
                    }
                }
            }
            button {
                class: "co-button co-button--primary",
                r#type: "button",
                disabled: !multi_floor_available || !multi_floor,
                onclick: move |_| on_create_floor.call(CreateFloorRequest::default()),
                "新しいフロアを開く"
            }
        }
    }
}

#[component]
fn TeamPreviewCard(
    name: &'static str,
    role: &'static str,
    skills: &'static [&'static str],
) -> Element {
    rsx! {
        article { class: "team-preview__card",
            h3 { {name} }
            p { {role} }
            ul {
                for skill in skills {
                    li { code { {*skill} } }
                }
            }
        }
    }
}

const fn step_number(step: OnboardingStep) -> u8 {
    match step {
        OnboardingStep::Persona => 1,
        OnboardingStep::Welcome => 2,
        OnboardingStep::Home => 3,
        OnboardingStep::Orchestrator => 4,
        OnboardingStep::Repositories => 5,
        OnboardingStep::Team => 6,
        OnboardingStep::Reliability => 7,
        OnboardingStep::Done => 8,
    }
}

fn aria_provider_choices() -> [(
    &'static str,
    md_web_contracts::domains::config_onboarding::AgentProvider,
); 2] {
    use md_web_contracts::domains::config_onboarding::AgentProvider;

    [
        ("Claude Code", AgentProvider::Claude),
        ("Codex", AgentProvider::Codex),
    ]
}

fn unresolved_team_standards(resolved: &[LocalSkill]) -> Vec<String> {
    ARIA_STANDARD_SKILLS
        .iter()
        .chain(IMPLEMENTER_STANDARD_SKILLS)
        .chain(VERIFIER_STANDARD_SKILLS)
        .filter(|required| {
            !resolved
                .iter()
                .any(|skill| skill.name.eq_ignore_ascii_case(required))
        })
        .map(|required| String::from(*required))
        .collect()
}

fn repository_lines(value: &str) -> Vec<String> {
    let mut repositories = Vec::new();
    for line in value.lines() {
        let path = line.trim();
        if !path.is_empty() && !repositories.iter().any(|existing| existing == path) {
            repositories.push(String::from(path));
        }
    }
    repositories
}

const fn completed_step(has_persisted_receipt: bool) -> Option<OnboardingStep> {
    if has_persisted_receipt {
        Some(OnboardingStep::Done)
    } else {
        None
    }
}

const fn tool_kind(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Prerequisite => "prerequisite",
        ToolKind::Memory => "memory",
        ToolKind::Engine => "engine",
    }
}

fn update_copy(
    status: &UpdateStatus,
    repository: &ReleaseRepository,
) -> (String, String, Option<String>) {
    match status {
        UpdateStatus::Idle => (
            String::from("まだ確認していません"),
            format!("更新元：{}", repository.slug()),
            None,
        ),
        UpdateStatus::Checking => (
            String::from("確認中"),
            format!("{} の最新リリースを確認しています。", repository.slug()),
            None,
        ),
        UpdateStatus::Current => (
            String::from("最新版です"),
            format!("更新元：{}", repository.slug()),
            None,
        ),
        UpdateStatus::Available {
            version,
            release_url,
            ..
        } => (
            format!("v{version}を利用できます"),
            String::from("Web版は自動でプロセスを置き換えません。手動更新の案内を開きます。"),
            Some(release_url.clone()),
        ),
        UpdateStatus::Error { message_ja } => {
            (String::from("確認に失敗しました"), message_ja.clone(), None)
        }
    }
}

const fn capability_label(capability: AppCapability) -> &'static str {
    match capability {
        AppCapability::WindowBounds => "ウィンドウ位置",
        AppCapability::LoginItem => "ログイン時起動",
        AppCapability::NativeAutoUpdate => "ネイティブ自動更新",
        AppCapability::OsSettingsDeepLink => "OS設定リンク",
        AppCapability::NativeDesktopNotification => "デスクトップ通知",
        AppCapability::KeepDisplayAwake => "画面スリープ防止",
        AppCapability::MultiFloor => "複数フロア",
        AppCapability::ExternalLinks => "外部リンク",
    }
}

const fn availability_state(availability: CapabilityAvailability) -> &'static str {
    match availability {
        CapabilityAvailability::Available => "success",
        CapabilityAvailability::BrowserRestricted => "warning",
        CapabilityAvailability::ExternalSetup => "idle",
        CapabilityAvailability::NotApplicable => "na",
    }
}

const fn availability_label(availability: CapabilityAvailability) -> &'static str {
    match availability {
        CapabilityAvailability::Available => "利用可能",
        CapabilityAvailability::BrowserRestricted => "ブラウザー依存",
        CapabilityAvailability::ExternalSetup => "外部設定",
        CapabilityAvailability::NotApplicable => "N/A",
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::config_onboarding::{
        AppCapability, CapabilityAvailability, OnboardingStep, ReleaseRepository, UpdateStatus,
    };

    use super::{
        aria_provider_choices, availability_label, capability_label, completed_step,
        repository_lines, step_number, unresolved_team_standards, update_copy,
    };

    #[test]
    fn done_is_last_onboarding_step() {
        assert_eq!(step_number(OnboardingStep::Done), 8);
    }

    #[test]
    fn native_update_is_named_explicitly() {
        assert_eq!(
            capability_label(AppCapability::NativeAutoUpdate),
            "ネイティブ自動更新"
        );
    }

    #[test]
    fn not_applicable_is_rendered_as_na() {
        assert_eq!(
            availability_label(CapabilityAvailability::NotApplicable),
            "N/A"
        );
    }

    #[test]
    fn idle_update_names_fork() {
        let repository = ReleaseRepository {
            owner: String::from("fukugan-ai"),
            name: String::from("munder-difflin"),
        };
        let (_, detail, _) = update_copy(&UpdateStatus::Idle, &repository);

        assert!(detail.contains("fukugan-ai/munder-difflin"));
    }

    #[test]
    fn repository_lines_trim_and_deduplicate_paths() {
        assert_eq!(
            repository_lines(" /srv/repo\n/srv/repo\n/srv/other "),
            vec![String::from("/srv/repo"), String::from("/srv/other")]
        );
    }

    #[test]
    fn done_requires_a_persisted_receipt() {
        assert_eq!(completed_step(false), None);
        assert_eq!(completed_step(true), Some(OnboardingStep::Done));
    }

    #[test]
    fn aria_selector_only_offers_supported_profiles() {
        assert!(
            aria_provider_choices()
                .iter()
                .all(|(_, provider)| provider.aria_command().is_some()
                    && provider.recommended_aria_model().is_some())
        );
    }

    #[test]
    fn missing_mandatory_standards_are_reported() {
        assert_eq!(unresolved_team_standards(&[]).len(), 6);
    }
}
