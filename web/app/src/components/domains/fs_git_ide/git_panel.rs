use dioxus::prelude::*;
use md_web_contracts::domains::fs_git_ide::{
    CheckoutRequest, CiRun, GitCommit, GitCompare, GitFileAtRevision, GitFileChange, GitHubIssue,
    GitOverview, GitWorktree, WorkspaceId,
};

use super::api;

#[component]
pub(super) fn GitPanel(
    workspace_id: Option<WorkspaceId>,
    overview: Option<GitOverview>,
    issues: Vec<GitHubIssue>,
    ci_runs: Vec<CiRun>,
    note: Option<String>,
    on_refresh: EventHandler<()>,
    on_load_github: EventHandler<()>,
) -> Element {
    let mut extra_history = use_signal(Vec::<GitCommit>::new);
    let mut selected_revision = use_signal(String::new);
    let mut commit_files = use_signal(Vec::<GitFileChange>::new);
    let mut shown_file = use_signal(|| None::<GitFileAtRevision>);
    let mut base_ref = use_signal(|| String::from("main"));
    let mut head_ref = use_signal(|| String::from("HEAD"));
    let mut three_dot = use_signal(|| true);
    let mut comparison = use_signal(|| None::<GitCompare>);
    let mut worktrees = use_signal(Vec::<GitWorktree>::new);
    let mut checkout_ref = use_signal(String::new);
    let mut checkout_detach = use_signal(|| false);
    let mut checkout_confirmed = use_signal(|| false);
    let mut action_note = use_signal(|| None::<String>);

    let history_skip = overview
        .as_ref()
        .map_or(0, |git| {
            u32::try_from(git.commits.len()).unwrap_or(u32::MAX)
        })
        .saturating_add(u32::try_from(extra_history.read().len()).unwrap_or(u32::MAX));

    let workspace_for_history = workspace_id.clone();
    let load_more = move |_| {
        let Some(id) = workspace_for_history.clone() else {
            return;
        };
        spawn(async move {
            match api::git_history(id, 200, history_skip).await {
                Ok(rows) => extra_history.write().extend(rows),
                Err(error) => action_note.set(Some(error.to_string())),
            }
        });
    };

    let workspace_for_compare = workspace_id.clone();
    let compare = move |_| {
        let Some(id) = workspace_for_compare.clone() else {
            return;
        };
        let base = base_ref.read().clone();
        let head = head_ref.read().clone();
        let mode = *three_dot.read();
        spawn(async move {
            match api::git_compare_refs(id, base, head, mode).await {
                Ok(value) => comparison.set(Some(value)),
                Err(error) => action_note.set(Some(error.to_string())),
            }
        });
    };

    let workspace_for_worktrees = workspace_id.clone();
    let load_worktrees = move |_| {
        let Some(id) = workspace_for_worktrees.clone() else {
            return;
        };
        spawn(async move {
            match api::git_worktrees(id).await {
                Ok(rows) => worktrees.set(rows),
                Err(error) => action_note.set(Some(error.to_string())),
            }
        });
    };

    let workspace_for_checkout = workspace_id.clone();
    let checkout = move |_| {
        let Some(id) = workspace_for_checkout.clone() else {
            return;
        };
        let reference = checkout_ref.read().trim().to_owned();
        if !checkout_ready(&reference, *checkout_confirmed.read()) {
            action_note.set(Some(String::from(
                "参照を入力し、ローカル切替を確認してください",
            )));
            return;
        }
        let detach = *checkout_detach.read();
        spawn(async move {
            match api::git_checkout(CheckoutRequest {
                workspace_id: id,
                reference,
                detach,
                confirmed: true,
            })
            .await
            {
                Ok(_) => {
                    checkout_confirmed.set(false);
                    action_note.set(Some(String::from("ローカルcheckoutが完了しました")));
                    on_refresh.call(());
                }
                Err(error) => action_note.set(Some(error.to_string())),
            }
        });
    };

    rsx! {
        aside { class: "md-git-panel", aria_label: "GitとGitHub",
            div { class: "md-git-panel__heading",
                h3 { "Git" }
                button { class: "md-ide-button md-ide-button--quiet", r#type: "button", onclick: move |_| on_refresh.call(()), "更新" }
            }
            if let Some(message) = note.or_else(|| action_note.read().clone()) {
                p { class: "md-ide-note", role: "status", "{message}" }
            }
            match &overview {
                Some(git) if git.is_repo => rsx! {
                    div { class: "md-git-panel__branch",
                        strong { {git.branch.as_ref().and_then(|branch| branch.current.as_deref()).unwrap_or("DETACHED HEAD")} }
                        if let Some(ahead) = &git.ahead_behind { span { "↑{ahead.ahead} ↓{ahead.behind}" } }
                    }
                    div { class: "md-git-panel__changes",
                        h4 { "変更" }
                        if let Some(status) = &git.status {
                            for entry in &status.staged { p { "S {entry.path}" } }
                            for entry in &status.unstaged { p { "M {entry.path}" } }
                            for path in &status.untracked { p { "? {path}" } }
                            if status.staged.is_empty() && status.unstaged.is_empty() && status.untracked.is_empty() {
                                p { class: "md-ide-note", "作業ツリーはクリーンです" }
                            }
                        }
                    }
                    div { class: "md-git-panel__history",
                        h4 { "履歴グラフ" }
                        for commit in git.commits.iter().chain(extra_history.read().iter()) {
                            button {
                                key: "{commit.sha}", class: "md-git-history-row", r#type: "button",
                                onclick: {
                                    let commit = commit.clone();
                                    let workspace_id = workspace_id.clone();
                                    move |_| {
                                        let Some(id) = workspace_id.clone() else { return; };
                                        selected_revision.set(commit.sha.clone());
                                        shown_file.set(None);
                                        let revision = commit.sha.clone();
                                        spawn(async move {
                                            match api::git_commit_files(id, revision).await {
                                                Ok(rows) => commit_files.set(rows),
                                                Err(error) => action_note.set(Some(error.to_string())),
                                            }
                                        });
                                    }
                                },
                                code { "{commit.short_sha}" }
                                span { "{commit.subject}" }
                            }
                        }
                        button { class: "md-ide-button md-ide-button--quiet", r#type: "button", onclick: load_more, "さらに200件" }
                    }
                    if !commit_files.read().is_empty() {
                        div { class: "md-git-panel__tool",
                            h4 { "commit files" }
                            for file in commit_files.read().iter() {
                                button {
                                    class: "md-git-file-row", r#type: "button",
                                    onclick: {
                                        let path = file.path.clone();
                                        let workspace_id = workspace_id.clone();
                                        move |_| {
                                            let Some(id) = workspace_id.clone() else { return; };
                                            let revision = selected_revision.read().clone();
                                            let path = path.clone();
                                            spawn(async move {
                                                match api::git_show_file(id, revision, path).await {
                                                    Ok(file) => shown_file.set(Some(file)),
                                                    Err(error) => action_note.set(Some(error.to_string())),
                                                }
                                            });
                                        }
                                    },
                                    "{file.status} {file.path}"
                                }
                            }
                            if let Some(file) = shown_file.read().as_ref() {
                                if file.is_binary { p { class: "md-ide-note", "バイナリ" } }
                                else if file.exists { pre { class: "md-git-file-preview", "{file.content}" } }
                                else { p { class: "md-ide-note", "このrevisionには存在しません" } }
                            }
                        }
                    }
                    div { class: "md-git-panel__tool",
                        h4 { "参照を比較" }
                        input { aria_label: "比較元", value: "{base_ref}", oninput: move |event| base_ref.set(event.value()) }
                        input { aria_label: "比較先", value: "{head_ref}", oninput: move |event| head_ref.set(event.value()) }
                        label { input { r#type: "checkbox", checked: *three_dot.read(), onchange: move |event| three_dot.set(event.checked()) } "merge-baseから比較" }
                        button { class: "md-ide-button", r#type: "button", onclick: compare, "比較" }
                        if let Some(result) = comparison.read().as_ref() {
                            p { class: "md-ide-note", "ahead {result.ahead} / behind {result.behind} / {result.files.len()} files" }
                        }
                    }
                    div { class: "md-git-panel__tool",
                        div { class: "md-git-panel__heading",
                            h4 { "worktrees" }
                            button { class: "md-ide-button md-ide-button--quiet", r#type: "button", onclick: load_worktrees, "取得" }
                        }
                        for worktree in worktrees.read().iter() {
                            p { title: "{worktree.path}", {worktree.branch.as_deref().unwrap_or("detached")} }
                        }
                    }
                    div { class: "md-git-panel__tool",
                        h4 { "安全なローカルcheckout" }
                        input { aria_label: "checkoutする参照", value: "{checkout_ref}", oninput: move |event| checkout_ref.set(event.value()) }
                        label { input { r#type: "checkbox", checked: *checkout_detach.read(), onchange: move |event| checkout_detach.set(event.checked()) } "detached" }
                        label { input { r#type: "checkbox", checked: *checkout_confirmed.read(), onchange: move |event| checkout_confirmed.set(event.checked()) } "未保存変更がなく、Agent停止済みと確認" }
                        button { class: "md-ide-button", r#type: "button", disabled: !checkout_ready(&checkout_ref.read(), *checkout_confirmed.read()), onclick: checkout, "checkout" }
                    }
                },
                Some(_) => rsx! { p { class: "md-ide-note", "Gitリポジトリではありません" } },
                None => rsx! { p { class: "md-ide-note", "Git情報を読み込み中…" } },
            }
            div { class: "md-git-panel__heading md-git-panel__heading--github",
                h3 { "GitHub" }
                button { class: "md-ide-button md-ide-button--quiet", r#type: "button", onclick: move |_| on_load_github.call(()), "Issue / CIを取得" }
            }
            div { class: "md-git-panel__github",
                for issue in issues { a { key: "issue-{issue.number}", href: issue.url, target: "_blank", rel: "noreferrer", "#{issue.number} {issue.title}" } }
                for run in ci_runs { a { key: "run-{run.url}", href: run.url, target: "_blank", rel: "noreferrer", "{run.name} · {run.status}" } }
            }
        }
    }
}

fn checkout_ready(reference: &str, confirmed: bool) -> bool {
    confirmed && !reference.trim().is_empty() && !reference.trim_start().starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::checkout_ready;

    #[test]
    fn checkout_requires_confirmation_and_non_option_reference() {
        assert!(checkout_ready("main", true));
        assert!(!checkout_ready("main", false));
        assert!(!checkout_ready("--help", true));
    }
}
