use dioxus::prelude::*;

const MONACO_BRIDGE: Asset = asset!("/src/components/domains/fs_git_ide/assets/monaco_bridge.js");
const MONACO_BOOTSTRAP: Asset = asset!("/assets/domains/fs_git_ide/monaco_bootstrap.js");
// AMD modules need stable relative URLs, so register the whole private runtime as one folder asset.
const MONACO_ASSETS: Asset = asset!("/assets/domains/fs_git_ide/monaco", AssetOptions::folder());
const MONACO_WORKER: Asset = asset!(
    "/assets/domains/fs_git_ide/monaco_worker.js",
    AssetOptions::js().with_hash_suffix(false)
);

#[component]
pub(super) fn MonacoIsland(
    value: String,
    language: String,
    read_only: bool,
    save_state: String,
    on_change: EventHandler<String>,
    on_save: EventHandler<()>,
) -> Element {
    rsx! {
        document::Script { src: MONACO_BOOTSTRAP }
        document::Script { r#type: "module", src: MONACO_BRIDGE }
        div {
            class: "md-monaco-island",
            "data-monaco-island": "true",
            "data-monaco-assets": MONACO_ASSETS,
            "data-monaco-worker": MONACO_WORKER,
            "data-language": language,
            "data-readonly": read_only.to_string(),
            "data-state": save_state,
            div { class: "md-monaco-island__toolbar",
                span { "エディター" }
                span { class: "md-monaco-island__state", role: "status", "{save_state}" }
                button {
                    id: "md-monaco-save-proxy",
                    class: "md-ide-button md-ide-button--primary",
                    r#type: "button",
                    disabled: read_only,
                    onclick: move |_| on_save.call(()),
                    "保存"
                }
            }
            div { class: "md-monaco-island__surface", "data-monaco-surface": "true" }
            p { class: "md-monaco-island__fallback", role: "status", "Monacoを読み込めないため簡易エディターで表示しています" }
            textarea {
                class: "md-monaco-island__source",
                "data-monaco-source": "true",
                aria_label: "ファイル内容",
                readonly: read_only,
                value,
                oninput: move |event| on_change.call(event.value()),
                onkeydown: move |event| {
                    if event.modifiers().contains(Modifiers::CONTROL)
                        && event.key() == Key::Character(String::from("s"))
                    {
                        on_save.call(());
                    }
                },
            }
        }
    }
}
