use dioxus::prelude::*;
use md_web_contracts::domains::fs_git_ide::{
    BinaryFile, CiRun, DirEntry, GitDiff, GitHubIssue, GitOverview, WorkspaceCapability,
    WorkspaceId, WorkspaceSummary, WriteFileRequest,
};

use super::api;
use super::file_tree::FileTree;
use super::git_panel::GitPanel;
use super::image_preview::ImagePreview;
use super::monaco::MonacoIsland;

const IDE_CSS: Asset = asset!("/assets/domains/fs_git_ide.css");

#[component]
pub(crate) fn FsGitIde(#[props(default)] initial_workspace_path: Option<String>) -> Element {
    let workspaces = use_resource(api::workspaces);
    let mut selected = use_signal(|| None::<WorkspaceId>);
    let mut current_dir = use_signal(String::new);
    let mut active_file = use_signal(|| None::<String>);
    let mut content = use_signal(String::new);
    let mut original = use_signal(String::new);
    let mut save_state = use_signal(|| String::from("待機中"));
    let mut notice = use_signal(|| None::<String>);
    let mut diff = use_signal(|| None::<GitDiff>);
    let mut image = use_signal(|| None::<BinaryFile>);
    let mut issues = use_signal(Vec::<GitHubIssue>::new);
    let mut ci_runs = use_signal(Vec::<CiRun>::new);

    use_effect(move || {
        if selected.peek().is_some() {
            return;
        }
        if let Some(Ok(items)) = workspaces.read().as_ref()
            && let Some(workspace) = initial_workspace_path
                .as_deref()
                .and_then(|path| workspace_for_path(items, path))
                .or_else(|| items.first().map(|workspace| workspace.id.clone()))
        {
            selected.set(Some(workspace));
        }
    });

    let mut entries = use_resource(move || {
        let workspace_id = selected.read().clone();
        let rel_path = current_dir.read().clone();
        async move {
            match workspace_id {
                Some(id) => api::list_dir(id, rel_path).await,
                None => Ok(Vec::new()),
            }
        }
    });

    let mut git = use_resource(move || {
        let workspace_id = selected.read().clone();
        async move {
            match workspace_id {
                Some(id) => api::git_overview(id).await.map(Some),
                None => Ok(None),
            }
        }
    });

    let workspace_rows = workspaces
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let directory_rows = entries
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .cloned()
        .unwrap_or_default();
    let git_snapshot: Option<GitOverview> = git
        .read()
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .and_then(Clone::clone);
    let selected_value = selected
        .read()
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let mutable_workspace = selected
        .read()
        .as_ref()
        .and_then(|id| workspace_rows.iter().find(|workspace| workspace.id == *id))
        .is_some_and(|workspace| workspace.capability == WorkspaceCapability::PrivateMutable);
    let active_name = active_file
        .read()
        .as_ref()
        .and_then(|path| path.rsplit('/').next())
        .map(String::from);

    let open_entry = move |entry: DirEntry| {
        let rel_path = join_rel(&current_dir.read(), &entry.name);
        if entry.is_dir {
            current_dir.set(rel_path);
            active_file.set(None);
            diff.set(None);
            image.set(None);
            return;
        }
        let Some(workspace_id) = selected.read().clone() else {
            return;
        };
        active_file.set(Some(rel_path.clone()));
        content.set(String::new());
        original.set(String::new());
        diff.set(None);
        image.set(None);
        save_state.set(String::from("読み込み中"));
        spawn(async move {
            if is_image_path(&rel_path) {
                match api::read_binary(workspace_id, rel_path).await {
                    Ok(file) => {
                        image.set(Some(file));
                        save_state.set(String::from("プレビュー"));
                        notice.set(None);
                    }
                    Err(error) => {
                        save_state.set(String::from("エラー"));
                        notice.set(Some(error.to_string()));
                    }
                }
                return;
            }
            match api::read_text(workspace_id, rel_path).await {
                Ok(file) => {
                    original.set(file.content.clone());
                    content.set(file.content);
                    save_state.set(String::from("保存済み"));
                    notice.set(None);
                }
                Err(error) => {
                    save_state.set(String::from("エラー"));
                    notice.set(Some(error.to_string()));
                }
            }
        });
    };

    let save_file = move |_| {
        if !mutable_workspace {
            notice.set(Some(String::from(
                "登録元は参照専用です。Agent用private workspaceを選択してください。",
            )));
            return;
        }
        let Some(workspace_id) = selected.read().clone() else {
            return;
        };
        let Some(rel_path) = active_file.read().clone() else {
            return;
        };
        let snapshot = content.read().clone();
        if snapshot == *original.read() {
            save_state.set(String::from("保存済み"));
            return;
        }
        save_state.set(String::from("保存中"));
        spawn(async move {
            let request = WriteFileRequest {
                workspace_id,
                rel_path,
                content: snapshot.clone(),
            };
            match api::write_text(request).await {
                Ok(_) => {
                    original.set(snapshot);
                    save_state.set(String::from("保存済み"));
                    notice.set(None);
                    git.restart();
                    entries.restart();
                }
                Err(error) => {
                    save_state.set(String::from("保存失敗"));
                    notice.set(Some(error.to_string()));
                }
            }
        });
    };

    let load_diff = move |_| {
        let Some(workspace_id) = selected.read().clone() else {
            return;
        };
        let Some(rel_path) = active_file.read().clone() else {
            return;
        };
        notice.set(Some(String::from("差分を読み込み中…")));
        spawn(async move {
            match api::git_diff(workspace_id, rel_path).await {
                Ok(value) => {
                    diff.set(Some(value));
                    notice.set(None);
                }
                Err(error) => notice.set(Some(error.to_string())),
            }
        });
    };

    let load_github = move |_| {
        let Some(workspace_id) = selected.read().clone() else {
            return;
        };
        notice.set(Some(String::from("GitHubからIssueとCIを取得中…")));
        spawn(async move {
            let issue_result = api::github_issues(workspace_id.clone()).await;
            let run_result = api::github_ci_runs(workspace_id).await;
            match (issue_result, run_result) {
                (Ok(new_issues), Ok(new_runs)) => {
                    issues.set(new_issues);
                    ci_runs.set(new_runs);
                    notice.set(None);
                }
                (Err(error), _) | (_, Err(error)) => notice.set(Some(error.to_string())),
            }
        });
    };

    rsx! {
        document::Link { rel: "stylesheet", href: IDE_CSS }
        section { class: "md-ide", "data-fs-git-ide": "true", aria_labelledby: "md-ide-title",
            header { class: "md-ide__header",
                div {
                    h2 { id: "md-ide-title", "IDE" }
                    p {
                        if mutable_workspace {
                            "Agent用private workspaceを編集しています"
                        } else {
                            "登録元は参照専用です。編集にはAgent用private workspaceを選択してください"
                        }
                    }
                }
                label { class: "md-ide__workspace",
                    span { "ワークスペース" }
                    select {
                        value: selected_value,
                        onchange: move |event| {
                            selected.set(Some(WorkspaceId(event.value())));
                            current_dir.set(String::new());
                            active_file.set(None);
                            content.set(String::new());
                            original.set(String::new());
                            diff.set(None);
                            image.set(None);
                        },
                        if workspace_rows.is_empty() {
                            option { value: "", "登録済みworkspaceなし" }
                        }
                        for workspace in workspace_rows {
                            option {
                                key: "{workspace.id}", value: workspace.id.to_string(),
                                if workspace.capability == WorkspaceCapability::PrivateMutable {
                                    "{workspace.name}（private）"
                                } else {
                                    "{workspace.name}（参照専用）"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(message) = notice.read().as_ref() {
                p { class: "md-ide__notice", role: "status", "{message}" }
            }
            div { class: "md-ide__layout",
                FileTree {
                    rel_path: current_dir.read().clone(),
                    entries: directory_rows,
                    active_file: active_name,
                    on_open: open_entry,
                    on_up: move |_| {
                        let parent = parent_rel(&current_dir.read());
                        current_dir.set(parent);
                        active_file.set(None);
                        diff.set(None);
                        image.set(None);
                    },
                }
                main { class: "md-ide__editor",
                    if let Some(path) = active_file.read().as_ref() {
                        div { class: "md-ide__filebar",
                            code { "{path}" }
                            button {
                                class: "md-ide-button md-ide-button--quiet",
                                r#type: "button",
                                onclick: load_diff,
                                "HEADとの差分"
                            }
                        }
                        if let Some(current_image) = image.read().as_ref() {
                            ImagePreview { file: current_image.clone() }
                        } else if let Some(current_diff) = diff.read().as_ref() {
                            DiffView { diff: current_diff.clone(), on_close: move |_| diff.set(None) }
                        } else {
                            MonacoIsland {
                                key: "{path}",
                                value: content.read().clone(),
                                language: language_for_path(path),
                                read_only: !mutable_workspace,
                                save_state: if mutable_workspace {
                                    save_state.read().clone()
                                } else {
                                    String::from("参照専用")
                                },
                                on_change: move |value| {
                                    content.set(value);
                                    save_state.set(if *content.read() == *original.read() {
                                        String::from("保存済み")
                                    } else {
                                        String::from("未保存")
                                    });
                                },
                                on_save: save_file,
                            }
                        }
                    } else {
                        div { class: "md-ide__empty",
                            h3 { "ファイルを選択" }
                            p { "左のツリーからテキストファイルを開けます。" }
                        }
                    }
                }
                GitPanel {
                    workspace_id: selected.read().clone(),
                    mutable_workspace,
                    overview: git_snapshot,
                    issues: issues.read().clone(),
                    ci_runs: ci_runs.read().clone(),
                    note: notice.read().clone(),
                    on_refresh: move |_| git.restart(),
                    on_load_github: load_github,
                }
            }
        }
    }
}

#[component]
fn DiffView(diff: GitDiff, on_close: EventHandler<()>) -> Element {
    rsx! {
        section { class: "md-ide-diff", aria_label: "Git差分",
            div { class: "md-ide-diff__header",
                span { "HEAD → 作業ツリー" }
                button {
                    class: "md-ide-button md-ide-button--quiet",
                    r#type: "button",
                    onclick: move |_| on_close.call(()),
                    "閉じる"
                }
            }
            if diff.is_binary {
                p { class: "md-ide-note", "バイナリファイルはテキスト差分を表示できません" }
            } else {
                div { class: "md-ide-diff__columns",
                    pre { aria_label: "HEADの内容", "{diff.head}" }
                    pre { aria_label: "作業ツリーの内容", "{diff.working}" }
                }
            }
        }
    }
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        String::from(name)
    } else {
        format!("{parent}/{name}")
    }
}

fn parent_rel(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(String::new, |(parent, _)| String::from(parent))
}

fn language_for_path(path: &str) -> String {
    let extension = path
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("");
    String::from(match extension.to_ascii_lowercase().as_str() {
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "rs" => "rust",
        "md" | "markdown" => "markdown",
        "css" | "scss" | "less" => "css",
        "html" | "htm" => "html",
        "yml" | "yaml" => "yaml",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "shell",
        "py" => "python",
        _ => "plaintext",
    })
}

fn is_image_path(path: &str) -> bool {
    path.rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .is_some_and(|extension| {
            matches!(
                extension.as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico" | "avif"
            )
        })
}

fn workspace_for_path(workspaces: &[WorkspaceSummary], path: &str) -> Option<WorkspaceId> {
    workspaces
        .iter()
        .filter(|workspace| path_is_within(path, &workspace.display_path))
        .max_by_key(|workspace| workspace.display_path.len())
        .map(|workspace| workspace.id.clone())
}

fn path_is_within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| root.ends_with(['/', '\\']) || suffix.starts_with(['/', '\\']))
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::fs_git_ide::{
        WorkspaceCapability, WorkspaceId, WorkspaceSummary,
    };

    use super::{is_image_path, join_rel, language_for_path, parent_rel, workspace_for_path};

    #[test]
    fn root_child_has_no_leading_slash() {
        assert_eq!(join_rel("", "src"), "src");
    }

    #[test]
    fn parent_of_root_child_is_root() {
        assert_eq!(parent_rel("src"), "");
    }

    #[test]
    fn rust_extension_selects_rust_language() {
        assert_eq!(language_for_path("src/main.rs"), "rust");
    }

    #[test]
    fn image_detection_is_case_insensitive() {
        assert!(is_image_path("screenshots/Office.PNG"));
        assert!(!is_image_path("src/main.rs"));
    }

    #[test]
    fn initial_path_selects_the_longest_registered_root() {
        let workspaces = vec![
            WorkspaceSummary {
                id: WorkspaceId(String::from("main")),
                name: String::from("main"),
                display_path: String::from("/srv/repo"),
                capability: WorkspaceCapability::SourceReadOnly,
            },
            WorkspaceSummary {
                id: WorkspaceId(String::from("worktree")),
                name: String::from("worktree"),
                display_path: String::from("/srv/repo-worktrees/task"),
                capability: WorkspaceCapability::PrivateMutable,
            },
        ];

        assert_eq!(
            workspace_for_path(&workspaces, "/srv/repo-worktrees/task/src").as_ref(),
            Some(&WorkspaceId(String::from("worktree")))
        );
        assert!(workspace_for_path(&workspaces, "/srv/repository").is_none());
    }
}
