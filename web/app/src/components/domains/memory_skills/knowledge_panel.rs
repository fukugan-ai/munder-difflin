use dioxus::prelude::*;
use md_web_contracts::domains::memory_skills::{
    KnowledgeDetail, KnowledgeDocument, KnowledgeHit, KnowledgeStatus, KnowledgeUploadRequest,
};

const MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

#[component]
pub(super) fn KnowledgePanel(
    status: KnowledgeStatus,
    documents: Vec<KnowledgeDocument>,
    hits: Vec<KnowledgeHit>,
    detail: Option<KnowledgeDetail>,
    busy: bool,
    on_search: EventHandler<String>,
    on_upload: EventHandler<KnowledgeUploadRequest>,
    on_remove: EventHandler<String>,
    on_get: EventHandler<String>,
) -> Element {
    let mut query = use_signal(String::new);
    rsx! {
        div { class: "domain-stack", "data-testid": "knowledge-panel",
            div { class: "domain-summary",
                div {
                    h2 { "ナレッジ" }
                    p { "ローカル文書を取り込み、エージェントと同じ索引を検索します。" }
                }
                span { class: "domain-state domain-state--success",
                    "{status.document_count}件 · {status.chunk_count}チャンク"
                }
            }
            div { class: "domain-actions",
                label { class: "domain-button domain-button--primary",
                    "data-ui-state": if busy { "loading" } else { "default" },
                    if busy { "取込中…" } else { "ファイルを取り込む" }
                    input {
                        class: "domain-visually-hidden",
                        r#type: "file",
                        disabled: busy || !status.enabled,
                        onchange: move |event| {
                            let Some(file) = event.files().into_iter().next() else { return; };
                            if file.size() > MAX_UPLOAD_BYTES { return; }
                            let upload = on_upload;
                            spawn(async move {
                                if let Ok(bytes) = file.read_bytes().await {
                                    upload.call(KnowledgeUploadRequest {
                                        source_name: file.name(),
                                        title: None,
                                        tags: Vec::new(),
                                        caption: None,
                                        bytes: bytes.to_vec(),
                                    });
                                }
                            });
                        },
                    }
                }
            }
            form {
                class: "domain-search",
                onsubmit: move |event| {
                    event.prevent_default();
                    let value = query().trim().to_owned();
                    if !value.is_empty() { on_search.call(value); }
                },
                label { r#for: "knowledge-query", "ナレッジを検索" }
                div { class: "domain-search__row",
                    input {
                        id: "knowledge-query",
                        value: "{query}",
                        disabled: busy,
                        maxlength: 512,
                        placeholder: "文書の内容を検索…",
                        oninput: move |event| query.set(event.value()),
                    }
                    button {
                        class: "domain-button",
                        r#type: "submit",
                        disabled: busy || query().trim().is_empty(),
                        "検索"
                    }
                }
            }
            if !hits.is_empty() {
                section { class: "domain-section", aria_labelledby: "knowledge-results-title",
                    h3 { id: "knowledge-results-title", "検索結果" }
                    ul { class: "domain-list",
                        for hit in hits {
                            li { class: "domain-list__item",
                                strong { {hit.title} }
                                span { class: "domain-meta", "スコア {format_score(hit.score)}" }
                                p { {hit.snippet} }
                            }
                        }
                    }
                }
            }
            section { class: "domain-section", aria_labelledby: "knowledge-documents-title",
                h3 { id: "knowledge-documents-title", "取り込み済み" }
                if documents.is_empty() {
                    p { class: "domain-empty", "取り込まれた文書はありません。" }
                } else {
                    ul { class: "domain-list",
                        for document in documents {
                            KnowledgeDocumentRow { document, busy, on_remove, on_get }
                        }
                    }
                }
            }
            if let Some(detail) = detail {
                section { class: "domain-section knowledge-detail", aria_labelledby: "knowledge-detail-title",
                    h3 { id: "knowledge-detail-title", {detail.document.title} }
                    p { class: "domain-meta", "{detail.document.source} · {detail.document.bytes} bytes" }
                    pre { {detail.text} }
                }
            }
        }
    }
}

#[component]
fn KnowledgeDocumentRow(
    document: KnowledgeDocument,
    busy: bool,
    on_remove: EventHandler<String>,
    on_get: EventHandler<String>,
) -> Element {
    let document_id = document.id.clone();
    let detail_id = document.id.clone();
    rsx! {
        li { class: "domain-list__item domain-list__item--action",
            div {
                strong { {document.title} }
                p { class: "domain-meta", "{document.modality} · {document.chunk_count}チャンク · {document.source}" }
            }
            div { class: "domain-actions",
                button {
                    class: "domain-button",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_get.call(detail_id.clone()),
                    "詳細"
                }
                button {
                    class: "domain-button domain-button--danger",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_remove.call(document_id.clone()),
                    "削除"
                }
            }
        }
    }
}

fn format_score(score: f64) -> String {
    format!("{:.0}%", score.clamp(0.0, 1.0) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::format_score;

    #[test]
    fn score_is_clamped_for_display() {
        assert_eq!(format_score(2.0), "100%");
    }
}
