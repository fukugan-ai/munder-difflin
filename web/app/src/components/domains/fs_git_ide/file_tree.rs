use dioxus::prelude::*;
use md_web_contracts::domains::fs_git_ide::DirEntry;

#[component]
pub(super) fn FileTree(
    rel_path: String,
    entries: Vec<DirEntry>,
    active_file: Option<String>,
    on_open: EventHandler<DirEntry>,
    on_up: EventHandler<()>,
) -> Element {
    let display_path = if rel_path.is_empty() {
        String::from("/")
    } else {
        rel_path.clone()
    };
    rsx! {
        section { class: "md-ide-tree", aria_label: "ファイルツリー",
            div { class: "md-ide-tree__path",
                button {
                    class: "md-ide-button md-ide-button--quiet",
                    r#type: "button",
                    disabled: rel_path.is_empty(),
                    aria_label: "親フォルダーへ戻る",
                    onclick: move |_| on_up.call(()),
                    "↑"
                }
                code { "{display_path}" }
            }
            div { class: "md-ide-tree__rows", role: "tree",
                if entries.is_empty() {
                    p { class: "md-ide-note", "このフォルダーは空です" }
                }
                for entry in entries {
                    {
                        let is_active = active_file.as_deref() == Some(entry.name.as_str());
                        let button_class = if is_active {
                            "md-ide-tree__row md-ide-tree__row--active"
                        } else {
                            "md-ide-tree__row"
                        };
                        rsx! {
                            button {
                                key: "{entry.name}",
                                class: button_class,
                                r#type: "button",
                                role: "treeitem",
                                aria_selected: is_active.to_string(),
                                onclick: move |_| on_open.call(entry.clone()),
                                span { aria_hidden: "true", if entry.is_dir { "▸" } else { "·" } }
                                span { class: "md-ide-tree__name", "{entry.name}" }
                                if !entry.is_dir {
                                    span { class: "md-ide-tree__size", "{format_size(entry.size)}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::format_size;

    #[test]
    fn bytes_are_not_rounded_to_kilobytes() {
        assert_eq!(format_size(42), "42 B");
    }
}
