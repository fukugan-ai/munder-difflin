use std::collections::BTreeMap;

use dioxus::prelude::*;
use md_web_contracts::domains::pty_agents::{
    AgentProvider, AgentRecord, AgentRole, AgentStatus, RestartAgentRequest, RestoreAgentRequest,
    SpawnAgentRequest, TerminalPresence,
};

use super::view_model::{PtyAgentsAction, PtyAgentsViewModel};

const DEFAULT_COLS: u16 = 100;
const DEFAULT_ROWS: u16 = 30;

/// Local agent roster and terminal workspace. Network/process work is delegated through `on_action`.
#[component]
pub fn PtyAgentsDomain(
    view: PtyAgentsViewModel,
    on_action: EventHandler<PtyAgentsAction>,
) -> Element {
    let mut name = use_signal(String::new);
    let mut cwd = use_signal(String::new);
    let mut command = use_signal(|| String::from("codex"));
    let mut model = use_signal(String::new);
    let mut resume_session_id = use_signal(String::new);
    let mut isolate = use_signal(|| true);
    let terminal_inputs = use_signal(BTreeMap::<String, String>::new);
    let selected = view.selected_agent().cloned();
    let state = if view.loading {
        "loading"
    } else if view.error_ja.is_some() {
        "error"
    } else {
        "success"
    };

    rsx! {
        section {
            class: "pty-agents",
            aria_labelledby: "pty-agents-title",
            "data-ui-state": state,
            header { class: "pty-agents__header",
                div {
                    h1 { id: "pty-agents-title", "エージェントとターミナル" }
                    p { "ローカルのCLIエージェントを起動し、作業をそのまま確認できます。" }
                }
                button {
                    class: "pty-agent__button pty-agent__button--quiet",
                    r#type: "button",
                    disabled: view.loading,
                    "data-ui-state": state,
                    onclick: move |_| on_action.call(PtyAgentsAction::Refresh),
                    if view.loading { "読込中…" } else { "再読み込み" }
                }
            }

            if let Some(error) = &view.error_ja {
                div { class: "pty-agents__notice", role: "alert",
                    strong { "ターミナル操作に失敗しました" }
                    span { {error.as_str()} }
                }
            }

            div { class: "pty-agents__layout",
                aside { class: "pty-agents__roster", aria_label: "エージェント一覧",
                    div { class: "pty-agents__roster-heading",
                        h2 { "稼働中" }
                        span { "{view.agents.len()}名" }
                    }
                    if view.agents.is_empty() {
                        div { class: "pty-agents__empty",
                            strong { "エージェントがいません" }
                            p { "下のフォームから最初のエージェントを起動できます。" }
                        }
                    } else {
                        div { class: "pty-agents__agent-list",
                            for agent in &view.agents {
                                {agent_button(
                                    agent,
                                    view.selected_agent_id.as_deref(),
                                    agent.pty_id.as_deref().is_some_and(|pty_id| view.terminal_is_busy(pty_id)),
                                    on_action,
                                )}
                            }
                        }
                    }

                    if !view.restorable_agents.is_empty() {
                        div { class: "pty-agents__restore",
                            div { class: "pty-agents__roster-heading",
                                h2 { "復元可能" }
                                button {
                                    class: "pty-agent__button pty-agent__button--quiet",
                                    r#type: "button",
                                    onclick: move |_| on_action.call(PtyAgentsAction::RestoreAll),
                                    "すべて復元"
                                }
                            }
                            for agent in &view.restorable_agents {
                                article { class: "pty-agent-card pty-agent-card--restorable",
                                    strong { {agent.name.as_str()} }
                                    span { {agent.cwd.as_str()} }
                                    button {
                                        class: "pty-agent__button pty-agent__button--quiet",
                                        r#type: "button",
                                        onclick: {
                                            let agent = agent.clone();
                                            move |_| on_action.call(PtyAgentsAction::Restore(
                                                RestoreAgentRequest { agent: agent.clone(), prefer_worktree: true }
                                            ))
                                        },
                                        "復元"
                                    }
                                }
                            }
                        }
                    }
                }

                div { class: "pty-agents__workspace",
                    match selected {
                        Some(agent) => rsx! {
                            {terminal_workspace(&view, agent, terminal_inputs, on_action)}
                        },
                        None => rsx! {
                            div { class: "pty-agents__empty pty-agents__empty--workspace",
                                strong { "ターミナルを選んでください" }
                                p { "稼働中のエージェントを選ぶと、出力と入力欄がここに表示されます。" }
                            }
                        },
                    }
                }
            }

            div { class: "pty-agents__hire",
                div { class: "pty-agents__hire-copy",
                    h2 { "エージェントを追加" }
                    p { "名前、作業フォルダー、実行するCLIを指定します。" }
                }
                div { class: "pty-agents__hire-fields",
                    label { class: "pty-agent__field",
                        span { "名前" }
                        input {
                            class: "pty-agent__input",
                            r#type: "text",
                            value: "{name}",
                            placeholder: "Dev 1",
                            autocomplete: "off",
                            oninput: move |event| name.set(event.value()),
                        }
                        small { "一覧に表示する名前です。" }
                    }
                    label { class: "pty-agent__field",
                        span { "作業フォルダー" }
                        input {
                            class: "pty-agent__input",
                            r#type: "text",
                            value: "{cwd}",
                            placeholder: "/home/user/project",
                            autocomplete: "off",
                            oninput: move |event| cwd.set(event.value()),
                        }
                        small { "サーバー上に存在する絶対パスを指定します。" }
                    }
                    label { class: "pty-agent__field",
                        span { "CLIコマンド" }
                        input {
                            class: "pty-agent__input",
                            r#type: "text",
                            value: "{command}",
                            placeholder: "codex",
                            autocomplete: "off",
                            oninput: move |event| command.set(event.value()),
                        }
                        small { "シェル文字列ではなく、実行ファイル名を指定します。" }
                    }
                    label { class: "pty-agent__field",
                        span { "モデル（任意）" }
                        input {
                            class: "pty-agent__input",
                            r#type: "text",
                            value: "{model}",
                            placeholder: "未指定ならCLIの既定値",
                            autocomplete: "off",
                            oninput: move |event| model.set(event.value()),
                        }
                        small { "CLIに渡すモデルIDです。" }
                    }
                    label { class: "pty-agent__field",
                        span { "再開するセッションID（任意）" }
                        input {
                            class: "pty-agent__input",
                            r#type: "text",
                            value: "{resume_session_id}",
                            placeholder: "session-id",
                            autocomplete: "off",
                            oninput: move |event| resume_session_id.set(event.value()),
                        }
                        small { "見つからない場合は現在のプロセスを置き換えません。" }
                    }
                    label { class: "pty-agent__check",
                        input {
                            r#type: "checkbox",
                            checked: *isolate.read(),
                            onchange: move |event| isolate.set(event.checked()),
                        }
                        span { "Git worktreeで分離する" }
                    }
                    button {
                        class: "pty-agent__button pty-agent__button--primary",
                        r#type: "button",
                        disabled: name.read().trim().is_empty()
                            || cwd.read().trim().is_empty()
                            || command.read().trim().is_empty()
                            || view.loading,
                        "data-ui-state": if view.loading { "loading" } else { "default" },
                        onclick: move |_| {
                            let agent_name = name.read().trim().to_owned();
                            let agent_cwd = cwd.read().trim().to_owned();
                            let agent_command = command.read().trim().to_owned();
                            let agent_model = model.read().trim().to_owned();
                            let session_id = resume_session_id.read().trim().to_owned();
                            if agent_name.is_empty() || agent_cwd.is_empty() || agent_command.is_empty() {
                                return;
                            }
                            let id = slug_id(&agent_name);
                            on_action.call(PtyAgentsAction::Spawn(SpawnAgentRequest {
                                id,
                                name: agent_name,
                                provider: provider_for_command(&agent_command),
                                role: AgentRole::default(),
                                description: String::from("ローカルCLIエージェント"),
                                cwd: agent_cwd,
                                command: agent_command,
                                args: Vec::new(),
                                model: (!agent_model.is_empty()).then_some(agent_model),
                                cols: DEFAULT_COLS,
                                rows: DEFAULT_ROWS,
                                isolate: *isolate.read(),
                                resume: !session_id.is_empty(),
                                require_resume: !session_id.is_empty(),
                                resume_session_id: (!session_id.is_empty()).then_some(session_id),
                            }));
                            name.set(String::new());
                        },
                        if view.loading { "起動中…" } else { "エージェントを起動" }
                    }
                }
            }
        }
    }
}

fn agent_button(
    agent: &AgentRecord,
    selected_id: Option<&str>,
    terminal_busy: bool,
    on_action: EventHandler<PtyAgentsAction>,
) -> Element {
    let selected = selected_id == Some(agent.id.as_str());
    let agent_id = agent.id.clone();
    rsx! {
        button {
            class: if selected { "pty-agent-card is-selected" } else { "pty-agent-card" },
            r#type: "button",
            aria_pressed: selected,
            onclick: move |_| on_action.call(PtyAgentsAction::Select(agent_id.clone())),
            span { class: "pty-agent-card__status", "data-status": status_key(agent.status), aria_hidden: "true" }
            span { class: "pty-agent-card__copy",
                strong { {agent.name.as_str()} }
                small {
                    if terminal_busy { "処理中" } else { {status_label(agent.status)} }
                    " · " {agent.action_ja.as_str()}
                }
            }
        }
    }
}

fn terminal_workspace(
    view: &PtyAgentsViewModel,
    agent: AgentRecord,
    mut terminal_inputs: Signal<BTreeMap<String, String>>,
    on_action: EventHandler<PtyAgentsAction>,
) -> Element {
    let Some(pty_id) = agent.pty_id.clone() else {
        return rsx! {
            div { class: "pty-agents__empty pty-agents__empty--workspace",
                strong { "このエージェントにターミナルはありません" }
                p { "アーカイブを復元するか、新しいプロセスとして起動してください。" }
            }
        };
    };
    let terminal_text = view.terminal_text(&pty_id);
    let draft = terminal_inputs
        .read()
        .get(&agent.id)
        .cloned()
        .unwrap_or_default();
    let can_send = !draft.trim().is_empty();
    rsx! {
        article { class: "pty-terminal", "data-pty-id": "{pty_id}",
            header { class: "pty-terminal__header",
                div {
                    h2 { {agent.name.as_str()} }
                    p { {agent.cwd.as_str()} }
                }
                div { class: "pty-terminal__actions",
                    button {
                        class: "pty-agent__button pty-agent__button--quiet",
                        r#type: "button",
                        onclick: {
                            let agent_id = agent.id.clone();
                            let provider = agent.provider;
                            let model = agent.model.clone();
                            move |_| on_action.call(PtyAgentsAction::Restart(RestartAgentRequest {
                                agent_id: agent_id.clone(),
                                provider,
                                model: model.clone(),
                                resume: false,
                                require_resume: false,
                            }))
                        },
                        "再起動"
                    }
                    button {
                        class: "pty-agent__button pty-agent__button--quiet",
                        r#type: "button",
                        onclick: {
                            let pty_id = pty_id.clone();
                            move |_| on_action.call(PtyAgentsAction::Redraw(pty_id.clone()))
                        },
                        "再描画"
                    }
                    button {
                        class: "pty-agent__button pty-agent__button--quiet",
                        r#type: "button",
                        disabled: agent.session_id.is_none(),
                        title: if agent.session_id.is_none() { "再開できるセッションIDがありません" } else { "同じ会話を再開" },
                        onclick: {
                            let agent_id = agent.id.clone();
                            let provider = agent.provider;
                            let model = agent.model.clone();
                            move |_| on_action.call(PtyAgentsAction::Restart(RestartAgentRequest {
                                agent_id: agent_id.clone(),
                                provider,
                                model: model.clone(),
                                resume: true,
                                require_resume: true,
                            }))
                        },
                        "再起動して続行"
                    }
                    button {
                        class: "pty-agent__button pty-agent__button--danger",
                        r#type: "button",
                        onclick: {
                            let pty_id = pty_id.clone();
                            move |_| on_action.call(PtyAgentsAction::Kill(pty_id.clone()))
                        },
                        "終了"
                    }
                }
            }
            div {
                class: "pty-terminal__surface",
                role: "log",
                aria_live: "polite",
                aria_label: format!("{}のターミナル出力", agent.name),
                div {
                    class: "pty-terminal__xterm",
                    "data-pty-id": "{pty_id}",
                    "data-terminal-generation": "0",
                    aria_hidden: "true",
                }
                pre { class: "pty-terminal__fallback", {terminal_text} }
            }
            label { class: "pty-agent__field pty-terminal__composer",
                span { "エージェントへ送る" }
                textarea {
                    class: "pty-agent__input pty-agent__input--composer",
                    rows: "3",
                    value: "{draft}",
                    placeholder: "作業内容を入力",
                    oninput: {
                        let agent_id = agent.id.clone();
                        let pty_id = pty_id.clone();
                        move |event| {
                            let value = event.value();
                            terminal_inputs.write().insert(agent_id.clone(), value.clone());
                            on_action.call(PtyAgentsAction::Presence {
                                pty_id: pty_id.clone(),
                                presence: TerminalPresence {
                                    draft_nonempty: !value.trim().is_empty(),
                                    ..TerminalPresence::default()
                                },
                            });
                        }
                    },
                    oncompositionstart: {
                        let pty_id = pty_id.clone();
                        move |_| on_action.call(PtyAgentsAction::Presence {
                            pty_id: pty_id.clone(),
                            presence: TerminalPresence {
                                draft_nonempty: true,
                                composing: true,
                                ..TerminalPresence::default()
                            },
                        })
                    },
                    oncompositionend: {
                        let pty_id = pty_id.clone();
                        let agent_id = agent.id.clone();
                        move |_| on_action.call(PtyAgentsAction::Presence {
                            pty_id: pty_id.clone(),
                            presence: TerminalPresence {
                                draft_nonempty: terminal_inputs
                                    .read()
                                    .get(&agent_id)
                                    .is_some_and(|value| !value.trim().is_empty()),
                                ..TerminalPresence::default()
                            },
                        })
                    },
                }
                small { "エージェントが作業中のときは、送信内容をキューに保管します。" }
            }
            button {
                class: "pty-agent__button pty-agent__button--primary pty-terminal__send",
                r#type: "button",
                disabled: !can_send,
                onclick: {
                    let agent_id = agent.id;
                    move |_| {
                        let text = terminal_inputs
                            .read()
                            .get(&agent_id)
                            .map(|value| value.trim().to_owned())
                            .unwrap_or_default();
                        if text.is_empty() {
                            return;
                        }
                        on_action.call(PtyAgentsAction::QueueMessage {
                            agent_id: agent_id.clone(),
                            text,
                        });
                        terminal_inputs.write().remove(&agent_id);
                        on_action.call(PtyAgentsAction::Presence {
                            pty_id: pty_id.clone(),
                            presence: TerminalPresence::default(),
                        });
                    }
                },
                "送信"
            }
        }
    }
}

fn status_key(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Idle => "idle",
        AgentStatus::Working | AgentStatus::Starting => "working",
        AgentStatus::Waiting | AgentStatus::Blocked => "blocked",
        AgentStatus::Looping => "looping",
        AgentStatus::Exited | AgentStatus::Archived | AgentStatus::Restorable => "offline",
    }
}

fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Starting => "起動中",
        AgentStatus::Idle => "待機中",
        AgentStatus::Working => "作業中",
        AgentStatus::Waiting => "入力待ち",
        AgentStatus::Blocked => "ブロック中",
        AgentStatus::Looping => "制限中",
        AgentStatus::Exited => "終了",
        AgentStatus::Archived => "アーカイブ済み",
        AgentStatus::Restorable => "復元可能",
    }
}

fn provider_for_command(command: &str) -> AgentProvider {
    let executable = command
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(command)
        .to_ascii_lowercase();
    match executable.as_str() {
        "claude" | "claude.exe" => AgentProvider::Claude,
        "codex" | "codex.exe" => AgentProvider::Codex,
        "grok" | "grok.exe" => AgentProvider::Grok,
        "kimi" | "kimi.exe" => AgentProvider::Kimi,
        "gemini" | "gemini.exe" => AgentProvider::Gemini,
        "agy" | "agy.exe" => AgentProvider::Antigravity,
        "qwen" | "qwen.exe" => AgentProvider::Qwen,
        "opencode" | "opencode.exe" => AgentProvider::OpenCode,
        "crush" | "crush.exe" => AgentProvider::Crush,
        "pi" | "pi.exe" => AgentProvider::Pi,
        "copilot" | "copilot.exe" => AgentProvider::Copilot,
        "cursor" | "cursor.exe" => AgentProvider::Cursor,
        _ => AgentProvider::Custom,
    }
}

fn slug_id(name: &str) -> String {
    let mut id = String::with_capacity(name.len());
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
            id.push(character.to_ascii_lowercase());
            separator = false;
        } else if !separator && !id.is_empty() {
            id.push('-');
            separator = true;
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        String::from("agent")
    } else {
        id
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{AgentProvider, AgentStatus};

    use super::{provider_for_command, slug_id, status_label};

    #[test]
    fn provider_inference_handles_absolute_codex_path() {
        assert_eq!(
            provider_for_command("/usr/local/bin/codex"),
            AgentProvider::Codex
        );
    }

    #[test]
    fn slug_rejects_path_separators() {
        assert_eq!(slug_id("Dev/../../One"), "dev-one");
    }

    #[test]
    fn blocked_status_has_japanese_label() {
        assert_eq!(status_label(AgentStatus::Blocked), "ブロック中");
    }
}
