use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::memory_skills::{
    KnowledgeDetail, KnowledgeDocument, KnowledgeFileResult, KnowledgeHit, KnowledgeIngestResponse,
    KnowledgeStatus, MemoryGraphEdge, MemoryGraphNode, MemoryGraphSnapshot,
};
use serde::{Deserialize, Serialize};

use super::DomainError;

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_INDEX_CHARS: usize = 5 * 1024 * 1024;
const CHUNK_CHARS: usize = 1_200;
const CHUNK_OVERLAP: usize = 150;
const MAX_SEARCH_RESULTS: usize = 100;
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize, Serialize)]
struct IndexRecord {
    document_id: String,
    title: String,
    source: String,
    modality: String,
    chunk_index: u64,
    text: String,
}

struct DocumentInput<'a> {
    staged_path: &'a Path,
    source_name: &'a str,
    title: Option<&'a str>,
    tags: &'a [String],
    caption: Option<&'a str>,
    document_id: &'a str,
}

pub struct KnowledgeService {
    root: PathBuf,
    enabled: bool,
}

impl KnowledgeService {
    pub fn new(root: PathBuf, enabled: bool) -> Self {
        Self { root, enabled }
    }

    pub fn status(&self) -> Result<KnowledgeStatus, DomainError> {
        let documents = self.list()?;
        let mut chunk_count = 0_u64;
        let mut by_modality = BTreeMap::new();
        for document in &documents {
            chunk_count = chunk_count.saturating_add(document.chunk_count);
            let count = by_modality
                .entry(document.modality.clone())
                .or_insert(0_u64);
            *count = count.saturating_add(1);
        }
        Ok(KnowledgeStatus {
            enabled: self.enabled,
            document_count: u64::try_from(documents.len()).unwrap_or(u64::MAX),
            chunk_count,
            by_modality,
        })
    }

    pub fn graph(&self) -> Result<MemoryGraphSnapshot, DomainError> {
        let documents = self.list()?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut tags = BTreeMap::new();
        for document in documents {
            nodes.push(MemoryGraphNode {
                id: document.id.clone(),
                label: document.title,
                modality: document.modality,
                weight: document.chunk_count.max(1),
            });
            for tag in document.tags {
                let tag_id = format!("tag:{}", tag.to_lowercase());
                tags.entry(tag_id.clone()).or_insert_with(|| tag.clone());
                edges.push(MemoryGraphEdge {
                    source: document.id.clone(),
                    target: tag_id,
                    relation: String::from("tagged"),
                });
            }
        }
        nodes.extend(tags.into_iter().map(|(id, label)| MemoryGraphNode {
            id,
            label,
            modality: String::from("tag"),
            weight: 1,
        }));
        Ok(MemoryGraphSnapshot { nodes, edges })
    }

    pub fn list(&self) -> Result<Vec<KnowledgeDocument>, DomainError> {
        let docs = self.root.join("docs");
        if !docs.is_dir() {
            return Ok(Vec::new());
        }
        let mut documents = Vec::new();
        for entry in fs::read_dir(docs)? {
            let entry = match entry {
                Ok(value) => value,
                Err(_) => continue,
            };
            let meta = entry.path().join("meta.json");
            if let Ok(text) = fs::read_to_string(meta)
                && let Ok(document) = serde_json::from_str::<KnowledgeDocument>(&text)
            {
                documents.push(document);
            }
        }
        documents.sort_by(|left, right| right.added_at.cmp(&left.added_at));
        Ok(documents)
    }

    pub fn get(
        &self,
        document_id: &str,
    ) -> Result<Option<(KnowledgeDocument, String)>, DomainError> {
        validate_id(document_id)?;
        let dir = self.root.join("docs").join(document_id);
        if !dir.is_dir() {
            return Ok(None);
        }
        let meta = serde_json::from_str(&fs::read_to_string(dir.join("meta.json"))?)?;
        let text = fs::read_to_string(dir.join("text.md")).unwrap_or_default();
        Ok(Some((meta, text)))
    }

    pub fn get_detail(&self, document_id: &str) -> Result<Option<KnowledgeDetail>, DomainError> {
        Ok(self
            .get(document_id)?
            .map(|(document, text)| KnowledgeDetail { document, text }))
    }

    pub fn ingest_uploaded_file(
        &self,
        staged_path: &Path,
        source_name: &str,
        title: Option<&str>,
        tags: &[String],
        caption: Option<&str>,
    ) -> Result<KnowledgeIngestResponse, DomainError> {
        let _writer = knowledge_writer()
            .lock()
            .map_err(|_| DomainError::Unavailable("knowledge writer lock is poisoned"))?;
        if !self.enabled {
            return Err(DomainError::Unavailable("knowledge graph is disabled"));
        }
        let source_name = safe_file_name(source_name)?;
        let metadata = fs::metadata(staged_path)?;
        if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
            return Err(DomainError::InvalidInput("uploaded file exceeds the limit"));
        }
        let document_id = next_id();
        let docs = self.root.join("docs");
        fs::create_dir_all(&docs)?;
        let temp = docs.join(format!(".{document_id}.ingesting"));
        let destination = docs.join(&document_id);
        fs::create_dir(&temp)?;

        let result = self.write_document(
            &temp,
            DocumentInput {
                staged_path,
                source_name: &source_name,
                title,
                tags,
                caption,
                document_id: &document_id,
            },
        );
        let document = match result {
            Ok(value) => value,
            Err(error) => {
                let _ = fs::remove_dir_all(&temp);
                return Err(error);
            }
        };
        let index = self.root.join("index.jsonl");
        let prepared_index = prepare_index_append(&index, &document.1, &document_id)?;
        fs::rename(&temp, &destination)?;
        if let Err(error) = fs::rename(&prepared_index, &index) {
            let _ = fs::remove_dir_all(&destination);
            let _ = fs::remove_file(&prepared_index);
            return Err(DomainError::Io(error));
        }
        Ok(KnowledgeIngestResponse {
            ok: true,
            results: vec![KnowledgeFileResult {
                ok: true,
                source_name,
                document_id: Some(document_id),
                chunk_count: Some(document.0.chunk_count),
                error: None,
            }],
            error: None,
        })
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<KnowledgeHit>, DomainError> {
        let query = query.trim().to_lowercase();
        if query.is_empty() || query.chars().count() > 512 {
            return Err(DomainError::InvalidInput("invalid knowledge query"));
        }
        let terms: Vec<&str> = query
            .split_whitespace()
            .filter(|term| term.len() > 1)
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let index = self.root.join("index.jsonl");
        if !index.is_file() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for line in fs::read_to_string(index)?.lines() {
            let Ok(record) = serde_json::from_str::<IndexRecord>(line) else {
                continue;
            };
            let lower = record.text.to_lowercase();
            let matched = terms.iter().filter(|term| lower.contains(**term)).count();
            if matched == 0 {
                continue;
            }
            let score = matched as f64 / terms.len() as f64;
            hits.push(KnowledgeHit {
                document_id: record.document_id,
                title: record.title,
                source: record.source,
                modality: record.modality,
                chunk_index: record.chunk_index,
                score,
                snippet: record.text.chars().take(320).collect(),
            });
        }
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.document_id.cmp(&right.document_id))
                .then_with(|| left.chunk_index.cmp(&right.chunk_index))
        });
        hits.truncate(limit.clamp(1, MAX_SEARCH_RESULTS));
        Ok(hits)
    }

    pub fn remove(&self, document_id: &str) -> Result<bool, DomainError> {
        let _writer = knowledge_writer()
            .lock()
            .map_err(|_| DomainError::Unavailable("knowledge writer lock is poisoned"))?;
        validate_id(document_id)?;
        let dir = self.root.join("docs").join(document_id);
        if !dir.is_dir() {
            return Ok(false);
        }
        let index = self.root.join("index.jsonl");
        let temp = index.with_extension(format!("jsonl.{document_id}.rewriting"));
        if index.is_file() {
            let mut kept = String::new();
            for line in fs::read_to_string(&index)?.lines() {
                let Ok(record) = serde_json::from_str::<IndexRecord>(line) else {
                    continue;
                };
                if record.document_id != document_id {
                    kept.push_str(line);
                    kept.push('\n');
                }
            }
            write_synced(&temp, kept.as_bytes())?;
        }
        let quarantine = dir.with_extension("removing");
        fs::rename(&dir, &quarantine)?;
        if temp.is_file()
            && let Err(error) = fs::rename(&temp, &index)
        {
            let _ = fs::rename(&quarantine, &dir);
            let _ = fs::remove_file(&temp);
            return Err(DomainError::Io(error));
        }
        fs::remove_dir_all(quarantine)?;
        Ok(true)
    }

    fn write_document(
        &self,
        dir: &Path,
        input: DocumentInput<'_>,
    ) -> Result<(KnowledgeDocument, Vec<IndexRecord>), DomainError> {
        let extension = Path::new(input.source_name)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("bin")
            .to_lowercase();
        let modality = modality(&extension);
        let bytes = fs::metadata(input.staged_path)?.len();
        fs::copy(input.staged_path, dir.join(format!("original.{extension}")))?;
        let extracted = extract_text(input.staged_path, modality, input.caption)?;
        fs::write(dir.join("text.md"), &extracted)?;
        let truncated = extracted.chars().count() > MAX_INDEX_CHARS;
        let index_text: String = extracted.chars().take(MAX_INDEX_CHARS).collect();
        let title = input
            .title
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(input.source_name);
        let chunks = chunk_text(&index_text);
        let added_at = epoch_millis()?.to_string();
        let document = KnowledgeDocument {
            id: input.document_id.to_owned(),
            title: title.to_owned(),
            source: input.source_name.to_owned(),
            modality: modality.to_owned(),
            mime: mime(&extension).map(str::to_owned),
            original_extension: extension,
            bytes,
            tags: input
                .tags
                .iter()
                .take(32)
                .map(|tag| tag.chars().take(64).collect())
                .collect(),
            caption: input
                .caption
                .map(|value| value.chars().take(2_000).collect()),
            chunk_count: u64::try_from(chunks.len()).unwrap_or(u64::MAX),
            added_at,
            extractor: String::from(if modality == "text" || modality == "code" {
                "rust-text"
            } else {
                "metadata"
            }),
            truncated,
        };
        fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(&document)?)?;
        let records = chunks
            .into_iter()
            .enumerate()
            .map(|(index, text)| IndexRecord {
                document_id: document.id.clone(),
                title: document.title.clone(),
                source: document.source.clone(),
                modality: document.modality.clone(),
                chunk_index: u64::try_from(index).unwrap_or(u64::MAX),
                text,
            })
            .collect();
        Ok((document, records))
    }
}

fn prepare_index_append(
    path: &Path,
    records: &[IndexRecord],
    transaction_id: &str,
) -> Result<PathBuf, DomainError> {
    let temp = path.with_extension(format!("jsonl.{transaction_id}.writing"));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    if path.is_file() {
        let existing = fs::read(path)?;
        file.write_all(&existing)?;
        if !existing.is_empty() && existing.last() != Some(&b'\n') {
            file.write_all(b"\n")?;
        }
    }
    for record in records {
        serde_json::to_writer(&mut file, record)?;
        file.write_all(b"\n")?;
    }
    file.sync_all()?;
    Ok(temp)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), DomainError> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn knowledge_writer() -> &'static Mutex<()> {
    static WRITER: OnceLock<Mutex<()>> = OnceLock::new();
    WRITER.get_or_init(|| Mutex::new(()))
}

fn safe_file_name(value: &str) -> Result<String, DomainError> {
    let name = Path::new(value)
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or(DomainError::InvalidInput("invalid upload name"))?;
    if name.is_empty() || name.len() > 255 || name != value {
        return Err(DomainError::InvalidInput("invalid upload name"));
    }
    Ok(name.to_owned())
}

fn validate_id(value: &str) -> Result<(), DomainError> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::InvalidInput("invalid knowledge document id"));
    }
    Ok(())
}

fn next_id() -> String {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0_u128, |duration| duration.as_nanos());
    let sequence = u128::from(NEXT_ID.fetch_add(1, Ordering::Relaxed));
    format!("{:032x}", time ^ sequence)
}

fn epoch_millis() -> Result<u128, DomainError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|_| DomainError::Unavailable("system clock is before the Unix epoch"))
}

fn modality(extension: &str) -> &'static str {
    match extension {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" => "image",
        "pdf" => "pdf",
        "csv" | "tsv" => "sheet",
        "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "sql" | "sh" => "code",
        _ => "text",
    }
}

fn mime(extension: &str) -> Option<&'static str> {
    match extension {
        "md" => Some("text/markdown"),
        "txt" => Some("text/plain"),
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "pdf" => Some("application/pdf"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "svg" => Some("image/svg+xml"),
        _ => None,
    }
}

fn extract_text(path: &Path, modality: &str, caption: Option<&str>) -> Result<String, DomainError> {
    if matches!(modality, "text" | "code" | "sheet") {
        return Ok(fs::read_to_string(path).unwrap_or_default());
    }
    Ok(caption.unwrap_or_default().to_owned())
}

fn chunk_text(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::from("untitled")];
    }
    let mut chunks = Vec::new();
    let mut start = 0_usize;
    while start < chars.len() {
        let end = start.saturating_add(CHUNK_CHARS).min(chars.len());
        chunks.push(chars[start..end].iter().collect());
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(CHUNK_OVERLAP);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::{chunk_text, modality, safe_file_name, validate_id};

    #[test]
    fn upload_name_refuses_parent_components() {
        assert!(safe_file_name("../secret.txt").is_err());
    }

    #[test]
    fn document_id_requires_fixed_hex_shape() {
        assert!(validate_id("abc").is_err());
    }

    #[test]
    fn rust_source_is_code_modality() {
        assert_eq!(modality("rs"), "code");
    }

    #[test]
    fn empty_text_still_has_searchable_title_chunk() {
        assert_eq!(chunk_text("").as_slice(), [String::from("untitled")]);
    }
}
