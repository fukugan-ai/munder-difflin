use std::collections::BTreeMap;

use dioxus::prelude::*;
use md_web_contracts::{
    AgentControlSnapshot, HiveMessage, HiveTask, PreservedWorktreeSnapshot, TaskStatus,
    WorkerSnapshot, WorkerStatus,
};

use super::view_model::{ControlAction, HiveTasksViewModel, MessageAction, TaskAction};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum HiveInitialTab {
    #[default]
    Tasks,
    AskMe,
    Threads,
    Control,
    Workers,
}

const TASK_COLUMNS: [(TaskStatus, &str); 4] = [
    (TaskStatus::Todo, "TODO"),
    (TaskStatus::Doing, "進行中"),
    (TaskStatus::Blocked, "ブロック中"),
    (TaskStatus::Done, "完了"),
];

/// Full local coordination surface. Network calls stay in the parent route/server adapter.
#[component]
pub fn HiveTasksDomain(
    view: HiveTasksViewModel,
    #[props(default)] initial_tab: HiveInitialTab,
    on_task: EventHandler<TaskAction>,
    on_message: EventHandler<MessageAction>,
    on_control: EventHandler<ControlAction>,
    on_stop_worker: EventHandler<String>,
    on_refresh: EventHandler<()>,
) -> Element {
    let tab = use_signal(|| initial_tab);
    let state = if view.loading {
        "loading"
    } else if view.error.is_some() {
        "error"
    } else {
        "success"
    };

    rsx! {
        section {
            class: "hive-domain",
            "data-testid": "hive-domain",
            aria_labelledby: "hive-domain-title",
            "data-ui-state": state,
            header { class: "hive-domain__header",
                div {
                    h1 { id: "hive-domain-title", "チーム運営" }
                    p { "タスク、会話、確認事項、ワーカーをローカルで管理します。" }
                }
                button {
                    class: "hive-button hive-button--quiet",
                    r#type: "button",
                    disabled: view.loading,
                    "data-ui-state": state,
                    aria_label: "チーム情報を再読み込み",
                    onclick: move |_| on_refresh.call(()),
                    if view.loading { "読込中…" } else { "再読み込み" }
                }
            }

            nav { class: "hive-tabs", aria_label: "チーム運営メニュー",
                {tab_button("タスク", HiveInitialTab::Tasks, *tab.read(), tab)}
                {tab_button("確認事項", HiveInitialTab::AskMe, *tab.read(), tab)}
                {tab_button("スレッド", HiveInitialTab::Threads, *tab.read(), tab)}
                {tab_button("操作", HiveInitialTab::Control, *tab.read(), tab)}
                {tab_button("ワーカー", HiveInitialTab::Workers, *tab.read(), tab)}
            }

            if let Some(error) = &view.error {
                div { class: "hive-notice hive-notice--error", role: "alert",
                    strong { "読み込みに失敗しました" }
                    span { {error.as_str()} }
                }
            }

            div { class: "hive-domain__body",
                details { class: "hive-runtime",
                    summary { "共有ボード・ログ・メモリ" }
                    section { h2 { "board.md" }
                        pre { {view.board.as_str()} }
                    }
                    if let Some(memory) = &view.selected_memory {
                        section { h2 { "選択中エージェントのmemory.md" }
                            pre { {memory.as_str()} }
                        }
                    }
                    section { h2 { "最新ログ" }
                        pre { {serde_json::to_string_pretty(&view.log_tail).unwrap_or_default()} }
                    }
                }
                match *tab.read() {
                    HiveInitialTab::Tasks => rsx! { TasksView { tasks: view.tasks, on_task } },
                    HiveInitialTab::AskMe => rsx! { AskMeView { tasks: view.tasks, on_task } },
                    HiveInitialTab::Threads => rsx! { ThreadsView {
                        messages: view.messages,
                        on_message,
                    } },
                    HiveInitialTab::Control => rsx! { ControlView {
                        agents: view.agents,
                        selected_agent_id: view.selected_agent_id,
                        snapshot: view.selected_control,
                        on_control,
                    } },
                    HiveInitialTab::Workers => rsx! { WorkersView {
                        workers: view.workers,
                        preserved: view.preserved_worktrees,
                        max_workers: view.max_workers,
                        on_stop_worker,
                    } },
                }
            }
        }
    }
}

fn tab_button(
    label: &'static str,
    target: HiveInitialTab,
    current: HiveInitialTab,
    mut tab: Signal<HiveInitialTab>,
) -> Element {
    let selected = target == current;
    rsx! {
        button {
            class: "hive-tab",
            r#type: "button",
            role: "tab",
            aria_selected: selected,
            "data-ui-state": if selected { "success" } else { "default" },
            onclick: move |_| tab.set(target),
            {label}
        }
    }
}

#[component]
fn TasksView(tasks: Vec<HiveTask>, on_task: EventHandler<TaskAction>) -> Element {
    rsx! {
        TaskComposer { on_task }
        div { class: "task-board", aria_label: "タスクボード",
            for (status, label) in TASK_COLUMNS {
                section { class: "task-column", "data-status": status_key(status),
                    header { class: "task-column__header",
                        h2 { {label} }
                        span { aria_label: "タスク件数",
                            {tasks.iter().filter(|task| task.status == status).count().to_string()}
                        }
                    }
                    div { class: "task-column__cards",
                        for task in tasks.iter().filter(|task| task.status == status) {
                            TaskCard { task: task.clone(), on_task }
                        }
                        if !tasks.iter().any(|task| task.status == status) {
                            p { class: "hive-empty", "タスクはありません" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TaskComposer(on_task: EventHandler<TaskAction>) -> Element {
    let mut title = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut assignee = use_signal(String::new);
    rsx! { form { class: "hive-composer", onsubmit: move |event| {
        event.prevent_default();
        let value = title.read().trim().to_owned();
        if !value.is_empty() {
            on_task.call(TaskAction::Create {
                title: value,
                description: Some(description.read().trim().to_owned()),
                assignee: Some(assignee.read().trim().to_owned()),
                priority: 1,
            });
            title.set(String::new()); description.set(String::new()); assignee.set(String::new());
        }
    },
        h2 { "タスクを作成" }
        input { aria_label: "タスク名", placeholder: "タスク名", value: "{title}", oninput: move |e| title.set(e.value()) }
        input { aria_label: "説明", placeholder: "説明", value: "{description}", oninput: move |e| description.set(e.value()) }
        input { aria_label: "担当ID", placeholder: "担当ID（任意）", value: "{assignee}", oninput: move |e| assignee.set(e.value()) }
        button { class: "hive-button hive-button--primary", r#type: "submit", "作成" }
    } }
}

#[component]
fn TaskCard(task: HiveTask, on_task: EventHandler<TaskAction>) -> Element {
    let task_id = task.id.clone();
    let delete_id = task.id.clone();
    rsx! {
        article { class: "task-card",
            div { class: "task-card__topline",
                span { class: "task-priority", "優先度 {task.priority}" }
                button {
                    class: "hive-icon-button",
                    r#type: "button",
                    aria_label: "{task.title}を削除",
                    title: "タスクを削除",
                    onclick: move |_| on_task.call(TaskAction::Delete { task_id: delete_id.clone() }),
                    "×"
                }
            }
            h3 { {task.title.as_str()} }
            if let Some(description) = &task.description {
                p { {description.as_str()} }
            }
            if let Some(assignee) = &task.assignee {
                span { class: "task-assignee", "担当: {assignee}" }
            }
            if task.waits_on_human() {
                span { class: "task-human-badge", "回答待ち" }
            }
            label { class: "task-move",
                span { "移動先" }
                select {
                    value: status_key(task.status),
                    onchange: move |event| {
                        if let Some(status) = parse_status(&event.value()) {
                            on_task.call(TaskAction::Move { task_id: task_id.clone(), status });
                        }
                    },
                    for (status, label) in TASK_COLUMNS {
                        option { value: status_key(status), {label} }
                    }
                }
            }
        }
    }
}

#[component]
fn AskMeView(tasks: Vec<HiveTask>, on_task: EventHandler<TaskAction>) -> Element {
    let waiting: Vec<HiveTask> = tasks.into_iter().filter(HiveTask::waits_on_human).collect();
    rsx! {
        div { class: "ask-list",
            if waiting.is_empty() {
                div { class: "hive-empty hive-empty--large",
                    h2 { "回答待ちはありません" }
                    p { "チームが判断を必要とすると、ここに表示されます。" }
                }
            }
            for task in waiting {
                AskCard { task, on_task }
            }
        }
    }
}

#[component]
fn AskCard(task: HiveTask, on_task: EventHandler<TaskAction>) -> Element {
    let mut answer = use_signal(String::new);
    let Some(question) = task.open_question() else {
        return rsx! {};
    };
    let task_id = task.id.clone();
    let dismiss_id = task.id.clone();
    rsx! {
        article { class: "ask-card",
            header {
                div {
                    h2 { {task.title.as_str()} }
                    if let Some(assignee) = &task.assignee {
                        span { "担当: {assignee}" }
                    }
                }
                button {
                    class: "hive-button hive-button--quiet",
                    r#type: "button",
                    onclick: move |_| on_task.call(TaskAction::DismissQuestion { task_id: dismiss_id.clone() }),
                    "今回は閉じる"
                }
            }
            p { class: "ask-card__question", {question.q.as_str()} }
            label { class: "hive-field",
                span { "回答" }
                textarea {
                    rows: "4",
                    value: "{answer}",
                    placeholder: "判断や完了結果を入力してください",
                    oninput: move |event| answer.set(event.value()),
                }
            }
            button {
                class: "hive-button hive-button--primary",
                r#type: "button",
                disabled: answer.read().trim().is_empty(),
                "data-ui-state": if answer.read().trim().is_empty() { "disabled" } else { "default" },
                onclick: move |_| {
                    let text = answer.read().trim().to_owned();
                    if !text.is_empty() {
                        on_task.call(TaskAction::Answer { task_id: task_id.clone(), answer: text });
                        answer.set(String::new());
                    }
                },
                "回答を送る"
            }
        }
    }
}

#[component]
fn ThreadsView(messages: Vec<HiveMessage>, on_message: EventHandler<MessageAction>) -> Element {
    let threads = group_messages(messages);
    rsx! {
        div { class: "thread-list",
            ThreadComposer { on_message }
            if threads.is_empty() {
                div { class: "hive-empty hive-empty--large", "会話はまだありません。" }
            }
            for (conversation, messages) in threads {
                ThreadCard { conversation, messages, on_message }
            }
        }
    }
}

#[component]
fn ThreadCard(
    conversation: String,
    messages: Vec<HiveMessage>,
    on_message: EventHandler<MessageAction>,
) -> Element {
    let mut draft = use_signal(String::new);
    let subject = messages
        .first()
        .map_or("件名なし", |message| message.subject.as_str());
    rsx! {
        details { class: "thread-card", open: true,
            summary {
                strong { {subject} }
                span { "{messages.len()}件" }
            }
            div { class: "thread-card__messages",
                for message in messages {
                    article { class: "thread-message",
                        header {
                            strong { {message.from.as_str()} }
                            span { {act_label(message.act)} }
                            time { {message.created_at.as_str()} }
                        }
                        p { {message.body.as_str()} }
                    }
                }
            }
            div { class: "thread-reply",
                label { class: "hive-field",
                    span { "返信" }
                    textarea {
                        rows: "3",
                        value: "{draft}",
                        oninput: move |event| draft.set(event.value()),
                    }
                }
                button {
                    class: "hive-button hive-button--primary",
                    r#type: "button",
                    disabled: draft.read().trim().is_empty(),
                    onclick: move |_| {
                        let text = draft.read().trim().to_owned();
                        if !text.is_empty() {
                            on_message.call(MessageAction::Reply {
                                conversation: conversation.clone(), body: text,
                            });
                            draft.set(String::new());
                        }
                    },
                    "返信する"
                }
            }
        }
    }
}

#[component]
fn ThreadComposer(on_message: EventHandler<MessageAction>) -> Element {
    let mut subject = use_signal(String::new);
    let mut body = use_signal(String::new);
    rsx! { form { class: "hive-composer", onsubmit: move |event| {
        event.prevent_default();
        let next_subject = subject.read().trim().to_owned();
        let next_body = body.read().trim().to_owned();
        if !next_subject.is_empty() && !next_body.is_empty() {
            on_message.call(MessageAction::NewThread { subject: next_subject, body: next_body });
            subject.set(String::new()); body.set(String::new());
        }
    },
        h2 { "新しいスレッド" }
        input { aria_label: "件名", placeholder: "件名", value: "{subject}", oninput: move |e| subject.set(e.value()) }
        textarea { aria_label: "本文", placeholder: "本文", value: "{body}", oninput: move |e| body.set(e.value()) }
        button { class: "hive-button hive-button--primary", r#type: "submit", "開始" }
    } }
}

#[component]
fn ControlView(
    agents: Vec<md_web_contracts::HiveAgent>,
    selected_agent_id: Option<String>,
    snapshot: AgentControlSnapshot,
    on_control: EventHandler<ControlAction>,
) -> Element {
    let mut steer = use_signal(String::new);
    let mut role = use_signal(String::new);
    let selected = selected_agent_id
        .as_deref()
        .and_then(|id| agents.iter().find(|agent| agent.id == id));
    let Some(agent) = selected else {
        return rsx! {
            div { class: "hive-empty hive-empty--large", "操作するエージェントを選択してください。" }
        };
    };
    let agent_id = agent.id.clone();
    let role_id = agent.id.clone();
    let hold_id = agent.id.clone();
    let held = agent.on_hold;
    let pause_id = agent.id.clone();
    let delivery_id = agent.id.clone();
    let resume_id = agent.id.clone();
    let halt_id = agent.id.clone();
    rsx! {
        section { class: "control-panel",
            header {
                div {
                    h2 { {agent.name.as_str()} }
                    p { "{agent.role} · {agent.status}" }
                }
                span { class: "control-state", "メモ待機 {snapshot.pending_steers}件" }
            }
            div { class: "control-actions",
                label { class: "hive-field", span { "役割" }
                    input { value: "{role}", placeholder: "{agent.role}", oninput: move |e| role.set(e.value()) }
                }
                button { class: "hive-button", r#type: "button", onclick: move |_| {
                    let value = role.read().trim().to_owned();
                    if !value.is_empty() { on_control.call(ControlAction::PatchRole { agent_id: role_id.clone(), role: value }); }
                }, "役割を保存" }
                button { class: "hive-button", r#type: "button", onclick: move |_| {
                    on_control.call(ControlAction::SetHold { agent_id: hold_id.clone(), on: !held });
                }, if held { "保留を解除" } else { "1:1保留" } }
                button {
                    class: "hive-button",
                    r#type: "button",
                    "data-ui-state": if snapshot.paused { "success" } else { "default" },
                    onclick: move |_| on_control.call(ControlAction::Pause {
                        agent_id: pause_id.clone(),
                        on: !snapshot.paused,
                    }),
                    if snapshot.paused { "一時停止中" } else { "一時停止" }
                }
                button {
                    class: "hive-button",
                    r#type: "button",
                    "data-ui-state": if snapshot.auto_delivery_paused { "success" } else { "default" },
                    onclick: move |_| on_control.call(ControlAction::AutoDelivery {
                        agent_id: delivery_id.clone(),
                        paused: !snapshot.auto_delivery_paused,
                    }),
                    if snapshot.auto_delivery_paused { "自動配信を再開" } else { "自動配信を保留" }
                }
                button {
                    class: "hive-button hive-button--quiet",
                    r#type: "button",
                    onclick: move |_| on_control.call(ControlAction::Resume { agent_id: resume_id.clone() }),
                    "再開"
                }
                button {
                    class: "hive-button hive-button--danger",
                    r#type: "button",
                    onclick: move |_| on_control.call(ControlAction::Halt { agent_id: halt_id.clone() }),
                    "安全に停止"
                }
            }
            label { class: "hive-field",
                span { "次の区切りで渡すメモ" }
                textarea {
                    rows: "3",
                    value: "{steer}",
                    oninput: move |event| steer.set(event.value()),
                }
            }
            button {
                class: "hive-button hive-button--primary",
                r#type: "button",
                disabled: steer.read().trim().is_empty(),
                onclick: move |_| {
                    let text = steer.read().trim().to_owned();
                    if !text.is_empty() {
                        on_control.call(ControlAction::Steer { agent_id: agent_id.clone(), text });
                        steer.set(String::new());
                    }
                },
                "メモを送る"
            }
        }
    }
}

#[component]
fn WorkersView(
    workers: Vec<WorkerSnapshot>,
    preserved: Vec<PreservedWorktreeSnapshot>,
    max_workers: usize,
    on_stop_worker: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "workers-view",
            header { class: "workers-view__header",
                h2 { "稼働中のワーカー" }
                span { "{workers.len()} / {max_workers}" }
            }
            if workers.is_empty() {
                p { class: "hive-empty", "現在実行中のワーカーはありません。" }
            }
            for worker in workers {
                WorkerCard { worker, on_stop_worker }
            }
            if !preserved.is_empty() {
                section { class: "preserved-worktrees",
                    h2 { "保持中のworktree" }
                    p { "未統合の作業を破棄せず保持しています。" }
                    for item in preserved {
                        article {
                            strong { {item.worker_id.as_str()} }
                            code { {item.worktree_path.as_str()} }
                            span { "ベース: {item.base_branch}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn WorkerCard(worker: WorkerSnapshot, on_stop_worker: EventHandler<String>) -> Element {
    let worker_id = worker.worker_id.clone();
    let releasing = worker.status == WorkerStatus::Releasing;
    rsx! {
        article { class: "worker-card", "data-ui-state": if releasing { "loading" } else { "success" },
            header {
                div {
                    span { class: "worker-status", if releasing { "停止中" } else { "稼働中" } }
                    h3 { {worker.name.as_str()} }
                }
                button {
                    class: "hive-button hive-button--danger",
                    r#type: "button",
                    disabled: releasing,
                    onclick: move |_| on_stop_worker.call(worker_id.clone()),
                    if releasing { "停止中…" } else { "停止" }
                }
            }
            dl {
                div { dt { "ID" } dd { {worker.worker_id.as_str()} } }
                div { dt { "ベース" } dd { {worker.base_branch.as_str()} } }
                div { dt { "トークン" } dd { {worker.tokens_used.to_string()} } }
                div { dt { "待機" } dd {
                    {worker.idle_ms.map_or_else(|| String::from("PTY終了"), |ms| format!("{}秒", ms / 1000))}
                } }
            }
        }
    }
}

fn group_messages(messages: Vec<HiveMessage>) -> BTreeMap<String, Vec<HiveMessage>> {
    let mut grouped = BTreeMap::new();
    for message in messages {
        grouped
            .entry(message.conversation.clone())
            .or_insert_with(Vec::new)
            .push(message);
    }
    for thread in grouped.values_mut() {
        thread.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    }
    grouped
}

const fn status_key(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Todo => "todo",
        TaskStatus::Doing => "doing",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
    }
}

fn parse_status(value: &str) -> Option<TaskStatus> {
    match value {
        "todo" => Some(TaskStatus::Todo),
        "doing" => Some(TaskStatus::Doing),
        "blocked" => Some(TaskStatus::Blocked),
        "done" => Some(TaskStatus::Done),
        _ => None,
    }
}

const fn act_label(act: md_web_contracts::MessageAct) -> &'static str {
    match act {
        md_web_contracts::MessageAct::Request => "依頼",
        md_web_contracts::MessageAct::Inform => "共有",
        md_web_contracts::MessageAct::Propose => "提案",
        md_web_contracts::MessageAct::Query => "質問",
        md_web_contracts::MessageAct::Agree => "同意",
        md_web_contracts::MessageAct::Refuse => "辞退",
        md_web_contracts::MessageAct::Done => "完了",
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_status, status_key};
    use md_web_contracts::TaskStatus;

    #[test]
    fn task_status_round_trips() {
        assert_eq!(
            parse_status(status_key(TaskStatus::Blocked)),
            Some(TaskStatus::Blocked)
        );
    }

    #[test]
    fn unknown_status_is_rejected() {
        assert_eq!(parse_status("unknown"), None);
    }
}
