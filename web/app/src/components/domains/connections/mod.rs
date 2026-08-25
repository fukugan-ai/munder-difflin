use dioxus::prelude::*;
use md_web_contracts::domains::connections::{
    CliAuthPhase, CliAuthProvider, CliAuthSnapshot, CliAuthView, ConnectionsSnapshot,
    ContextAction, ContextRule, ContextTriggerConfig, IntegrationAuthType, IntegrationKind,
    IntegrationUpsert, MissionKind, OneTimeSecret, RuntimeStatus, ScheduledMission,
    TriggerDecision, TriggerMode, TriggerSource,
};

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectionUiAction {
    Refresh,
    SaveSlackSigningSecret(String),
    SaveSlackBotToken(String),
    SetSlackEnabled(bool),
    SetSlackProactivePosting(bool),
    SetSlackEndpoint {
        channel_id: String,
        port: u16,
    },
    StartSlack,
    StopSlack,
    BeginIntegrationFromTemplate(String),
    ProbeIntegration(String),
    RemoveIntegration(String),
    SaveIntegration {
        request: IntegrationUpsert,
        secret: String,
    },
    AddWebhook,
    RotateWebhookSecret(String),
    SetWebhookEnabled {
        id: String,
        enabled: bool,
    },
    RemoveWebhook(String),
    StartWebhookListener,
    StopWebhookListener,
    SetContextEnabled {
        action: ContextAction,
        enabled: bool,
    },
    SetContext(ContextTriggerConfig),
    SetOrganisationEnabled(bool),
    SaveOrganisationKey(String),
    SetOrganisationMode(TriggerMode),
    DecideHistory {
        id: String,
        decision: TriggerDecision,
    },
    ClearHistory(TriggerSource),
    SetMissionEnabled {
        id: String,
        enabled: bool,
    },
    RemoveMission(String),
    UpsertMission(ScheduledMission),
    StartBroker,
    StopBroker,
    StartCliAuth(CliAuthProvider),
    CancelCliAuth {
        provider: CliAuthProvider,
        generation: u64,
    },
    RefreshCliAuth,
    SubmitCliAuthCode {
        provider: CliAuthProvider,
        generation: u64,
        code: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelTab {
    Connections,
    Triggers,
}

#[component]
pub fn ConnectionsPanel(
    snapshot: ConnectionsSnapshot,
    cli_auth: CliAuthSnapshot,
    one_time_secret: Option<OneTimeSecret>,
    on_action: EventHandler<ConnectionUiAction>,
) -> Element {
    let mut tab = use_signal(|| PanelTab::Connections);
    let mut signing_secret = use_signal(String::new);
    let mut bot_token = use_signal(String::new);
    let mut slack_channel = use_signal(|| snapshot.slack.channel_id.clone().unwrap_or_default());
    let mut slack_port = use_signal(|| snapshot.slack.port.to_string());
    let mut editing_mission = use_signal(|| None::<ScheduledMission>);

    let ConnectionsSnapshot {
        slack,
        webhook_listener,
        integrations,
        integration_templates,
        webhooks,
        context,
        organisation,
        trigger_history,
        missions,
        broker,
    } = snapshot;

    let selected_tab = tab();
    let slack_running = matches!(slack.listener.state, RuntimeStatus::Running);
    let webhook_running = matches!(webhook_listener.state, RuntimeStatus::Running);
    let has_enabled_webhook = webhooks
        .iter()
        .any(|webhook| webhook.enabled && webhook.has_secret);
    let has_enabled_integration = integrations.iter().any(|item| item.enabled);

    rsx! {
        section { class: "connections-domain", aria_labelledby: "connections-title",
            header { class: "connections-domain__heading",
                div {
                    h1 { id: "connections-title", "外部連携とトリガー" }
                    p { "ローカルのAIチームが、外部サービスや定期実行とつながる入口です。" }
                }
                button {
                    class: "connection-button connection-button--quiet",
                    r#type: "button",
                    onclick: move |_| on_action.call(ConnectionUiAction::Refresh),
                    "再読み込み"
                }
            }

            nav { class: "connection-tabs", aria_label: "接続設定",
                button {
                    class: if selected_tab == PanelTab::Connections { "connection-tab is-active" } else { "connection-tab" },
                    r#type: "button",
                    aria_selected: selected_tab == PanelTab::Connections,
                    onclick: move |_| tab.set(PanelTab::Connections),
                    "接続"
                }
                button {
                    class: if selected_tab == PanelTab::Triggers { "connection-tab is-active" } else { "connection-tab" },
                    r#type: "button",
                    aria_selected: selected_tab == PanelTab::Triggers,
                    onclick: move |_| tab.set(PanelTab::Triggers),
                    "トリガー"
                }
            }

            if let Some(secret) = one_time_secret {
                aside { class: "one-time-secret", role: "status",
                    strong { "新しいシークレット（今回だけ表示）" }
                    code { {secret.reveal_once()} }
                    p { "コピー後は再表示できません。" }
                }
            }

            if selected_tab == PanelTab::Connections {
                div { class: "connection-stack", "data-testid": "connections-panel",
                    section { class: "connection-card", aria_labelledby: "integrations-title",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "integrations-title", "外部連携" }
                                p { "保存済みの秘密情報は画面へ戻しません。" }
                            }
                        }
                        if integrations.is_empty() {
                            p { class: "connection-empty", "外部連携はまだありません。" }
                        } else {
                            ul { class: "connection-list",
                                for integration in integrations.iter() {
                                    {
                                        let probe_id = integration.id.clone();
                                        let remove_id = integration.id.clone();
                                        rsx! {
                                            li { class: "connection-row",
                                                div { class: "connection-row__main",
                                                    strong { {integration.label.clone()} }
                                                    code { {integration.base_url.clone()} }
                                                    span { class: if integration.enabled && (!integration.auth_type.needs_secret() || integration.has_secret) { "connection-status is-ready" } else { "connection-status" },
                                                        if !integration.enabled { "停止中" }
                                                        else if integration.auth_type.needs_secret() && !integration.has_secret { "シークレット未設定" }
                                                        else { "利用可能" }
                                                    }
                                                }
                                                div { class: "connection-actions",
                                                    button {
                                                        class: "connection-button connection-button--quiet",
                                                        r#type: "button",
                                                        onclick: move |_| on_action.call(ConnectionUiAction::ProbeIntegration(probe_id.clone())),
                                                        "接続テスト"
                                                    }
                                                    button {
                                                        class: "connection-button connection-button--danger",
                                                        r#type: "button",
                                                        onclick: move |_| on_action.call(ConnectionUiAction::RemoveIntegration(remove_id.clone())),
                                                        "削除"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "template-grid",
                            for template in integration_templates {
                                {
                                    let template_id = template.id_suggestion.clone();
                                    rsx! {
                                        button {
                                            class: "template-choice",
                                            r#type: "button",
                                            onclick: move |_| on_action.call(ConnectionUiAction::BeginIntegrationFromTemplate(template_id.clone())),
                                            strong { {template.label} }
                                            span { {template.help} }
                                        }
                                    }
                                }
                            }
                        }
                        IntegrationEditor { on_action }
                    }

                    section { class: "connection-card", aria_labelledby: "broker-title",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "broker-title", "ローカル連携ブローカー" }
                                p { "ワーカーへ上流APIキーを渡さず、127.0.0.1だけで中継します。" }
                            }
                            span { class: if matches!(broker.state, RuntimeStatus::Running) { "connection-status is-ready" } else { "connection-status" },
                                if matches!(broker.state, RuntimeStatus::Running) { "待受中" } else { "停止中" }
                            }
                        }
                        div { class: "connection-actions",
                            button {
                                class: "connection-button connection-button--primary",
                                r#type: "button",
                                disabled: matches!(broker.state, RuntimeStatus::Running) || !has_enabled_integration,
                                onclick: move |_| on_action.call(ConnectionUiAction::StartBroker),
                                "ブローカー開始"
                            }
                            button {
                                class: "connection-button connection-button--quiet",
                                r#type: "button",
                                disabled: !matches!(broker.state, RuntimeStatus::Running),
                                onclick: move |_| on_action.call(ConnectionUiAction::StopBroker),
                                "停止"
                            }
                            if let Some(url) = broker.public_url { code { class: "connection-url", {url} } }
                        }
                    }

                    section { class: "connection-card", aria_labelledby: "cli-auth-title", "data-testid": "cli-auth-card",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "cli-auth-title", "AI CLIアカウント" }
                                p { "ブラウザーで公式の認証ページを開き、CLIの接続状態だけを確認します。" }
                            }
                            button {
                                class: "connection-button connection-button--quiet",
                                r#type: "button",
                                onclick: move |_| on_action.call(ConnectionUiAction::RefreshCliAuth),
                                "状態を更新"
                            }
                        }
                        div { class: "connection-list", aria_live: "polite",
                            for provider in [CliAuthProvider::Codex, CliAuthProvider::Claude] {
                                if let Some(view) = cli_auth.providers.iter().find(|view| view.provider == provider) {
                                    ProviderAuthRow { view: view.clone(), on_action }
                                }
                            }
                        }
                    }

                    section { class: "connection-card", aria_labelledby: "slack-title", "data-testid": "slack-card",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "slack-title", "Slack" }
                                p { "メンションとスレッド返信をMichaelのキューへ渡します。" }
                            }
                            span { class: if slack_running { "connection-status is-ready" } else { "connection-status" },
                                if slack_running { "接続済み" } else { "停止中" }
                            }
                        }
                        div { class: "connection-form-grid",
                            label { class: "connection-field",
                                span { "署名シークレット" }
                                input {
                                    r#type: "password",
                                    autocomplete: "off",
                                    value: "{signing_secret}",
                                    placeholder: if slack.has_signing_secret { "保存済み — 変更時だけ入力" } else { "Signing Secret" },
                                    oninput: move |event| signing_secret.set(event.value()),
                                }
                            }
                            button {
                                class: "connection-button",
                                r#type: "button",
                                disabled: signing_secret().trim().is_empty(),
                                onclick: move |_| {
                                    let value = signing_secret();
                                    if !value.trim().is_empty() {
                                        on_action.call(ConnectionUiAction::SaveSlackSigningSecret(value));
                                        signing_secret.set(String::new());
                                    }
                                },
                                "保存"
                            }
                            label { class: "connection-field",
                                span { "Botトークン" }
                                input {
                                    r#type: "password",
                                    autocomplete: "off",
                                    value: "{bot_token}",
                                    placeholder: if slack.has_bot_token { "保存済み — 変更時だけ入力" } else { "xoxb-…" },
                                    oninput: move |event| bot_token.set(event.value()),
                                }
                            }
                            button {
                                class: "connection-button",
                                r#type: "button",
                                disabled: bot_token().trim().is_empty(),
                                onclick: move |_| {
                                    let value = bot_token();
                                    if !value.trim().is_empty() {
                                        on_action.call(ConnectionUiAction::SaveSlackBotToken(value));
                                        bot_token.set(String::new());
                                    }
                                },
                                "保存"
                            }
                        }
                        div { class: "connection-toggle-row",
                            ToggleButton {
                                label: "Slack連携",
                                enabled: slack.enabled,
                                on_toggle: move |enabled| on_action.call(ConnectionUiAction::SetSlackEnabled(enabled)),
                            }
                            ToggleButton {
                                label: "アプリからの自発投稿",
                                enabled: slack.proactive_posting,
                                on_toggle: move |enabled| on_action.call(ConnectionUiAction::SetSlackProactivePosting(enabled)),
                            }
                        }
                        div { class: "connection-form-grid connection-form-grid--wide",
                            label { class: "connection-field",
                                span { "チャンネルID（任意）" }
                                input {
                                    value: "{slack_channel}",
                                    placeholder: "C0123456789",
                                    oninput: move |event| slack_channel.set(event.value()),
                                }
                            }
                            label { class: "connection-field",
                                span { "待受ポート" }
                                input {
                                    r#type: "number",
                                    min: "1",
                                    max: "65535",
                                    value: "{slack_port}",
                                    oninput: move |event| slack_port.set(event.value()),
                                }
                            }
                            button {
                                class: "connection-button",
                                r#type: "button",
                                disabled: slack_port().parse::<u16>().is_err(),
                                onclick: move |_| {
                                    if let Ok(port) = slack_port().parse::<u16>() {
                                        on_action.call(ConnectionUiAction::SetSlackEndpoint {
                                            channel_id: slack_channel(), port,
                                        });
                                    }
                                },
                                "接続先を保存"
                            }
                        }
                        div { class: "connection-actions",
                            button {
                                class: "connection-button connection-button--primary",
                                r#type: "button",
                                disabled: slack_running || !slack.enabled || !slack.has_signing_secret,
                                onclick: move |_| on_action.call(ConnectionUiAction::StartSlack),
                                "開始"
                            }
                            button {
                                class: "connection-button connection-button--quiet",
                                r#type: "button",
                                disabled: !slack_running,
                                onclick: move |_| on_action.call(ConnectionUiAction::StopSlack),
                                "停止"
                            }
                            if let Some(url) = slack.listener.public_url {
                                code { class: "connection-url", {url} }
                            }
                        }
                    }

                    section { class: "connection-card", aria_labelledby: "webhook-title", "data-testid": "webhooks-card",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "webhook-title", "Webhook" }
                                p { "複数の送信元を1つの待受サーバーで受け付けます。" }
                            }
                            span { class: if webhook_running { "connection-status is-ready" } else { "connection-status" },
                                if webhook_running { "待受中" } else { "停止中" }
                            }
                        }
                        div { class: "connection-actions",
                            button {
                                class: "connection-button",
                                r#type: "button",
                                onclick: move |_| on_action.call(ConnectionUiAction::AddWebhook),
                                "Webhookを追加"
                            }
                            button {
                                class: "connection-button connection-button--primary",
                                r#type: "button",
                                disabled: webhook_running || !has_enabled_webhook,
                                onclick: move |_| on_action.call(ConnectionUiAction::StartWebhookListener),
                                "待受開始"
                            }
                            button {
                                class: "connection-button connection-button--quiet",
                                r#type: "button",
                                disabled: !webhook_running,
                                onclick: move |_| on_action.call(ConnectionUiAction::StopWebhookListener),
                                "停止"
                            }
                        }
                        if webhooks.is_empty() {
                            p { class: "connection-empty", "Webhookはまだありません。" }
                        } else {
                            ul { class: "connection-list",
                                for webhook in webhooks {
                                    {
                                        let toggle_id = webhook.id.clone();
                                        let rotate_id = webhook.id.clone();
                                        let remove_id = webhook.id.clone();
                                        let next_enabled = !webhook.enabled;
                                        rsx! {
                                            li { class: "connection-row connection-row--stacked",
                                                div { class: "connection-row__main",
                                                    strong { {webhook.name} }
                                                    if let Some(url) = webhook.endpoint_url { code { {url} } }
                                                    span { class: "connection-status",
                                                        if webhook.enabled { "有効" } else { "無効" }
                                                        if !webhook.has_secret { " · シークレット未設定" }
                                                    }
                                                }
                                                div { class: "connection-actions",
                                                    button {
                                                        class: "connection-button connection-button--quiet",
                                                        r#type: "button",
                                                        disabled: next_enabled && !webhook.has_secret,
                                                        onclick: move |_| on_action.call(ConnectionUiAction::SetWebhookEnabled { id: toggle_id.clone(), enabled: next_enabled }),
                                                        if next_enabled { "有効化" } else { "無効化" }
                                                    }
                                                    button {
                                                        class: "connection-button connection-button--quiet",
                                                        r#type: "button",
                                                        onclick: move |_| on_action.call(ConnectionUiAction::RotateWebhookSecret(rotate_id.clone())),
                                                        "シークレット更新"
                                                    }
                                                    button {
                                                        class: "connection-button connection-button--danger",
                                                        r#type: "button",
                                                        onclick: move |_| on_action.call(ConnectionUiAction::RemoveWebhook(remove_id.clone())),
                                                        "削除"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "connection-stack", "data-testid": "triggers-panel",
                    section { class: "connection-card", aria_labelledby: "context-trigger-title",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "context-trigger-title", "コンテキスト" }
                                p { "使用量と経過時間の両方を満たしたときだけ実行します。" }
                            }
                        }
                        div { class: "connection-toggle-row",
                            ToggleButton {
                                label: "自動コンパクト",
                                enabled: context.compact.enabled,
                                on_toggle: move |enabled| on_action.call(ConnectionUiAction::SetContextEnabled { action: ContextAction::Compact, enabled }),
                            }
                            ToggleButton {
                                label: "自動クリア",
                                enabled: context.clear.enabled,
                                on_toggle: move |enabled| on_action.call(ConnectionUiAction::SetContextEnabled { action: ContextAction::Clear, enabled }),
                            }
                        }
                        ContextEditor { context, on_action }
                    }

                    section { class: "connection-card", aria_labelledby: "missions-title",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "missions-title", "スケジュール" }
                                p { "一定間隔または曜日・時刻でプロンプトを実行します。" }
                            }
                            button {
                                class: "connection-button",
                                r#type: "button",
                                onclick: move |_| editing_mission.set(Some(default_mission())),
                                "追加"
                            }
                        }
                        if let Some(mission) = editing_mission() {
                            MissionEditor {
                                mission,
                                on_cancel: move |_| editing_mission.set(None),
                                on_save: move |mission| {
                                    on_action.call(ConnectionUiAction::UpsertMission(mission));
                                    editing_mission.set(None);
                                },
                            }
                        }
                        if missions.is_empty() {
                            p { class: "connection-empty", "スケジュールはありません。" }
                        } else {
                            ul { class: "connection-list",
                                for mission in missions {
                                    {
                                        let toggle_id = mission.id.clone();
                                        let remove_id = mission.id.clone();
                                        let edit_mission = mission.clone();
                                        let next_enabled = !mission.enabled;
                                        rsx! {
                                            li { class: "connection-row",
                                                div { class: "connection-row__main",
                                                    strong { {mission.label} }
                                                    span { "→ {mission.to} · {format_duration(mission.interval_ms)}" }
                                                }
                                                div { class: "connection-actions",
                                                    button {
                                                        class: "connection-button connection-button--quiet",
                                                        r#type: "button",
                                                        onclick: move |_| editing_mission.set(Some(edit_mission.clone())),
                                                        "編集"
                                                    }
                                                    button {
                                                        class: "connection-button connection-button--quiet",
                                                        r#type: "button",
                                                        onclick: move |_| on_action.call(ConnectionUiAction::SetMissionEnabled { id: toggle_id.clone(), enabled: next_enabled }),
                                                        if next_enabled { "オン" } else { "オフ" }
                                                    }
                                                    button {
                                                        class: "connection-button connection-button--danger",
                                                        r#type: "button",
                                                        onclick: move |_| on_action.call(ConnectionUiAction::RemoveMission(remove_id.clone())),
                                                        "削除"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    section { class: "connection-card", aria_labelledby: "organisation-title",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "organisation-title", "組織" }
                                p { "APIキーは書き込み専用で保存され、画面へ戻りません。" }
                            }
                            ToggleButton {
                                label: "組織トリガー",
                                enabled: organisation.enabled,
                                on_toggle: move |enabled| on_action.call(ConnectionUiAction::SetOrganisationEnabled(enabled)),
                            }
                        }
                        p { class: "connection-note",
                            if organisation.has_api_key { "APIキーは保存済みです。" } else { "APIキーは未設定です。" }
                        }
                        OrganisationEditor { mode: organisation.mode, has_key: organisation.has_api_key, on_action }
                    }

                    section { class: "connection-card", aria_labelledby: "history-title",
                        div { class: "connection-card__heading",
                            div {
                                h2 { id: "history-title", "トリガー履歴" }
                                p { "受信内容と承認結果を新しい順で表示します。" }
                            }
                            div { class: "connection-actions",
                                button {
                                    class: "connection-button connection-button--danger",
                                    r#type: "button",
                                    onclick: move |_| on_action.call(ConnectionUiAction::ClearHistory(TriggerSource::Webhook)),
                                    "Webhook履歴を削除"
                                }
                            }
                        }
                        if trigger_history.is_empty() {
                            p { class: "connection-empty", "履歴はありません。" }
                        } else {
                            ol { class: "trigger-history",
                                for entry in trigger_history {
                                    {
                                        let approve_id = entry.id.clone();
                                        let reject_id = entry.id.clone();
                                        let pending = matches!(entry.decision, Some(TriggerDecision::Pending));
                                        rsx! {
                                            li {
                                                div { class: "trigger-history__meta",
                                                    strong { {entry.source_name} }
                                                    span { {entry.peer} }
                                                }
                                                p { {entry.body} }
                                                if pending {
                                                    div { class: "connection-actions",
                                                        button {
                                                            class: "connection-button connection-button--primary",
                                                            r#type: "button",
                                                            onclick: move |_| on_action.call(ConnectionUiAction::DecideHistory { id: approve_id.clone(), decision: TriggerDecision::Approved }),
                                                            "承認"
                                                        }
                                                        button {
                                                            class: "connection-button connection-button--danger",
                                                            r#type: "button",
                                                            onclick: move |_| on_action.call(ConnectionUiAction::DecideHistory { id: reject_id.clone(), decision: TriggerDecision::Rejected }),
                                                            "拒否"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ToggleButton(label: &'static str, enabled: bool, on_toggle: EventHandler<bool>) -> Element {
    rsx! {
        button {
            class: if enabled { "connection-toggle is-on" } else { "connection-toggle" },
            r#type: "button",
            role: "switch",
            aria_checked: enabled,
            onclick: move |_| on_toggle.call(!enabled),
            span { {label} }
            strong { if enabled { "オン" } else { "オフ" } }
        }
    }
}

#[component]
fn ProviderAuthRow(view: CliAuthView, on_action: EventHandler<ConnectionUiAction>) -> Element {
    let mut input_code = use_signal(String::new);
    let provider = view.provider;
    let generation = view.generation;
    let primary_control_id = match provider {
        CliAuthProvider::Codex => "cli-auth-codex-primary",
        CliAuthProvider::Claude => "cli-auth-claude-primary",
    };
    let active = matches!(
        view.phase,
        CliAuthPhase::Starting | CliAuthPhase::AwaitingUser | CliAuthPhase::Verifying
    );
    let connected = view.phase == CliAuthPhase::Connected;
    let failed = matches!(
        view.phase,
        CliAuthPhase::Failed | CliAuthPhase::Cancelled | CliAuthPhase::TimedOut
    );
    let status_class = if connected {
        "connection-status is-ready"
    } else {
        "connection-status"
    };
    let (button_label, state) = auth_button_presentation(view.phase);

    rsx! {
        article { class: "connection-row connection-row--stacked cli-auth-row", "data-provider": "{provider.label()}",
            div { class: "connection-row__main",
                strong { {provider.label()} }
                span { class: status_class, {view.detail_ja.clone()} }
            }
            if let Some(uri) = view.verification_uri.clone() {
                div { class: "cli-auth-prompt",
                    a {
                        class: "connection-button connection-button--primary",
                        href: uri,
                        target: "_blank",
                        rel: "noopener noreferrer",
                        "サインインを開く"
                    }
                    if let Some(code) = view.user_code.clone() {
                        code { class: "cli-auth-code", {code.clone()} }
                        button {
                            class: "connection-button connection-button--quiet",
                            r#type: "button",
                            onclick: move |_| {
                                let script = format!("navigator.clipboard.writeText({code:?})");
                                document::eval(&script);
                            },
                            "コードをコピー"
                        }
                    }
                }
            }
            if view.accepts_code_input {
                div { class: "connection-form-grid cli-auth-input",
                    label { class: "connection-field",
                        span { "CLIが要求したコード" }
                        input {
                            r#type: "text",
                            autocomplete: "one-time-code",
                            value: "{input_code}",
                            oninput: move |event| input_code.set(event.value()),
                        }
                    }
                    button {
                        class: "connection-button connection-button--primary",
                        r#type: "button",
                        disabled: input_code().trim().is_empty(),
                        onclick: move |_| on_action.call(ConnectionUiAction::SubmitCliAuthCode {
                            provider,
                            generation,
                            code: input_code().trim().to_owned(),
                        }),
                        "コードを送信"
                    }
                }
            }
            div { class: "connection-actions",
                button {
                    id: primary_control_id,
                    class: "connection-button connection-button--primary",
                    r#type: "button",
                    disabled: active || connected || view.phase == CliAuthPhase::NotInstalled,
                    "data-ui-state": state,
                    onclick: move |_| on_action.call(ConnectionUiAction::StartCliAuth(provider)),
                    {button_label}
                }
                if view.can_cancel {
                    button {
                        class: "connection-button connection-button--quiet",
                        r#type: "button",
                        onclick: move |_| {
                            on_action.call(ConnectionUiAction::CancelCliAuth {
                                provider,
                                generation,
                            });
                            let script = format!(
                                "setTimeout(() => document.getElementById({primary_control_id:?})?.focus(), 800)"
                            );
                            document::eval(&script);
                        },
                        "キャンセル"
                    }
                }
            }
            if failed {
                p { class: "connection-note", role: "alert", "もう一度試すか、CLIのインストール状態を確認してください。" }
            }
        }
    }
}

const fn auth_button_presentation(phase: CliAuthPhase) -> (&'static str, &'static str) {
    match phase {
        CliAuthPhase::NotInstalled => ("CLI未検出", "default"),
        CliAuthPhase::Starting => ("開始中…", "loading"),
        CliAuthPhase::AwaitingUser => ("認証待ち", "loading"),
        CliAuthPhase::Verifying => ("確認中…", "loading"),
        CliAuthPhase::Connected => ("接続済み", "success"),
        CliAuthPhase::Failed | CliAuthPhase::Cancelled | CliAuthPhase::TimedOut => {
            ("再試行", "error")
        }
        CliAuthPhase::SignedOut | CliAuthPhase::StatusUnknown => ("接続", "default"),
    }
}

#[component]
fn IntegrationEditor(on_action: EventHandler<ConnectionUiAction>) -> Element {
    let mut id = use_signal(String::new);
    let mut label = use_signal(String::new);
    let mut base_url = use_signal(String::new);
    let mut auth = use_signal(|| String::from("bearer"));
    let mut auth_header = use_signal(String::new);
    let mut secret = use_signal(String::new);
    let mut enabled = use_signal(|| true);
    let valid =
        !id().trim().is_empty() && !label().trim().is_empty() && !base_url().trim().is_empty();
    rsx! {
        form { class: "connection-editor", onsubmit: move |event| {
            event.prevent_default();
            let auth_type = match auth().as_str() {
                "none" => IntegrationAuthType::None,
                "header" => IntegrationAuthType::Header,
                "github" => IntegrationAuthType::Github,
                _ => IntegrationAuthType::Bearer,
            };
            on_action.call(ConnectionUiAction::SaveIntegration {
                request: IntegrationUpsert {
                    id: id().trim().to_owned(), label: label().trim().to_owned(),
                    kind: if auth_type == IntegrationAuthType::Github { IntegrationKind::Github } else { IntegrationKind::CustomRest },
                    base_url: base_url().trim().to_owned(), auth_type,
                    auth_header: (auth_type == IntegrationAuthType::Header).then(|| auth_header().trim().to_owned()),
                    enabled: enabled(),
                },
                secret: secret(),
            });
            secret.set(String::new());
        },
            h3 { "外部連携を追加・編集" }
            div { class: "connection-editor__grid",
                TextField { label: "ID", value: id(), placeholder: "internal-api", on_input: move |value| id.set(value) }
                TextField { label: "表示名", value: label(), placeholder: "社内API", on_input: move |value| label.set(value) }
                TextField { label: "ベースURL", value: base_url(), placeholder: "https://api.example.com", on_input: move |value| base_url.set(value) }
                label { class: "connection-field", span { "認証方式" }
                    select { value: "{auth}", onchange: move |event| auth.set(event.value()),
                        option { value: "none", "認証なし" }
                        option { value: "bearer", "Bearer" }
                        option { value: "header", "カスタムヘッダー" }
                        option { value: "github", "GitHub token" }
                    }
                }
                if auth() == "header" {
                    TextField { label: "認証ヘッダー", value: auth_header(), placeholder: "x-api-key", on_input: move |value| auth_header.set(value) }
                }
                label { class: "connection-field", span { "シークレット（変更時だけ入力）" }
                    input { r#type: "password", autocomplete: "off", value: "{secret}", oninput: move |event| secret.set(event.value()) }
                }
            }
            div { class: "connection-actions",
                ToggleButton { label: "有効", enabled: enabled(), on_toggle: move |value| enabled.set(value) }
                button { class: "connection-button", r#type: "submit", disabled: !valid, "保存" }
            }
        }
    }
}

#[component]
fn ContextEditor(
    context: ContextTriggerConfig,
    on_action: EventHandler<ConnectionUiAction>,
) -> Element {
    let compact_every = use_signal(|| context.compact.every_ms.to_string());
    let compact_min = use_signal(|| context.compact.min_context_pct.to_string());
    let compact_large = use_signal(|| context.compact.min_context_pct_large_window.to_string());
    let compact_message = use_signal(|| context.compact.message.clone());
    let clear_every = use_signal(|| context.clear.every_ms.to_string());
    let clear_min = use_signal(|| context.clear.min_context_pct.to_string());
    let clear_large = use_signal(|| context.clear.min_context_pct_large_window.to_string());
    let clear_message = use_signal(|| context.clear.message.clone());
    rsx! {
        div { class: "connection-editor",
            h3 { "条件とメッセージ" }
            div { class: "connection-editor__split",
                ContextRuleFields { title: "コンパクト", every: compact_every, minimum: compact_min, large: compact_large, message: compact_message }
                ContextRuleFields { title: "クリア", every: clear_every, minimum: clear_min, large: clear_large, message: clear_message }
            }
            button { class: "connection-button", r#type: "button", onclick: move |_| {
                let compact = parse_context_rule(context.compact.enabled, &compact_every(), &compact_min(), &compact_large(), compact_message());
                let clear = parse_context_rule(context.clear.enabled, &clear_every(), &clear_min(), &clear_large(), clear_message());
                if let (Some(compact), Some(clear)) = (compact, clear) {
                    on_action.call(ConnectionUiAction::SetContext(ContextTriggerConfig { compact, clear }));
                }
            }, "条件を保存" }
        }
    }
}

#[component]
fn ContextRuleFields(
    title: &'static str,
    every: Signal<String>,
    minimum: Signal<String>,
    large: Signal<String>,
    message: Signal<String>,
) -> Element {
    let mut every = every;
    let mut minimum = minimum;
    let mut large = large;
    let mut message = message;
    rsx! { fieldset { class: "connection-subform",
        legend { {title} }
        TextField { label: "間隔（ミリ秒）", value: every(), placeholder: "7200000", on_input: move |value| every.set(value) }
        TextField { label: "通常しきい値（%）", value: minimum(), placeholder: "60", on_input: move |value| minimum.set(value) }
        TextField { label: "大規模しきい値（%）", value: large(), placeholder: "40", on_input: move |value| large.set(value) }
        label { class: "connection-field", span { "実行メッセージ" }
            textarea { value: "{message}", oninput: move |event| message.set(event.value()) }
        }
    } }
}

#[component]
fn OrganisationEditor(
    mode: TriggerMode,
    has_key: bool,
    on_action: EventHandler<ConnectionUiAction>,
) -> Element {
    let mut key = use_signal(String::new);
    let selected = match mode {
        TriggerMode::Strict => "strict",
        TriggerMode::AllowAll => "allow-all",
        TriggerMode::CommunicationOnly => "communication-only",
    };
    rsx! { div { class: "connection-form-grid connection-form-grid--wide",
        label { class: "connection-field", span { "判定モード" }
            select { value: selected, onchange: move |event| {
                let mode = match event.value().as_str() { "allow-all" => TriggerMode::AllowAll, "communication-only" => TriggerMode::CommunicationOnly, _ => TriggerMode::Strict };
                on_action.call(ConnectionUiAction::SetOrganisationMode(mode));
            },
                option { value: "strict", "承認必須" }
                option { value: "allow-all", "すべて許可" }
                option { value: "communication-only", "連絡のみ許可" }
            }
        }
        label { class: "connection-field", span { "組織APIキー" }
            input { r#type: "password", autocomplete: "off", value: "{key}", placeholder: if has_key { "保存済み — 変更時だけ入力" } else { "API key" }, oninput: move |event| key.set(event.value()) }
        }
        button { class: "connection-button", r#type: "button", disabled: key().trim().is_empty(), onclick: move |_| {
            on_action.call(ConnectionUiAction::SaveOrganisationKey(key())); key.set(String::new());
        }, "キーを保存" }
    } }
}

#[component]
fn MissionEditor(
    mission: ScheduledMission,
    on_save: EventHandler<ScheduledMission>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut id = use_signal(|| mission.id.clone());
    let mut label = use_signal(|| mission.label.clone());
    let mut interval = use_signal(|| mission.interval_ms.to_string());
    let mut to = use_signal(|| mission.to.clone());
    let mut body = use_signal(|| mission.body.clone());
    let mut kind = use_signal(|| {
        match mission.kind {
            MissionKind::Heartbeat => "heartbeat",
            MissionKind::Compact => "compact",
            MissionKind::Dispatch => "dispatch",
        }
        .to_owned()
    });
    rsx! { form { class: "connection-editor", onsubmit: move |event| {
        event.prevent_default();
        let Ok(interval_ms) = interval().parse::<u64>() else { return; };
        on_save.call(ScheduledMission {
            id: id().trim().to_owned(), label: label().trim().to_owned(), interval_ms,
            weekly: mission.weekly.clone(), to: to().trim().to_owned(), body: body(),
            enabled: mission.enabled, last_fired_at_ms: mission.last_fired_at_ms,
            kind: match kind().as_str() { "heartbeat" => MissionKind::Heartbeat, "compact" => MissionKind::Compact, _ => MissionKind::Dispatch },
            quiet_threshold_ms: mission.quiet_threshold_ms,
        });
    },
        h3 { "スケジュールを追加・編集" }
        div { class: "connection-editor__grid",
            TextField { label: "ID", value: id(), placeholder: "daily-summary", on_input: move |value| id.set(value) }
            TextField { label: "表示名", value: label(), placeholder: "日次まとめ", on_input: move |value| label.set(value) }
            TextField { label: "間隔（ミリ秒）", value: interval(), placeholder: "3600000", on_input: move |value| interval.set(value) }
            TextField { label: "宛先", value: to(), placeholder: "god", on_input: move |value| to.set(value) }
            label { class: "connection-field", span { "種別" }
                select { value: "{kind}", onchange: move |event| kind.set(event.value()),
                    option { value: "dispatch", "プロンプト実行" }
                    option { value: "heartbeat", "ハートビート" }
                    option { value: "compact", "コンパクト" }
                }
            }
            label { class: "connection-field connection-field--wide", span { "本文" }
                textarea { value: "{body}", oninput: move |event| body.set(event.value()) }
            }
        }
        div { class: "connection-actions",
            button { class: "connection-button connection-button--primary", r#type: "submit", "保存" }
            button { class: "connection-button connection-button--quiet", r#type: "button", onclick: move |_| on_cancel.call(()), "キャンセル" }
        }
    } }
}

#[component]
fn TextField(
    label: &'static str,
    value: String,
    placeholder: &'static str,
    on_input: EventHandler<String>,
) -> Element {
    rsx! { label { class: "connection-field", span { {label} }
        input { value, placeholder, oninput: move |event| on_input.call(event.value()) }
    } }
}

fn parse_context_rule(
    enabled: bool,
    every: &str,
    minimum: &str,
    large: &str,
    message: String,
) -> Option<ContextRule> {
    Some(ContextRule {
        enabled,
        every_ms: every.parse().ok()?,
        min_context_pct: minimum.parse().ok()?,
        min_context_pct_large_window: large.parse().ok()?,
        message,
    })
}

fn default_mission() -> ScheduledMission {
    ScheduledMission {
        id: String::new(),
        label: String::new(),
        interval_ms: 3_600_000,
        weekly: None,
        to: String::from("god"),
        body: String::new(),
        enabled: true,
        last_fired_at_ms: None,
        kind: MissionKind::Dispatch,
        quiet_threshold_ms: None,
    }
}

fn format_duration(milliseconds: u64) -> String {
    if milliseconds.is_multiple_of(3_600_000) {
        format!("{}時間", milliseconds / 3_600_000)
    } else {
        format!("{}分", milliseconds / 60_000)
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::connections::CliAuthPhase;

    use super::{auth_button_presentation, format_duration};

    #[test]
    fn whole_hours_are_formatted_as_hours() {
        assert_eq!(format_duration(7_200_000), "2時間");
    }

    #[test]
    fn partial_hours_are_formatted_as_minutes() {
        assert_eq!(format_duration(5_400_000), "90分");
    }

    #[test]
    fn cli_auth_button_exposes_loading_error_and_success_states() {
        assert_eq!(
            auth_button_presentation(CliAuthPhase::Starting),
            ("開始中…", "loading")
        );
        assert_eq!(
            auth_button_presentation(CliAuthPhase::Failed),
            ("再試行", "error")
        );
        assert_eq!(
            auth_button_presentation(CliAuthPhase::Connected),
            ("接続済み", "success")
        );
    }

    #[test]
    fn missing_cli_is_truthfully_disabled_state() {
        assert_eq!(
            auth_button_presentation(CliAuthPhase::NotInstalled),
            ("CLI未検出", "default")
        );
    }
}
