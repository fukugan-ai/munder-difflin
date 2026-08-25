use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use md_web_contracts::domains::memory_skills::{
    CommandOutcome, EmbeddingModel, MemorySearchRequest, MemoryStatus, ReflectResult,
    TextSearchHit, TextSearchResponse,
};

use super::{DomainError, ProcessControl};

const MAX_QUERY_CHARS: usize = 512;
const MAX_RESULTS: u16 = 100;
const MAX_HITS_PER_FILE: usize = 3;
const MAX_MEMORY_BYTES: u64 = 2 * 1024 * 1024;
const RECENT_SECTIONS_TO_KEEP: usize = 8;

pub struct MemoryService {
    hive_root: PathBuf,
    semantic_cli: Option<PathBuf>,
    enabled: bool,
    model: EmbeddingModel,
    process: ProcessControl,
}

impl MemoryService {
    pub fn new(
        hive_root: PathBuf,
        semantic_cli: Option<PathBuf>,
        enabled: bool,
        model: EmbeddingModel,
    ) -> Self {
        Self::with_process_control(
            hive_root,
            semantic_cli,
            enabled,
            model,
            ProcessControl::default(),
        )
    }

    pub fn with_process_control(
        hive_root: PathBuf,
        semantic_cli: Option<PathBuf>,
        enabled: bool,
        model: EmbeddingModel,
        process: ProcessControl,
    ) -> Self {
        Self {
            hive_root,
            semantic_cli,
            enabled,
            model,
            process,
        }
    }

    pub fn status(&self) -> MemoryStatus {
        let available = self.semantic_cli.as_deref().is_some_and(Path::is_file);
        let initialized = self.hive_root.join("palace").is_dir();
        MemoryStatus {
            available,
            enabled: self.enabled,
            active: available && self.enabled,
            initialized,
            model: self.model,
        }
    }

    pub fn read_agent_memory(&self, agent_id: &str) -> Result<String, DomainError> {
        validate_agent_id(agent_id)?;
        let path = self
            .hive_root
            .join("hive")
            .join("agents")
            .join(agent_id)
            .join("memory.md");
        read_bounded_text(&path, MAX_MEMORY_BYTES)
    }

    pub fn text_search(&self, query: &str) -> Result<TextSearchResponse, DomainError> {
        let query = normalize_query(query)?;
        let hive = self.hive_root.join("hive");
        let mut targets = vec![
            (hive.join("board.md"), String::from("board.md")),
            (hive.join("tasks.json"), String::from("tasks.json")),
        ];
        let agents = hive.join("agents");
        if let Ok(entries) = fs::read_dir(agents) {
            for entry in entries.flatten() {
                let id = entry.file_name().to_string_lossy().into_owned();
                if validate_agent_id(&id).is_ok() {
                    targets.push((entry.path().join("memory.md"), format!("{id}/memory.md")));
                }
            }
        }

        let mut results = Vec::new();
        for (path, source) in targets {
            if results.len() >= usize::from(MAX_RESULTS) || !path.is_file() {
                continue;
            }
            let text = match read_bounded_text(&path, MAX_MEMORY_BYTES) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let mut hits = 0_usize;
            for line in text.lines() {
                let Some(index) = line.to_lowercase().find(&query) else {
                    continue;
                };
                let excerpt = bounded_excerpt(line, index, query.len());
                results.push(TextSearchHit {
                    source: source.clone(),
                    excerpt,
                });
                hits += 1;
                if hits >= MAX_HITS_PER_FILE || results.len() >= usize::from(MAX_RESULTS) {
                    break;
                }
            }
        }
        Ok(TextSearchResponse { ok: true, results })
    }

    pub fn semantic_search(
        &self,
        request: &MemorySearchRequest,
    ) -> Result<CommandOutcome, DomainError> {
        let query = normalize_query(&request.query)?;
        let cli = self
            .semantic_cli
            .as_deref()
            .filter(|path| path.is_file())
            .ok_or(DomainError::Unavailable(
                "semantic memory CLI is not installed",
            ))?;
        if !self.enabled {
            return Err(DomainError::Unavailable("semantic memory is disabled"));
        }
        if let Some(wing) = request.wing.as_deref() {
            validate_agent_id(wing)?;
        }
        let mut command = Command::new(cli);
        command
            .arg("search")
            .arg(query)
            .arg("--results")
            .arg(request.results.clamp(1, MAX_RESULTS).to_string());
        if let Some(wing) = request.wing.as_deref() {
            command.arg("--wing").arg(wing);
        }
        let output = self.process.run(&mut command)?;
        Ok(command_outcome(output))
    }

    pub fn wake_up(&self, wing: Option<&str>) -> Result<CommandOutcome, DomainError> {
        let cli = self
            .semantic_cli
            .as_deref()
            .filter(|path| path.is_file())
            .ok_or(DomainError::Unavailable(
                "semantic memory CLI is not installed",
            ))?;
        if let Some(id) = wing {
            validate_agent_id(id)?;
        }
        let mut command = Command::new(cli);
        command.arg("wake-up");
        if let Some(id) = wing {
            command.arg("--wing").arg(id);
        }
        Ok(command_outcome(self.process.run(&mut command)?))
    }

    pub fn mine_agent(&self, agent_id: &str) -> Result<CommandOutcome, DomainError> {
        validate_agent_id(agent_id)?;
        let cli = self
            .semantic_cli
            .as_deref()
            .filter(|path| path.is_file())
            .ok_or(DomainError::Unavailable(
                "semantic memory CLI is not installed",
            ))?;
        let agent_dir = self.hive_root.join("hive").join("agents").join(agent_id);
        let mut command = Command::new(cli);
        command
            .arg("mine")
            .arg(agent_dir)
            .arg("--wing")
            .arg(agent_id)
            .arg("--agent")
            .arg(agent_id);
        let output = self.process.run(&mut command)?;
        Ok(command_outcome(output))
    }

    pub fn reflect_agent(
        &self,
        agent_id: &str,
        summarizer_cli: &Path,
    ) -> Result<ReflectResult, DomainError> {
        validate_agent_id(agent_id)?;
        if !summarizer_cli.is_file() {
            return Err(DomainError::Unavailable("summarizer CLI is not installed"));
        }
        let memory = self
            .hive_root
            .join("hive")
            .join("agents")
            .join(agent_id)
            .join("memory.md");
        let original = read_bounded_text(&memory, MAX_MEMORY_BYTES)?;
        let old_bytes = u64::try_from(original.len()).unwrap_or(u64::MAX);
        let sections = split_sections(&original);
        if sections.len() <= RECENT_SECTIONS_TO_KEEP {
            return Ok(ReflectResult {
                agent_id: agent_id.to_owned(),
                condensed: false,
                reason: String::from("nothing-to-evict"),
                old_bytes: Some(old_bytes),
                new_bytes: None,
            });
        }
        let split_at = sections.len() - RECENT_SECTIONS_TO_KEEP;
        let old = sections[..split_at].join("\n");
        let recent = sections[split_at..].join("\n");
        let prompt = format!(
            "次の長期メモリ履歴を、決定・原因・手順・path・数値を残して1500語以内に要約してください。要約本文だけを返してください。\n\n{old}"
        );
        let mut command = Command::new(summarizer_cli);
        command
            .arg("-p")
            .arg(prompt)
            .arg("--model")
            .arg("claude-haiku-4-5");
        let output = self.process.run(&mut command)?;
        if !output.status.success() {
            return Ok(ReflectResult {
                agent_id: agent_id.to_owned(),
                condensed: false,
                reason: String::from("summarize-failed"),
                old_bytes: Some(old_bytes),
                new_bytes: None,
            });
        }
        let summary = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if summary.is_empty() || summary.len() >= old.len() {
            return Ok(ReflectResult {
                agent_id: agent_id.to_owned(),
                condensed: false,
                reason: String::from("summary-verification-failed"),
                old_bytes: Some(old_bytes),
                new_bytes: None,
            });
        }
        let rebuilt = format!(
            "# Memory — {agent_id}\n\n## 🗜 Condensed history\n\n{summary}\n\n## Recent\n\n{recent}\n"
        );
        let new_bytes = u64::try_from(rebuilt.len()).unwrap_or(u64::MAX);
        let backup_dir = self
            .hive_root
            .join("hive")
            .join("backups")
            .join("reflect")
            .join(agent_id);
        fs::create_dir_all(&backup_dir)?;
        fs::copy(&memory, backup_dir.join("memory.md"))?;
        let temp = memory.with_extension("md.reflecting");
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        file.write_all(rebuilt.as_bytes())?;
        file.sync_all()?;
        fs::rename(temp, memory)?;
        Ok(ReflectResult {
            agent_id: agent_id.to_owned(),
            condensed: true,
            reason: String::from("condensed"),
            old_bytes: Some(old_bytes),
            new_bytes: Some(new_bytes),
        })
    }
}

fn normalize_query(query: &str) -> Result<String, DomainError> {
    let value = query.trim();
    if value.is_empty() {
        return Err(DomainError::InvalidInput("empty query"));
    }
    if value.chars().count() > MAX_QUERY_CHARS {
        return Err(DomainError::InvalidInput("query is too long"));
    }
    Ok(value.to_lowercase())
}

fn validate_agent_id(agent_id: &str) -> Result<(), DomainError> {
    if agent_id.is_empty()
        || agent_id.len() > 128
        || !agent_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(DomainError::InvalidInput("invalid agent id"));
    }
    Ok(())
}

fn read_bounded_text(path: &Path, limit: u64) -> Result<String, DomainError> {
    let metadata = fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            DomainError::NotFound
        } else {
            DomainError::Io(error)
        }
    })?;
    if metadata.len() > limit {
        return Err(DomainError::InvalidInput("file exceeds read limit"));
    }
    Ok(fs::read_to_string(path)?)
}

fn bounded_excerpt(line: &str, match_byte: usize, query_bytes: usize) -> String {
    let lower = line.to_lowercase();
    let before = lower[..match_byte].chars().count();
    let matched = lower[match_byte..match_byte.saturating_add(query_bytes).min(lower.len())]
        .chars()
        .count();
    let start = before.saturating_sub(40);
    line.chars()
        .skip(start)
        .take(matched.saturating_add(80))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn split_sections(text: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if line.starts_with("## ") && !current.trim().is_empty() {
            sections.push(std::mem::take(&mut current));
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        sections.push(current);
    }
    sections
}

fn command_outcome(output: std::process::Output) -> CommandOutcome {
    let ok = output.status.success();
    CommandOutcome {
        ok,
        output: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        error: (!ok).then(|| String::from_utf8_lossy(&output.stderr).trim().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_excerpt, normalize_query, split_sections, validate_agent_id};

    #[test]
    fn empty_query_is_rejected() {
        assert!(normalize_query("  ").is_err());
    }

    #[test]
    fn excerpt_handles_multibyte_text() {
        let line = "前段 日本語の決定 後段";
        let index = line.find("日本語").unwrap_or(0);

        assert!(bounded_excerpt(line, index, "日本語".len()).contains("日本語"));
    }

    #[test]
    fn sections_split_on_level_two_headings() {
        assert_eq!(split_sections("# A\n## One\na\n## Two\nb\n").len(), 3);
    }

    #[test]
    fn agent_id_refuses_path_components() {
        assert!(validate_agent_id("../outside").is_err());
    }
}
