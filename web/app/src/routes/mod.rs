use dioxus::prelude::*;
use dioxus_router::{Link, Routable, use_navigator};
use md_web_contracts::domains::config_onboarding::{
    CreateFloorRequest, FinishOnboardingRequest, FinishOnboardingResult, ResetNamespaceRequest,
    ShutdownRequest, UpdateStatus,
};

use crate::app::SelectedAgentContext;
use crate::components::domains::config_onboarding::ConfigOnboardingPanel;
use crate::components::domains::connections::{ConnectionUiAction, ConnectionsPanel};
use crate::components::domains::fs_git_ide::FsGitIde;
use crate::components::domains::hive_tasks::{
    ControlAction, HiveInitialTab, HiveTasksDomain, HiveTasksViewModel, MessageAction, TaskAction,
};
use crate::components::domains::memory_skills::{BaseSkillsOnboardingPanel, MemorySkillsWorkspace};
use crate::components::domains::office_ui::OfficeUi;
use crate::components::domains::pty_agents::{
    PtyAgentsAction, PtyAgentsDomain, PtyAgentsViewModel,
};
use crate::components::domains::voice_realtime::VoiceRealtimeDomain;
use crate::components::shell::AppShell;
use crate::server_fns::{
    activity_tail, base_skill_assignments, base_skills_catalog, base_skills_install,
    config_bootstrap, config_change_home, config_get, config_patch, config_set_agent_token_cap,
    config_write_provider_key, connections_add_integration_template, connections_clear_history,
    connections_create_default_webhook, connections_decide_history, connections_domain_snapshot,
    connections_probe_integration, connections_remove_integration, connections_remove_mission,
    connections_remove_webhook, connections_replace_missions, connections_rotate_webhook_secret,
    connections_set_context, connections_set_context_enabled, connections_set_mission_enabled,
    connections_set_organisation, connections_start_broker, connections_start_slack,
    connections_start_webhooks, connections_stop_broker, connections_stop_slack,
    connections_stop_webhooks, connections_update_slack, connections_upsert_integration,
    connections_upsert_webhook, connections_write_integration_secret,
    connections_write_organisation_key, connections_write_slack_secret, floor_create,
    history_query, hive_answer_question, hive_control_auto_delivery, hive_control_halt,
    hive_control_pause, hive_control_resume, hive_control_steer, hive_create_task,
    hive_delete_task, hive_dismiss_question, hive_move_task, hive_new_thread, hive_patch_role,
    hive_reply, hive_set_hold, hive_snapshot, hive_stop_worker, knowledge_get, knowledge_remove,
    knowledge_search, knowledge_upload, list_agents, memory_graph, memory_mine, memory_reflect,
    memory_semantic_search, memory_skills_snapshot, memory_wake_up, office_close_agent,
    office_dismiss_restore, office_dismiss_toast, office_focus, office_live_poll, office_note,
    office_rename, office_reorder, office_restore_all, office_select, office_snapshot,
    office_spawn, office_theme_preference, onboarding_finish, onboarding_spawn_team, pty_input,
    pty_kill, pty_queue, pty_redraw, pty_resize, pty_restart, pty_restore, pty_spawn, reset_all,
    save_base_skill_assignments, shutdown, skills_catalog, skills_install, skills_local,
    skills_uninstall, telemetry_waterfall, tools_status, update_check,
};

const PTY_BRIDGE_JS: Asset = asset!("/src/components/domains/pty_agents/xterm_bridge.js");
const XTERM_JS: Asset = asset!("/assets/vendor/xterm.js");
const XTERM_FIT_JS: Asset = asset!("/assets/vendor/addon-fit.js");
const XTERM_UNICODE11_JS: Asset = asset!("/assets/vendor/addon-unicode11.js");
const XTERM_CSS: Asset = asset!("/assets/vendor/xterm.css");

#[derive(Clone, Debug, PartialEq, Routable)]
#[rustfmt::skip]
pub(crate) enum AppRoute {
    #[route("/")]
    Office {},
    #[layout(AppShell)]
        #[route("/onboarding")]
        Onboarding {},
        #[route("/settings")]
        Settings {},
        #[route("/connections")]
        Connections {},
        #[route("/workspace")]
        Workspace {},
        #[route("/hive")]
        Hive {},
        #[route("/memory")]
        Memory {},
        #[route("/agents")]
        Agents {},
        #[route("/voice")]
        Voice {},
    #[end_layout]
    #[route("/:..route")]
    NotFound { route: Vec<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NavItem {
    pub(crate) route: AppRoute,
    pub(crate) label: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) enabled: bool,
}

pub(crate) fn nav_items() -> Vec<NavItem> {
    vec![
        nav(AppRoute::Office {}, "オフィス", "▦", true),
        nav(AppRoute::Onboarding {}, "初期設定", "◎", true),
        nav(AppRoute::Connections {}, "接続", "⌁", true),
        nav(AppRoute::Workspace {}, "ファイルとGit", "◇", true),
        nav(AppRoute::Hive {}, "タスク", "✓", true),
        nav(AppRoute::Memory {}, "記憶とスキル", "✦", true),
        nav(AppRoute::Agents {}, "ターミナルとAgent", ">_", true),
        nav(AppRoute::Voice {}, "音声", "◉", true),
    ]
}

fn nav(route: AppRoute, label: &'static str, icon: &'static str, enabled: bool) -> NavItem {
    NavItem {
        route,
        label,
        icon,
        enabled,
    }
}

#[component]
fn Office() -> Element {
    let config = use_resource(config_get);
    let navigator = use_navigator();
    let should_redirect = matches!(
        config.read().as_ref(),
        Some(Ok(config)) if config.requires_onboarding()
    );
    use_effect(move || {
        if should_redirect {
            navigator.replace(AppRoute::Onboarding {});
        }
    });
    match config.read().as_ref() {
        None => {
            return rsx! { main { id: "main-content", class: "office-load-state",
                role: "status", "初期設定を確認しています…"
            } };
        }
        Some(Err(_)) => {
            return rsx! { main { id: "main-content", class: "office-load-state",
                p { role: "alert", "初期設定を確認できません。PostgreSQL接続を確認してください。" }
            } };
        }
        Some(Ok(config)) if config.requires_onboarding() => {
            return rsx! { main { id: "main-content", class: "office-load-state",
                role: "status", "初期設定へ移動しています…"
            } };
        }
        Some(Ok(_)) => {}
    }
    rsx! { OfficeReady {} }
}

#[component]
fn OfficeReady() -> Element {
    let mut add_agent_open = use_signal(|| false);
    let mut add_agent_spawning = use_signal(|| false);
    let mut action_error = use_signal(|| None::<String>);
    let mut selected_command_tab = use_signal(|| 0_usize);
    let mut office = use_resource(office_snapshot);
    let mut live_snapshot =
        use_signal(|| None::<md_web_contracts::domains::office_ui::OfficeSnapshot>);
    use_future(move || async move {
        loop {
            let since_revision = live_snapshot
                .read()
                .as_ref()
                .map(|snapshot| snapshot.revision);
            if let Ok(Some(snapshot)) = office_live_poll(since_revision).await {
                live_snapshot.set(Some(snapshot));
            }
            let _ = document::eval("await new Promise(resolve => setTimeout(resolve, 500));")
                .join::<serde_json::Value>()
                .await;
        }
    });
    let agent_records = use_resource(list_agents);
    let mut selected_context = use_context::<Signal<SelectedAgentContext>>();
    let navigator = use_navigator();
    let state = match office.read().as_ref().cloned() {
        Some(Ok(state)) => state,
        Some(Err(_)) => {
            return rsx! { main { id: "main-content", class: "office-load-state",
                p { role: "alert", "PostgreSQLの永続状態を利用できません。接続設定とschema versionを確認してください。" }
            } };
        }
        None => {
            return rsx! { main { id: "main-content", class: "office-load-state",
                role: "status", "オフィスの永続状態を読み込んでいます…"
            } };
        }
    };
    let mut snapshot = live_snapshot.read().clone().unwrap_or(state.snapshot);
    let selected_agent_id = snapshot
        .selected_agent_id
        .clone()
        .filter(|selected| snapshot.agents.iter().any(|agent| agent.id == *selected))
        .or_else(|| {
            snapshot
                .agents
                .iter()
                .find(|agent| agent.is_god)
                .map(|agent| agent.id.clone())
        })
        .or_else(|| snapshot.agents.first().map(|agent| agent.id.clone()));
    snapshot.selected_agent_id = selected_agent_id.clone();
    snapshot.agents.sort_by(|left, right| {
        left.project
            .cmp(&right.project)
            .then_with(|| right.is_god.cmp(&left.is_god))
    });
    let focus_mode = state.focus_mode;
    let running_terminals = u32::try_from(snapshot.agents.len()).unwrap_or(u32::MAX);
    rsx! { main { id: "main-content",
        if let Some(error) = action_error() {
            p { class: "office-global-error", role: "alert", {error} }
        }
        OfficeUi {
            snapshot,
            notices: state.notices, auto_mode: state.auto_mode,
            app_version: String::from(env!("CARGO_PKG_VERSION")),
            detail_panel: rsx! { OfficeCommandCenter { selected_agent_id, selected_tab: selected_command_tab } },
            focus_mode, add_agent_open: add_agent_open(), add_agent_spawning: add_agent_spawning(),
            on_add_agent: move |_| add_agent_open.set(true),
            on_close_add_agent: move |_| add_agent_open.set(false),
            on_spawn_agent: move |request| { spawn(async move {
                add_agent_spawning.set(true);
                action_error.set(office_spawn(request).await.err().map(|error| error.to_string()));
                add_agent_spawning.set(false);
                if action_error.read().is_none() { add_agent_open.set(false); }
                office.restart();
            }); },
            on_select: move |agent_id| { spawn(async move {
                action_error.set(office_select(Some(agent_id)).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_reorder: move |(from_id, to_id)| { spawn(async move {
                action_error.set(office_reorder(from_id, to_id).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_rename: move |(agent_id, name)| { spawn(async move {
                action_error.set(office_rename(agent_id, name).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_note: move |(agent_id, note)| { spawn(async move {
                action_error.set(office_note(agent_id, note).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_open_task: move |_: String| { navigator.push(AppRoute::Hive {}); },
            on_open_tasks: move |_| selected_command_tab.set(2),
            on_open_human_questions: move |_| selected_command_tab.set(3),
            on_request_close: move |_| { spawn(async move {
                action_error.set(shutdown(ShutdownRequest {
                    expected_running_terminals: running_terminals,
                    graceful: true,
                }).await.err().map(|error| error.to_string()));
            }); },
            on_restore_all: move |_| { spawn(async move {
                action_error.set(office_restore_all().await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_dismiss_restore: move |agent_id| { spawn(async move {
                action_error.set(office_dismiss_restore(agent_id).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_theme: move |preference| { spawn(async move {
                action_error.set(office_theme_preference(preference).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_open_ide: move |agent_id: String| {
                let workspace_path = agent_records
                    .read()
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .and_then(|(active, _)| active.iter().find(|agent| agent.id == agent_id))
                    .map(|agent| agent.cwd.clone());
                selected_context.set(SelectedAgentContext {
                    agent_id: Some(agent_id),
                    workspace_path,
                });
                navigator.push(AppRoute::Workspace {});
            },
            on_open_terminal: move |agent_id: String| {
                selected_context.write().agent_id = Some(agent_id);
                navigator.push(AppRoute::Agents {});
            },
            on_close_agent: move |agent_id| { spawn(async move {
                action_error.set(office_close_agent(agent_id).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_open_settings: move || { navigator.push(AppRoute::Settings {}); },
            on_toggle_focus: move |_| { spawn(async move {
                action_error.set(office_focus(!focus_mode).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
            on_dismiss_notice: move |(id, occurred_at)| { spawn(async move {
                action_error.set(office_dismiss_toast(id, occurred_at).await.err().map(|error| error.to_string()));
                office.restart();
            }); },
        }
    } }
}

#[component]
fn OfficeCommandCenter(
    selected_agent_id: Option<String>,
    mut selected_tab: Signal<usize>,
) -> Element {
    const TABS: [&str; 10] = [
        "ターミナル",
        "モニター",
        "タスク",
        "質問",
        "トリガー",
        "メモリ",
        "グラフ",
        "アクティビティ",
        "コマンド",
        "ワーカー",
    ];
    rsx! { section { class: "office-command-center", "data-agent-id": selected_agent_id,
        nav { class: "office-command-center__tabs", aria_label: "コマンドセンター",
            for (index, label) in TABS.iter().enumerate() {
                button { r#type: "button", class: if selected_tab() == index { "is-active" } else { "" },
                    aria_selected: (selected_tab() == index).to_string(),
                    onclick: move |_| selected_tab.set(index), {*label}
                }
            }
        }
        div {
            class: "office-command-center__panel",
            key: "{selected_agent_id:?}",
            match selected_tab() {
                0 => rsx! { SelectedAgents { initial_agent_id: selected_agent_id.clone() } },
                1 => rsx! { AgentMonitorCompact { agent_id: selected_agent_id.clone() } },
                2 => rsx! { SelectedHive { initial_tab: HiveInitialTab::Tasks, selected_agent_id: selected_agent_id.clone() } },
                3 => rsx! { SelectedHive { initial_tab: HiveInitialTab::AskMe, selected_agent_id: selected_agent_id.clone() } },
                4 => rsx! { TriggerCompact {} },
                5 => rsx! { SelectedMemory { initial_agent_id: selected_agent_id.clone() } },
                6 => rsx! { MemoryGraphCompact {} },
                7 => rsx! { ActivityCompact {} },
                8 => rsx! { CommandHistoryCompact { agent_id: selected_agent_id.clone() } },
                9 => rsx! { SelectedHive { initial_tab: HiveInitialTab::Workers, selected_agent_id: selected_agent_id.clone() } },
                _ => rsx! { p { role: "alert", "選択したタブを表示できません。" } },
            }
        }
    } }
}

#[component]
fn AgentMonitorCompact(agent_id: Option<String>) -> Element {
    let monitor = use_resource(move || telemetry_waterfall(agent_id.clone()));
    rsx! { section { class: "command-center-compact", aria_labelledby: "agent-monitor-title",
        h2 { id: "agent-monitor-title", "実行モニター" }
        match monitor.read().as_ref() {
            None => rsx! { p { role: "status", "実行状況を読み込んでいます…" } },
            Some(Err(_)) => rsx! { p { role: "alert", "実行状況を取得できませんでした。" } },
            Some(Ok(waterfall)) => rsx! {
                p { "{waterfall.rows.len()}件 · {waterfall.duration_ms}ms" }
                ul { class: "command-center-compact__list",
                    for row in waterfall.rows.iter().take(20) {
                        li {
                            {format!(
                                "{} · {}ms · {}",
                                row.tool,
                                row.duration_ms,
                                if row.success { "成功" } else { "失敗" },
                            )}
                        }
                    }
                }
            },
        }
    } }
}

#[component]
fn TriggerCompact() -> Element {
    let snapshot = use_resource(connections_domain_snapshot);
    rsx! { section { class: "command-center-compact", aria_labelledby: "trigger-compact-title",
        h2 { id: "trigger-compact-title", "トリガー" }
        match snapshot.read().as_ref() {
            None => rsx! { p { role: "status", "接続状態を読み込んでいます…" } },
            Some(Err(_)) => rsx! { p { role: "alert", "接続状態を取得できませんでした。" } },
            Some(Ok(value)) => rsx! {
                dl { class: "command-center-compact__metrics",
                    dt { "Slack" } dd { {if value.slack.enabled { "有効" } else { "無効" }} }
                    dt { "Webhook" } dd { "{value.webhooks.iter().filter(|item| item.enabled).count()}件" }
                    dt { "連携" } dd { "{value.integrations.len()}件" }
                    dt { "定期Mission" } dd { "{value.missions.iter().filter(|item| item.enabled).count()}件" }
                }
            },
        }
    } }
}

#[component]
fn MemoryGraphCompact() -> Element {
    let graph = use_resource(memory_graph);
    rsx! { section { class: "command-center-compact", aria_labelledby: "memory-graph-title",
        h2 { id: "memory-graph-title", "メモリグラフ" }
        match graph.read().as_ref() {
            None => rsx! { p { role: "status", "グラフを読み込んでいます…" } },
            Some(Err(_)) => rsx! { p { role: "alert", "グラフを取得できませんでした。" } },
            Some(Ok(value)) => rsx! {
                p { "{value.nodes.len()}ノード · {value.edges.len()}リンク" }
                ul { class: "command-center-compact__list",
                    for node in value.nodes.iter().take(20) {
                        li { strong { {node.label.clone()} } span { " · {node.modality}" } }
                    }
                }
            },
        }
    } }
}

#[component]
fn ActivityCompact() -> Element {
    let activity = use_resource(|| activity_tail(50));
    rsx! { section { class: "command-center-compact", aria_labelledby: "activity-compact-title",
        h2 { id: "activity-compact-title", "アクティビティ" }
        match activity.read().as_ref() {
            None => rsx! { p { role: "status", "活動を読み込んでいます…" } },
            Some(Err(_)) => rsx! { p { role: "alert", "活動を取得できませんでした。" } },
            Some(Ok(rows)) => rsx! { ul { class: "command-center-compact__list",
                for row in rows.iter() { li { strong { {row.kind.clone()} } span { " · {row.summary}" } } }
            } },
        }
    } }
}

#[component]
fn CommandHistoryCompact(agent_id: Option<String>) -> Element {
    use md_web_contracts::domains::memory_skills::HistoryQuery;
    let history = use_resource(move || {
        history_query(HistoryQuery {
            agent_id: agent_id.clone(),
            query: None,
            limit: 50,
        })
    });
    rsx! { section { class: "command-center-compact", aria_labelledby: "command-history-title",
        h2 { id: "command-history-title", "コマンド履歴" }
        match history.read().as_ref() {
            None => rsx! { p { role: "status", "履歴を読み込んでいます…" } },
            Some(Err(_)) => rsx! { p { role: "alert", "履歴を取得できませんでした。" } },
            Some(Ok(rows)) => rsx! { ol { class: "command-center-compact__list",
                for row in rows.iter() { li { {row.text.clone()} } }
            } },
        }
    } }
}

#[component]
fn Onboarding() -> Element {
    config_view()
}

#[component]
fn Settings() -> Element {
    config_view()
}

fn config_view() -> Element {
    let navigator = use_navigator();
    let mut bootstrap = use_resource(config_bootstrap);
    let mut installed_base_skills = use_resource(skills_local);
    let mut refresh_base_skill_catalog = use_signal(|| false);
    let mut base_skill_catalog =
        use_resource(move || base_skills_catalog(refresh_base_skill_catalog()));
    let mut team_assignments = use_resource(base_skill_assignments);
    let mut base_skill_busy = use_signal(|| false);
    let mut base_skill_error = use_signal(|| None::<String>);
    let active_agents = use_resource(list_agents);
    let mut update = use_signal(|| UpdateStatus::Idle);
    let mut action_error = use_signal(|| None::<String>);
    let mut finish_pending = use_signal(|| false);
    let mut finish_receipt = use_signal(|| None::<FinishOnboardingResult>);
    let mut finish_completed = use_signal(|| None::<FinishOnboardingResult>);
    let mut finish_error = use_signal(|| None::<String>);
    let mut reset_confirmation = use_signal(String::new);
    let (config, tools, capabilities, repository) = match bootstrap.read().as_ref().cloned() {
        Some(Ok(value)) => value,
        Some(Err(_)) => {
            return rsx! { div { class: "route-surface",
                p { role: "alert", "PostgreSQLの設定を読み込めません。MD_PG_*とschema versionを確認してください。" }
            } };
        }
        None => {
            return rsx! { div { class: "route-surface", role: "status", "設定を読み込んでいます…" } };
        }
    };
    let running_terminals = active_agents
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map_or(0, |(active, _)| {
            u32::try_from(active.len()).unwrap_or(u32::MAX)
        });
    let base_skills = installed_base_skills
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    rsx! { section { class: "route-surface",
        if let Some(error) = action_error() {
            p { role: "alert", class: "domain-error", {error} }
        }
        ConfigOnboardingPanel {
            config: config.clone(), tools, update: update(), capabilities,
            base_skills,
            release_repository: repository,
            finish_pending: finish_pending(),
            finish_result: finish_completed(),
            finish_error: finish_error(),
            on_patch: move |patch| { spawn(async move {
                action_error.set(config_patch(patch).await.err().map(|error| error.to_string()));
                bootstrap.restart();
            }); },
            on_finish: move |request: FinishOnboardingRequest| { spawn(async move {
                finish_pending.set(true);
                finish_error.set(None);
                let receipt = match finish_receipt.read().clone() {
                    Some(receipt) => Ok(receipt),
                    None => onboarding_finish(request).await,
                };
                match receipt {
                    Ok(receipt) => {
                        finish_receipt.set(Some(receipt.clone()));
                        match onboarding_spawn_team(receipt.clone()).await {
                            Ok(confirmed) if confirmed.config.onboarding_ready() => {
                                let mut completed = receipt;
                                completed.config = confirmed.config;
                                finish_completed.set(Some(completed));
                                finish_pending.set(false);
                                navigator.push(AppRoute::Office {});
                            }
                            Ok(_) => {
                                finish_error.set(Some(String::from(
                                    "初期チームの確認結果が不完全です。再試行してください。",
                                )));
                                finish_pending.set(false);
                            }
                            Err(_) => {
                                finish_error.set(Some(String::from(
                                    "設定は保存しましたが、Aria・Implementer・Verifierを起動できませんでした。設定を確認して再試行してください。",
                                )));
                                finish_pending.set(false);
                            }
                        }
                    }
                    Err(error) => {
                        finish_error.set(Some(error.to_string()));
                        finish_pending.set(false);
                    }
                }
            }); },
            on_refresh_tools: move |_| { spawn(async move {
                action_error.set(tools_status().await.err().map(|error| error.to_string()));
                bootstrap.restart();
            }); },
            on_check_update: move |_| { spawn(async move {
                match update_check().await {
                    Ok(status) => { update.set(status); action_error.set(None); },
                    Err(error) => action_error.set(Some(error.to_string())),
                }
            }); },
            on_open_release: move |url: String| { spawn(async move {
                let encoded = serde_json::to_string(&url).unwrap_or_else(|_| String::from("\"\""));
                let script = format!("window.open({encoded}, '_blank', 'noopener,noreferrer'); return true;");
                let _ = document::eval(&script).await;
            }); },
            on_create_floor: move |request: CreateFloorRequest| { spawn(async move {
                match floor_create(request).await {
                    Ok(floor) => {
                        let encoded = serde_json::to_string(&floor.path).unwrap_or_else(|_| String::from("\"\""));
                        let script = format!("window.open({encoded}, '_blank', 'noopener,noreferrer'); return true;");
                        let _ = document::eval(&script).await;
                        action_error.set(None);
                    }
                    Err(error) => action_error.set(Some(error.to_string())),
                }
            }); },
            on_change_home: move |request| { spawn(async move {
                action_error.set(config_change_home(request).await.err().map(|error| error.to_string()));
                bootstrap.restart();
            }); },
            on_set_agent_token_cap: move |request| { spawn(async move {
                action_error.set(config_set_agent_token_cap(request).await.err().map(|error| error.to_string()));
                bootstrap.restart();
            }); },
            on_provider_key: move |request| { spawn(async move {
                action_error.set(config_write_provider_key(request).await.err().map(|error| error.to_string()));
                bootstrap.restart();
            }); },
        }
        if config.requires_onboarding() {
            BaseSkillsOnboardingPanel {
                catalog: base_skill_catalog
                    .read()
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .cloned()
                    .unwrap_or_default(),
                assignments: team_assignments
                    .read()
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .cloned()
                    .unwrap_or_default(),
                busy: base_skill_busy(),
                error: base_skill_error(),
                on_refresh: move |_| {
                    refresh_base_skill_catalog.set(true);
                    base_skill_catalog.restart();
                },
                on_install: move |request| { spawn(async move {
                    base_skill_busy.set(true);
                    match base_skills_install(request).await {
                        Ok(_) => {
                            base_skill_error.set(None);
                            installed_base_skills.restart();
                            base_skill_catalog.restart();
                        }
                        Err(error) => base_skill_error.set(Some(error.to_string())),
                    }
                    base_skill_busy.set(false);
                }); },
                on_save_assignments: move |assignments| { spawn(async move {
                    base_skill_busy.set(true);
                    match save_base_skill_assignments(assignments).await {
                        Ok(_) => {
                            base_skill_error.set(None);
                            team_assignments.restart();
                        }
                        Err(error) => base_skill_error.set(Some(error.to_string())),
                    }
                    base_skill_busy.set(false);
                }); },
            }
        }
        if config.onboarding_ready() {
            section { class: "settings-section", aria_labelledby: "namespace-reset-title",
                h2 { id: "namespace-reset-title", "このオフィスを初期化" }
                p { "稼働中の処理とPTYを停止してから、現在のPostgreSQL namespaceだけを一つのtransactionで削除します。" }
                label { r#for: "namespace-reset-confirmation", "確認語句（RESET <namespace>）" }
                input {
                    id: "namespace-reset-confirmation",
                    value: reset_confirmation(),
                    autocomplete: "off",
                    spellcheck: "false",
                    oninput: move |event| reset_confirmation.set(event.value()),
                }
                button {
                    class: "co-button co-button--danger",
                    r#type: "button",
                    disabled: reset_confirmation().strip_prefix("RESET ").is_none_or(str::is_empty),
                    onclick: move |_| {
                        let confirmation = reset_confirmation();
                        spawn(async move {
                            let namespace = confirmation
                                .strip_prefix("RESET ")
                                .unwrap_or_default()
                                .to_owned();
                            match reset_all(ResetNamespaceRequest { namespace, confirmation }).await {
                                Ok(receipt) if receipt.reset => {
                                    action_error.set(None);
                                    navigator.replace(AppRoute::Onboarding {});
                                }
                                Ok(receipt) => action_error.set(Some(receipt.detail_ja)),
                                Err(error) => action_error.set(Some(error.to_string())),
                            }
                        });
                    },
                    "namespaceを初期化"
                }
            }
            section { class: "settings-section", aria_labelledby: "server-shutdown-title",
                h2 { id: "server-shutdown-title", "サーバーの終了" }
                p { "受付を止め、稼働中のPTYとproducerを終了してからPostgreSQL接続を閉じます。" }
                button { class: "co-button co-button--danger", r#type: "button",
                    onclick: move |_| { spawn(async move {
                        match shutdown(ShutdownRequest {
                            expected_running_terminals: running_terminals,
                            graceful: true,
                        }).await {
                            Ok(result) => action_error.set(Some(result.detail_ja)),
                            Err(error) => action_error.set(Some(error.to_string())),
                        }
                    }); },
                    "サーバーを安全に終了"
                }
            }
        }
    } }
}

#[component]
fn Connections() -> Element {
    use md_web_contracts::domains::connections::{
        SlackConfigPatch, SlackSecretKind, SlackSecretWrite, WriteOnlySecret,
    };

    let mut snapshot = use_resource(connections_domain_snapshot);
    let mut one_time_secret = use_signal(|| None);
    let mut action_error = use_signal(|| None::<String>);
    let value = match snapshot.read().as_ref().cloned() {
        Some(Ok(value)) => value,
        Some(Err(_)) => {
            return rsx! { div { class: "route-surface",
                p { role: "alert", "接続設定の永続状態を利用できません。サーバー設定を確認してください。" }
            } };
        }
        None => {
            return rsx! { div { class: "route-surface", role: "status", "接続設定を読み込んでいます…" } };
        }
    };
    let action_snapshot = value.clone();
    rsx! { section { class: "route-surface",
        if let Some(error) = action_error() {
            p { role: "alert", class: "domain-error", {error} }
        }
        ConnectionsPanel {
            snapshot: value,
            one_time_secret: one_time_secret(),
            on_action: move |action: ConnectionUiAction| {
                let current = action_snapshot.clone();
                spawn(async move {
                    let result = match action {
                        ConnectionUiAction::Refresh => Ok(()),
                        ConnectionUiAction::SaveSlackSigningSecret(secret) => {
                            match WriteOnlySecret::new(secret) {
                                Ok(secret) => connections_write_slack_secret(SlackSecretWrite {
                                    kind: SlackSecretKind::SigningSecret, secret,
                                })
                                    .await.map(|_| ()),
                                Err(_) => Err(ServerFnError::new("シークレットを入力してください")),
                            }
                        }
                        ConnectionUiAction::SaveSlackBotToken(secret) => {
                            match WriteOnlySecret::new(secret) {
                                Ok(secret) => connections_write_slack_secret(SlackSecretWrite {
                                    kind: SlackSecretKind::BotToken, secret,
                                })
                                    .await.map(|_| ()),
                                Err(_) => Err(ServerFnError::new("シークレットを入力してください")),
                            }
                        }
                        ConnectionUiAction::SetSlackEnabled(enabled) => connections_update_slack(
                            SlackConfigPatch { enabled: Some(enabled), channel_id: None, port: None,
                                proactive_posting: None }).await.map(|_| ()),
                        ConnectionUiAction::SetSlackProactivePosting(enabled) => connections_update_slack(
                            SlackConfigPatch { enabled: None, channel_id: None, port: None,
                                proactive_posting: Some(enabled) }).await.map(|_| ()),
                        ConnectionUiAction::SetSlackEndpoint { channel_id, port } => connections_update_slack(
                            SlackConfigPatch { enabled: None, channel_id: Some(channel_id), port: Some(port),
                                proactive_posting: None }).await.map(|_| ()),
                        ConnectionUiAction::StartSlack => connections_start_slack().await.map(|_| ()),
                        ConnectionUiAction::StopSlack => connections_stop_slack().await.map(|_| ()),
                        ConnectionUiAction::BeginIntegrationFromTemplate(id) =>
                            connections_add_integration_template(id).await.map(|_| ()),
                        ConnectionUiAction::ProbeIntegration(id) =>
                            connections_probe_integration(id, String::from("/")).await.map(|_| ()),
                        ConnectionUiAction::RemoveIntegration(id) =>
                            connections_remove_integration(id).await.map(|_| ()),
                        ConnectionUiAction::SaveIntegration { request, secret } => {
                            let id = request.id.clone();
                            match connections_upsert_integration(request).await {
                                Ok(_) if secret.trim().is_empty() => Ok(()),
                                Ok(_) => match WriteOnlySecret::new(secret) {
                                    Ok(secret) => connections_write_integration_secret(id, secret).await.map(|_| ()),
                                    Err(_) => Err(ServerFnError::new("シークレットを確認してください")),
                                },
                                Err(error) => Err(error),
                            }
                        }
                        ConnectionUiAction::AddWebhook => {
                            let name = format!("Webhook {}", current.webhooks.len() + 1);
                            connections_create_default_webhook(name).await.map(|created| {
                                one_time_secret.set(Some(created.secret));
                            })
                        }
                        ConnectionUiAction::RotateWebhookSecret(id) =>
                            connections_rotate_webhook_secret(id).await.map(|(secret, _)| {
                                one_time_secret.set(Some(secret));
                            }),
                        ConnectionUiAction::SetWebhookEnabled { id, enabled } => {
                            let request = current.webhooks.iter().find(|item| item.id == id).map(|item|
                                md_web_contracts::domains::connections::WebhookUpsert {
                                    id: item.id.clone(), name: item.name.clone(), enabled,
                                    mode: item.mode, schema: item.schema.clone(),
                                });
                            match request {
                                Some(request) => connections_upsert_webhook(request).await.map(|_| ()),
                                None => Err(ServerFnError::new("Webhookが見つかりません")),
                            }
                        }
                        ConnectionUiAction::RemoveWebhook(id) =>
                            connections_remove_webhook(id).await.map(|_| ()),
                        ConnectionUiAction::StartWebhookListener =>
                            connections_start_webhooks().await.map(|_| ()),
                        ConnectionUiAction::StopWebhookListener =>
                            connections_stop_webhooks().await.map(|_| ()),
                        ConnectionUiAction::SetContextEnabled { action, enabled } =>
                            connections_set_context_enabled(action, enabled).await.map(|_| ()),
                        ConnectionUiAction::SetContext(context) =>
                            connections_set_context(context).await.map(|_| ()),
                        ConnectionUiAction::SetOrganisationEnabled(enabled) =>
                            connections_set_organisation(enabled, current.organisation.mode).await.map(|_| ()),
                        ConnectionUiAction::SaveOrganisationKey(secret) => match WriteOnlySecret::new(secret) {
                            Ok(secret) => connections_write_organisation_key(secret).await.map(|_| ()),
                            Err(_) => Err(ServerFnError::new("組織APIキーを入力してください")),
                        },
                        ConnectionUiAction::SetOrganisationMode(mode) =>
                            connections_set_organisation(current.organisation.enabled, mode).await.map(|_| ()),
                        ConnectionUiAction::DecideHistory { id, decision } =>
                            connections_decide_history(id, decision, None).await.map(|_| ()),
                        ConnectionUiAction::ClearHistory(source) =>
                            connections_clear_history(Some(source)).await.map(|_| ()),
                        ConnectionUiAction::SetMissionEnabled { id, enabled } =>
                            connections_set_mission_enabled(id, enabled).await.map(|_| ()),
                        ConnectionUiAction::RemoveMission(id) =>
                            connections_remove_mission(id).await.map(|_| ()),
                        ConnectionUiAction::UpsertMission(mission) => {
                            let mut missions = current.missions.clone();
                            if let Some(existing) = missions.iter_mut().find(|item| item.id == mission.id) {
                                *existing = mission;
                            } else { missions.push(mission); }
                            connections_replace_missions(missions).await.map(|_| ())
                        }
                        ConnectionUiAction::StartBroker => connections_start_broker().await.map(|started| {
                            one_time_secret.set(Some(started.capability));
                        }),
                        ConnectionUiAction::StopBroker => connections_stop_broker().await.map(|_| ()),
                    };
                    action_error.set(result.err().map(|error| error.to_string()));
                    snapshot.restart();
                });
            },
        }
    } }
}

#[component]
fn Workspace() -> Element {
    let selected = use_context::<Signal<SelectedAgentContext>>();
    let initial_workspace_path = selected.read().workspace_path.clone();
    rsx! { section { class: "route-surface", FsGitIde { initial_workspace_path } } }
}

#[component]
fn Hive() -> Element {
    rsx! { SelectedHive { initial_tab: HiveInitialTab::Tasks, selected_agent_id: None } }
}

#[component]
fn SelectedHive(initial_tab: HiveInitialTab, selected_agent_id: Option<String>) -> Element {
    let mut snapshot = use_resource(move || hive_snapshot(selected_agent_id.clone()));
    let mut action_error = use_signal(|| None::<String>);
    use_effect(move || {
        spawn(async move {
            let _ = document::eval(
                r#"
                globalThis.__mdHiveEvents?.close();
                const stream = new EventSource('/api/hive/events/stream');
                globalThis.__mdHiveEvents = stream;
                const refresh = () => document.getElementById('hive-live-refresh')?.click();
                stream.addEventListener('hive', refresh);
                stream.addEventListener('hive-reset', refresh);
                return true;
                "#,
            )
            .await;
        });
    });
    use_drop(move || {
        spawn(async move {
            let _ = document::eval(
                "globalThis.__mdHiveEvents?.close(); globalThis.__mdHiveEvents = null; return true;",
            )
            .await;
        });
    });
    let view = match snapshot.read().as_ref().cloned() {
        Some(Ok(value)) => HiveTasksViewModel {
            board: value.board,
            tasks: value.tasks,
            agents: value.agents,
            messages: value.messages,
            log_tail: value.log_tail,
            selected_memory: value.selected_memory,
            selected_agent_id: value.selected_agent_id,
            selected_control: value.selected_control,
            workers: value.workers,
            preserved_worktrees: value.preserved_worktrees,
            max_workers: value.max_workers,
            loading: false,
            error: action_error(),
        },
        Some(Err(error)) => HiveTasksViewModel {
            error: Some(error.to_string()),
            ..HiveTasksViewModel::default()
        },
        None => HiveTasksViewModel {
            loading: true,
            ..HiveTasksViewModel::default()
        },
    };
    rsx! { section { class: "route-surface",
        button {
            id: "hive-live-refresh",
            class: "visually-hidden",
            r#type: "button",
            tabindex: "-1",
            aria_hidden: "true",
            onclick: move |_| snapshot.restart(),
            "イベント更新"
        }
        HiveTasksDomain {
            view,
            initial_tab,
            on_task: move |action: TaskAction| { spawn(async move {
                let result = match action {
                    TaskAction::Create { title, description, assignee, priority } =>
                        hive_create_task(title, description, assignee, priority).await.map(|_| ()),
                    TaskAction::Move { task_id, status } => hive_move_task(task_id, status).await.map(|_| ()),
                    TaskAction::Delete { task_id } => hive_delete_task(task_id).await,
                    TaskAction::Answer { task_id, answer } => hive_answer_question(task_id, answer).await.map(|_| ()),
                    TaskAction::DismissQuestion { task_id } => hive_dismiss_question(task_id).await.map(|_| ()),
                };
                action_error.set(result.err().map(|error| error.to_string()));
                snapshot.restart();
            }); },
            on_message: move |action: MessageAction| { spawn(async move {
                let result = match action {
                    MessageAction::Reply { conversation, body } => hive_reply(conversation, body).await.map(|_| ()),
                    MessageAction::NewThread { subject, body } => hive_new_thread(subject, body).await.map(|_| ()),
                };
                action_error.set(result.err().map(|error| error.to_string()));
                snapshot.restart();
            }); },
            on_control: move |action: ControlAction| { spawn(async move {
                let result = match action {
                    ControlAction::Pause { agent_id, on } => hive_control_pause(agent_id, on).await.map(|_| ()),
                    ControlAction::AutoDelivery { agent_id, paused } => hive_control_auto_delivery(agent_id, paused).await.map(|_| ()),
                    ControlAction::Resume { agent_id } => hive_control_resume(agent_id).await.map(|_| ()),
                    ControlAction::Steer { agent_id, text } => hive_control_steer(agent_id, text).await.map(|_| ()),
                    ControlAction::Halt { agent_id } => hive_control_halt(agent_id).await.map(|_| ()),
                    ControlAction::PatchRole { agent_id, role } => hive_patch_role(agent_id, role).await,
                    ControlAction::SetHold { agent_id, on } => hive_set_hold(agent_id, on).await,
                };
                action_error.set(result.err().map(|error| error.to_string()));
                snapshot.restart();
            }); },
            on_stop_worker: move |worker_id| { spawn(async move {
                action_error.set(hive_stop_worker(worker_id).await.err().map(|error| error.to_string()));
                snapshot.restart();
            }); },
            on_refresh: move |_| snapshot.restart(),
        }
    } }
}

#[component]
fn Memory() -> Element {
    let selected_context = use_context::<Signal<SelectedAgentContext>>();
    let initial_agent_id = selected_context.read().agent_id.clone();
    rsx! { SelectedMemory { initial_agent_id } }
}

#[component]
fn SelectedMemory(initial_agent_id: Option<String>) -> Element {
    use md_web_contracts::domains::memory_skills::{HistoryQuery, MemorySearchRequest};
    let mut snapshot = use_resource(memory_skills_snapshot);
    let graph = use_resource(memory_graph);
    let agents = use_resource(list_agents);
    let mut memory_result = use_signal(|| None);
    let mut knowledge_hits = use_signal(Vec::new);
    let mut knowledge_detail = use_signal(|| None);
    let mut searched_history = use_signal(Vec::new);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let current = match snapshot.read().as_ref().cloned() {
        Some(Ok(current)) => current,
        Some(Err(_)) => {
            return rsx! { div { class: "route-surface",
                p { role: "alert", "記憶とスキルの永続状態を利用できません。PostgreSQL設定を確認してください。" }
            } };
        }
        None => {
            return rsx! { div { class: "route-surface", role: "status", "記憶とスキルを読み込んでいます…" } };
        }
    };
    let available_agents = agents
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|(active, _)| active.clone())
        .unwrap_or_default();
    let selected_agent = use_memo(move || {
        initial_agent_id
            .clone()
            .filter(|selected| available_agents.iter().any(|agent| agent.id == *selected))
            .or_else(|| available_agents.first().map(|agent| agent.id.clone()))
    });
    let spans = current
        .telemetry
        .spans
        .values()
        .flatten()
        .cloned()
        .collect::<Vec<_>>();
    let history = if searched_history.read().is_empty() {
        current.history.clone()
    } else {
        searched_history()
    };
    rsx! { section { class: "route-surface",
        MemorySkillsWorkspace {
            memory_status: current.memory, memory_result: memory_result(),
            memory_graph: graph.read().as_ref().and_then(|result| result.as_ref().ok()).cloned().unwrap_or_default(),
            knowledge_status: current.knowledge, knowledge_documents: current.documents,
            knowledge_hits: knowledge_hits(), knowledge_detail: knowledge_detail(), local_skills: current.local_skills,
            catalog_skills: current.catalog.skills, activities: current.activities,
            spans, history, costs: current.costs, busy: busy(), error: error(),
            on_memory_search: move |query| { spawn(async move {
                busy.set(true);
                match memory_semantic_search(MemorySearchRequest { query, wing: selected_agent(), results: 20 }).await {
                    Ok(result) => { memory_result.set(Some(result)); error.set(None); }
                    Err(err) => error.set(Some(err.to_string())),
                }
                busy.set(false);
            }); },
            on_memory_wake_up: move || { spawn(async move {
                busy.set(true);
                match memory_wake_up(selected_agent()).await {
                    Ok(result) => { memory_result.set(Some(result)); error.set(None); }
                    Err(err) => error.set(Some(err.to_string())),
                }
                busy.set(false);
            }); },
            on_memory_mine: move || { if let Some(agent_id) = selected_agent() { spawn(async move {
                busy.set(true); error.set(memory_mine(agent_id).await.err().map(|err| err.to_string()));
                busy.set(false); snapshot.restart();
            }); } else { error.set(Some(String::from("対象エージェントを選択してください"))); } },
            on_memory_reflect: move || { if let Some(agent_id) = selected_agent() { spawn(async move {
                busy.set(true); error.set(memory_reflect(agent_id).await.err().map(|err| err.to_string()));
                busy.set(false); snapshot.restart();
            }); } else { error.set(Some(String::from("対象エージェントを選択してください"))); } },
            on_knowledge_search: move |query| { spawn(async move {
                match knowledge_search(query, 50).await { Ok(hits) => { knowledge_hits.set(hits); error.set(None); },
                    Err(err) => error.set(Some(err.to_string())) }
            }); },
            on_knowledge_upload: move |request| { spawn(async move {
                busy.set(true); error.set(knowledge_upload(request).await.err().map(|err| err.to_string()));
                busy.set(false); snapshot.restart();
            }); },
            on_knowledge_remove: move |id| { spawn(async move {
                error.set(knowledge_remove(id).await.err().map(|err| err.to_string())); snapshot.restart();
            }); },
            on_knowledge_get: move |id| { spawn(async move {
                match knowledge_get(id).await {
                    Ok(detail) => { knowledge_detail.set(detail); error.set(None); }
                    Err(err) => error.set(Some(err.to_string())),
                }
            }); },
            on_skill_refresh: move || { spawn(async move {
                error.set(skills_catalog(true).await.err().map(|err| err.to_string())); snapshot.restart();
            }); },
            on_skill_install: move |skill| { spawn(async move {
                error.set(skills_install(skill).await.err().map(|err| err.to_string())); snapshot.restart();
            }); },
            on_skill_uninstall: move |id| { spawn(async move {
                error.set(skills_uninstall(id).await.err().map(|err| err.to_string())); snapshot.restart();
            }); },
            on_activity_refresh: move || { spawn(async move {
                error.set(activity_tail(200).await.err().map(|err| err.to_string())); snapshot.restart();
            }); },
            on_history_search: move |query: String| { spawn(async move {
                let query = (!query.trim().is_empty()).then_some(query);
                match history_query(HistoryQuery { agent_id: selected_agent(), query, limit: 100 }).await {
                    Ok(rows) => { searched_history.set(rows); error.set(None); },
                    Err(err) => error.set(Some(err.to_string())),
                }
            }); },
        }
    } }
}

#[component]
fn Agents() -> Element {
    let selected_context = use_context::<Signal<SelectedAgentContext>>();
    let initial_agent_id = selected_context.read().agent_id.clone();
    rsx! { SelectedAgents { initial_agent_id } }
}

#[component]
fn SelectedAgents(initial_agent_id: Option<String>) -> Element {
    let mut selected = use_signal(|| initial_agent_id);
    let mut action_error = use_signal(|| None::<String>);
    let mut agents = use_resource(list_agents);
    let (rows, restorable_agents) = agents
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    if selected.read().is_none()
        && let Some(first) = rows.first()
    {
        selected.set(Some(first.id.clone()));
    }
    let view = PtyAgentsViewModel {
        agents: rows,
        restorable_agents,
        selected_agent_id: selected.read().clone(),
        loading: agents.read().is_none(),
        error_ja: action_error().or_else(|| {
            agents
                .read()
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map(|_| String::from("エージェント一覧を取得できませんでした。"))
        }),
        ..PtyAgentsViewModel::default()
    };
    let bridge_script = terminal_bridge_script(PTY_BRIDGE_JS);
    rsx! { section { class: "route-surface",
        button {
            id: "pty-runtime-refresh",
            r#type: "button",
            hidden: true,
            aria_hidden: "true",
            tabindex: "-1",
            onclick: move |_| agents.restart(),
        }
        document::Link { rel: "stylesheet", href: XTERM_CSS }
        document::Script { src: XTERM_JS }
        document::Script { src: XTERM_FIT_JS }
        document::Script { src: XTERM_UNICODE11_JS }
        document::Script { r#type: "module", {bridge_script} }
        PtyAgentsDomain { view, on_action: move |action: PtyAgentsAction| {
            match action {
                PtyAgentsAction::Select(agent_id) => selected.set(Some(agent_id)),
                PtyAgentsAction::Spawn(request) => { spawn(async move {
                    action_error.set(pty_spawn(request).await.err().map(|_| terminal_action_error()));
                    agents.restart();
                }); }
                PtyAgentsAction::Input { pty_id, data } => { spawn(async move {
                    action_error.set(pty_input(pty_id, data).await.err().map(|_| terminal_action_error()));
                }); }
                PtyAgentsAction::QueueMessage { agent_id, text } => { spawn(async move {
                    action_error.set(pty_queue(agent_id, text).await.err().map(|_| terminal_action_error()));
                }); }
                PtyAgentsAction::Presence { .. } => {
                    // The xterm bridge sends presence on the same ordered WebSocket as input.
                }
                PtyAgentsAction::Resize { pty_id, dimensions } => { spawn(async move {
                    action_error.set(pty_resize(pty_id, dimensions).await.err().map(|_| terminal_action_error()));
                }); }
                PtyAgentsAction::Redraw(pty_id) => { spawn(async move {
                    action_error.set(pty_redraw(pty_id).await.err().map(|_| terminal_action_error()));
                }); }
                PtyAgentsAction::Kill(pty_id) => { spawn(async move {
                    action_error.set(pty_kill(pty_id).await.err().map(|_| terminal_action_error()));
                    agents.restart();
                }); }
                PtyAgentsAction::Restart(request) => { spawn(async move {
                    action_error.set(pty_restart(request).await.err().map(|_| terminal_action_error()));
                    agents.restart();
                }); }
                PtyAgentsAction::Restore(request) => { spawn(async move {
                    action_error.set(pty_restore(request).await.err().map(|_| terminal_action_error()));
                    agents.restart();
                }); }
                PtyAgentsAction::RestoreAll => {
                    let requests = agents.read().as_ref().and_then(|result| result.as_ref().ok())
                        .map(|(_, rows)| rows.clone()).unwrap_or_default();
                    spawn(async move {
                        for agent in requests {
                            if pty_restore(md_web_contracts::domains::pty_agents::RestoreAgentRequest {
                                agent, prefer_worktree: true,
                            }).await.is_err() {
                                action_error.set(Some(terminal_action_error()));
                                break;
                            }
                        }
                        agents.restart();
                    });
                }
                PtyAgentsAction::Refresh => agents.restart(),
            }
        } }
    } }
}

fn terminal_action_error() -> String {
    String::from("ターミナル操作を完了できませんでした。設定と入力内容を確認してください。")
}

fn terminal_bridge_script(bridge: Asset) -> String {
    format!(
        r#"import {{ startTerminalBridge, applyServerFrame }} from '{bridge}';
globalThis.__mdTerminalRuntime?.stop?.();
const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
const endpoint = `${{protocol}}//${{location.host}}/ws/terminal`;
const attached = new Set();
const lastSeq = new Map();
const generations = new Map();
const controller = new AbortController();
let socket = null;
let reconnectTimer = null;
let reconnectDelay = 250;
let stopped = false;
let stopBridge = null;

function startBridgeWhenReady(attempt = 0) {{
  if (stopped) return;
  if (typeof globalThis.Terminal === 'function') {{
    stopBridge = startTerminalBridge();
    attachAll();
    return;
  }}
  if (attempt < 50) setTimeout(() => startBridgeWhenReady(attempt + 1), 20);
}}
startBridgeWhenReady();

function attachAll() {{
  if (!socket || socket.readyState !== WebSocket.OPEN) return;
  const mounted = new Set();
  document.querySelectorAll('.pty-terminal__xterm[data-pty-id]').forEach((element) => {{
    const ptyId = element.dataset.ptyId;
    if (!ptyId) return;
    mounted.add(ptyId);
    if (!attached.has(ptyId)) {{
      attached.add(ptyId);
      socket.send(JSON.stringify({{
        type: 'attach', data: {{ pty_id: ptyId, after_seq: lastSeq.get(ptyId) || 0 }}
      }}));
    }}
  }});
  for (const ptyId of [...attached]) {{
    if (!mounted.has(ptyId)) {{
      socket.send(JSON.stringify({{ type: 'detach', data: {{ pty_id: ptyId }} }}));
      attached.delete(ptyId);
    }}
  }}
}}

function scheduleReconnect() {{
  if (stopped || reconnectTimer !== null) return;
  reconnectTimer = setTimeout(() => {{ reconnectTimer = null; connect(); }}, reconnectDelay);
  reconnectDelay = Math.min(reconnectDelay * 2, 4000);
}}

function connect() {{
  if (stopped) return;
  socket = new WebSocket(endpoint);
  socket.addEventListener('open', () => {{
    reconnectDelay = 250;
    attached.clear();
    attachAll();
  }});
  socket.addEventListener('message', onMessage);
  socket.addEventListener('close', () => {{ attached.clear(); scheduleReconnect(); }});
  socket.addEventListener('error', () => socket?.close());
}}

function onMessage(event) {{
  let frame;
  try {{ frame = JSON.parse(event.data); }} catch {{ return; }}
  if (frame.type === 'attached' && frame.data?.pty?.id && Number.isInteger(frame.data.generation)) {{
    const ptyId = frame.data.pty.id;
    const previousGeneration = generations.get(ptyId);
    generations.set(ptyId, frame.data.generation);
    if (previousGeneration !== undefined && previousGeneration !== frame.data.generation) {{
      lastSeq.set(ptyId, 0);
      socket?.send(JSON.stringify({{
        type: 'attach', data: {{ pty_id: ptyId, after_seq: 0 }}
      }}));
    }}
  }}
  if (frame.data?.pty_id && Number.isInteger(frame.data.seq)) {{
    lastSeq.set(frame.data.pty_id, frame.data.seq);
  }}
  applyServerFrame(frame);
  if (frame.type === 'exited') {{
    document.getElementById('pty-runtime-refresh')?.click();
  }}
  if (frame.type === 'output' && frame.data?.pty_id) {{
    const root = document.querySelector(`.pty-terminal[data-pty-id="${{CSS.escape(frame.data.pty_id)}}"]`);
    const fallback = root?.querySelector('.pty-terminal__fallback');
    if (fallback) fallback.textContent += frame.data.data || '';
  }}
}}

function sendInput(event) {{
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({{
    type: 'input', data: {{ pty_id: event.detail.ptyId, data: event.detail.data }}
  }}));
}}

function sendResize(event) {{
  if (socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify({{
    type: 'resize', data: {{ pty_id: event.detail.ptyId,
      dimensions: {{ cols: event.detail.cols, rows: event.detail.rows }} }}
  }}));
}}

function sendPresence(event) {{
  if (socket?.readyState !== WebSocket.OPEN) return;
  const detail = event.detail || {{}};
  socket.send(JSON.stringify({{
    type: 'presence',
    data: {{
      pty_id: detail.ptyId,
      presence: {{
        draft_nonempty: Boolean(detail.draftNonempty),
        picker_open: Boolean(detail.pickerOpen),
        composing: Boolean(detail.composing),
        last_activity_at_ms: Math.min(Number(detail.lastActivityAtMs) || Date.now(), Date.now())
      }}
    }}
  }}));
}}

const observer = new MutationObserver(attachAll);
observer.observe(document.body, {{ childList: true, subtree: true }});
document.addEventListener('md-terminal-input', sendInput, {{ signal: controller.signal }});
document.addEventListener('md-terminal-resize', sendResize, {{ signal: controller.signal }});
document.addEventListener('md-terminal-presence', sendPresence, {{ signal: controller.signal }});
connect();

const runtime = {{
  stop() {{
    if (stopped) return;
    stopped = true;
    controller.abort();
    observer.disconnect();
    stopBridge?.();
    if (reconnectTimer !== null) clearTimeout(reconnectTimer);
    if (socket?.readyState === WebSocket.OPEN) {{
      for (const ptyId of attached) {{
        socket.send(JSON.stringify({{ type: 'detach', data: {{ pty_id: ptyId }} }}));
      }}
    }}
    attached.clear();
    socket?.close();
  }}
}};
globalThis.__mdTerminalRuntime = runtime;
addEventListener('pagehide', () => runtime.stop(), {{ once: true }});"#
    )
}

#[component]
fn Voice() -> Element {
    rsx! { section { class: "route-surface", VoiceRealtimeDomain {} } }
}

#[component]
fn NotFound(route: Vec<String>) -> Element {
    rsx! { section { class: "route-surface", h1 { "ページが見つかりません" }
        p { "指定されたパスは利用できません。" }
        Link { to: AppRoute::Office {}, "オフィスへ戻る" }
        span { hidden: true, {route.join("/")} }
    } }
}

#[cfg(test)]
mod tests {
    use super::{AppRoute, nav_items};

    #[test]
    fn enabled_routes_have_distinct_paths() {
        let enabled = nav_items()
            .into_iter()
            .filter(|item| item.enabled)
            .map(|item| item.route.to_string())
            .collect::<Vec<_>>();
        for (index, path) in enabled.iter().enumerate() {
            assert!(enabled[..index].iter().all(|previous| previous != path));
        }
        assert!(enabled.contains(&AppRoute::Connections {}.to_string()));
    }
}
