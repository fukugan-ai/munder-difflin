use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use md_web_contracts::domains::memory_skills::ActivityEntry;
use serde_json::Value;

use super::DomainError;

const MAX_ACTIVITY_ROWS: usize = 1_000;
const MAX_LOG_LINE_BYTES: usize = 64 * 1024;

pub struct ActivityService {
    log_path: PathBuf,
}

impl ActivityService {
    pub fn new(log_path: PathBuf) -> Self {
        Self { log_path }
    }

    pub fn tail(&self, limit: usize) -> Result<Vec<ActivityEntry>, DomainError> {
        if !self.log_path.is_file() {
            return Ok(Vec::new());
        }
        let mut rows = VecDeque::with_capacity(limit.clamp(1, MAX_ACTIVITY_ROWS));
        let reader = BufReader::new(File::open(&self.log_path)?);
        for line in reader.lines() {
            let line = match line {
                Ok(value) if value.len() <= MAX_LOG_LINE_BYTES => value,
                _ => continue,
            };
            let entry = parse_entry(&line);
            if rows.len() == limit.clamp(1, MAX_ACTIVITY_ROWS) {
                rows.pop_front();
            }
            rows.push_back(entry);
        }
        Ok(rows.into_iter().collect())
    }
}

fn parse_entry(line: &str) -> ActivityEntry {
    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(line) else {
        return ActivityEntry {
            timestamp_ms: 0,
            kind: String::from("unknown"),
            summary: String::from("読み取れない履歴行"),
            details: BTreeMap::new(),
        };
    };
    let timestamp_ms = object
        .remove("ts")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
    let kind = object
        .remove("kind")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from("unknown"));
    // Deliberately exclude body/message/prompt fields. Activity is metadata-only.
    for forbidden in ["body", "message", "prompt", "content", "text"] {
        object.remove(forbidden);
    }
    let details = object
        .into_iter()
        .filter_map(|(key, value)| scalar_string(&value).map(|scalar| (key, scalar)))
        .collect();
    let summary = summarize(&kind, &details);
    ActivityEntry {
        timestamp_ms,
        kind,
        summary,
        details,
    }
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.chars().take(256).collect()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn summarize(kind: &str, details: &BTreeMap<String, String>) -> String {
    match kind {
        "spawn" => format!("{} を起動", detail(details, "name", "agentId")),
        "archive" => format!(
            "{} のアーカイブ状態を更新",
            detail(details, "agentId", "id")
        ),
        "message" => format!(
            "{} → {}: {}",
            details.get("from").map_or("?", String::as_str),
            details.get("to").map_or("?", String::as_str),
            details
                .get("subject")
                .or_else(|| details.get("act"))
                .map_or("message", String::as_str)
        ),
        "drain" => format!("{} が受信箱を処理", detail(details, "agentId", "id")),
        "app-start" => String::from("アプリを起動"),
        _ => kind.to_owned(),
    }
}

fn detail<'a>(details: &'a BTreeMap<String, String>, primary: &str, fallback: &str) -> &'a str {
    details
        .get(primary)
        .or_else(|| details.get(fallback))
        .map_or("?", String::as_str)
}

#[cfg(test)]
mod tests {
    use super::parse_entry;

    #[test]
    fn message_body_never_crosses_activity_boundary() {
        let entry = parse_entry(r#"{"ts":1,"kind":"message","from":"a","to":"b","body":"secret"}"#);

        assert!(!entry.details.contains_key("body"));
    }

    #[test]
    fn malformed_line_becomes_unknown_entry() {
        assert_eq!(parse_entry("not-json").kind, "unknown");
    }
}
