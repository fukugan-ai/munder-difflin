use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use md_web_contracts::{HiveAgent, HiveMessage, HiveRegistry, HiveTask, TaskLedger};
use serde_json::{Map, Value};

/// Durable hive storage failure.
#[derive(Debug)]
pub enum HiveStoreError {
    InvalidRoot,
    InvalidLedger,
    DuplicateTask,
    TaskNotFound,
    LockPoisoned,
    Io(io::Error),
    Json(serde_json::Error),
}

impl From<io::Error> for HiveStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for HiveStoreError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Single-writer filesystem authority for one local hive directory.
pub struct HiveStore {
    root: PathBuf,
    writer: Mutex<()>,
}

impl HiveStore {
    /// Creates a store for an absolute, non-empty hive root.
    pub fn new(root: PathBuf) -> Result<Self, HiveStoreError> {
        if !root.is_absolute() || root.as_os_str().is_empty() {
            return Err(HiveStoreError::InvalidRoot);
        }
        Ok(Self {
            root,
            writer: Mutex::new(()),
        })
    }

    /// Reads typed task cards while preserving unknown fields on each card.
    pub fn tasks(&self) -> Result<TaskLedger, HiveStoreError> {
        let raw = self.read_tasks_value()?;
        serde_json::from_value(raw).map_err(HiveStoreError::Json)
    }

    /// Adds one task against the latest ledger without replacing other cards.
    pub fn add_task(&self, task: &HiveTask) -> Result<(), HiveStoreError> {
        let _guard = self.lock_writer()?;
        let mut ledger = self.read_tasks_value()?;
        let tasks = task_array_mut(&mut ledger)?;
        if tasks
            .iter()
            .any(|entry| task_id(entry) == Some(task.id.as_str()))
        {
            return Err(HiveStoreError::DuplicateTask);
        }
        tasks.push(serde_json::to_value(task)?);
        self.write_tasks_value(&ledger)
    }

    /// Applies only named fields to one raw card, preserving all unmentioned fields.
    pub fn patch_task(
        &self,
        id: &str,
        patch: &Map<String, Value>,
    ) -> Result<HiveTask, HiveStoreError> {
        let _guard = self.lock_writer()?;
        let mut ledger = self.read_tasks_value()?;
        let card = task_array_mut(&mut ledger)?
            .iter_mut()
            .find(|entry| task_id(entry) == Some(id))
            .ok_or(HiveStoreError::TaskNotFound)?;
        let object = card.as_object_mut().ok_or(HiveStoreError::InvalidLedger)?;
        for (key, value) in patch {
            if key != "id" {
                object.insert(key.clone(), value.clone());
            }
        }
        let updated = serde_json::from_value(Value::Object(object.clone()))?;
        self.write_tasks_value(&ledger)?;
        Ok(updated)
    }

    /// Deletes only the named task from the latest ledger.
    pub fn delete_task(&self, id: &str) -> Result<(), HiveStoreError> {
        let _guard = self.lock_writer()?;
        let mut ledger = self.read_tasks_value()?;
        let tasks = task_array_mut(&mut ledger)?;
        let before = tasks.len();
        tasks.retain(|entry| task_id(entry) != Some(id));
        if tasks.len() == before {
            return Err(HiveStoreError::TaskNotFound);
        }
        self.write_tasks_value(&ledger)
    }

    /// Reads the current registry, returning an empty roster when absent.
    pub fn registry(&self) -> Result<HiveRegistry, HiveStoreError> {
        let path = self.root.join("registry.json");
        if !path.exists() {
            return Ok(HiveRegistry::default());
        }
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(HiveStoreError::Json)
    }

    /// Reads the shared operator board.
    pub fn board(&self) -> Result<String, HiveStoreError> {
        let path = self.root.join("board.md");
        if !path.exists() {
            return Ok(String::new());
        }
        fs::read_to_string(path).map_err(Into::into)
    }

    /// Reads the newest structured Hive log rows, retaining malformed lines as raw text.
    pub fn log_tail(&self, limit: usize) -> Result<Vec<Value>, HiveStoreError> {
        let path = self.root.join("log.jsonl");
        if !path.exists() || limit == 0 {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(path)?;
        let mut rows: Vec<_> = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|_| serde_json::json!({ "raw": line }))
            })
            .collect();
        if rows.len() > limit {
            rows.drain(..rows.len() - limit);
        }
        Ok(rows)
    }

    /// Reads one agent's durable memory without accepting arbitrary paths.
    pub fn memory(&self, agent_id: &str) -> Result<String, HiveStoreError> {
        if !is_safe_segment(agent_id) {
            return Err(HiveStoreError::InvalidLedger);
        }
        let path = self.root.join("agents").join(agent_id).join("memory.md");
        if !path.exists() {
            return Ok(String::new());
        }
        fs::read_to_string(path).map_err(Into::into)
    }

    /// Patches one registry agent against the latest raw roster.
    pub fn patch_registry_agent(
        &self,
        agent_id: &str,
        patch: &Map<String, Value>,
    ) -> Result<HiveAgent, HiveStoreError> {
        if !is_safe_segment(agent_id) {
            return Err(HiveStoreError::InvalidLedger);
        }
        let _guard = self.lock_writer()?;
        let path = self.root.join("registry.json");
        let mut registry = if path.exists() {
            serde_json::from_slice::<Value>(&fs::read(&path)?)?
        } else {
            serde_json::json!({ "godId": null, "agents": {} })
        };
        let agent = registry
            .as_object_mut()
            .and_then(|root| root.get_mut("agents"))
            .and_then(Value::as_object_mut)
            .and_then(|agents| agents.get_mut(agent_id))
            .and_then(Value::as_object_mut)
            .ok_or(HiveStoreError::TaskNotFound)?;
        for (key, value) in patch {
            if key != "id" {
                agent.insert(key.clone(), value.clone());
            }
        }
        let updated = serde_json::from_value(Value::Object(agent.clone()))?;
        write_json_atomic(&path, &registry)?;
        Ok(updated)
    }

    /// Reads one agent's pending inbox without accepting arbitrary path input.
    pub fn inbox(&self, agent_id: &str) -> Result<Vec<HiveMessage>, HiveStoreError> {
        if !is_safe_segment(agent_id) {
            return Err(HiveStoreError::InvalidLedger);
        }
        read_message_dir(&self.root.join("agents").join(agent_id).join("inbox"))
    }

    /// Reads recent deduplicated inbox/outbox traffic for the Threads view.
    pub fn messages(&self, limit: usize) -> Result<Vec<HiveMessage>, HiveStoreError> {
        let agents_root = self.root.join("agents");
        if !agents_root.is_dir() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut messages = Vec::new();
        for entry in fs::read_dir(agents_root)? {
            let entry = entry?;
            let agent_id = entry.file_name();
            let Some(agent_id) = agent_id.to_str() else {
                continue;
            };
            if !entry.file_type()?.is_dir() || !is_safe_segment(agent_id) {
                continue;
            }
            let base = entry.path();
            for path in [
                base.join("inbox"),
                base.join("inbox").join(".done"),
                base.join("outbox"),
                base.join("outbox").join(".sent"),
            ] {
                messages.extend(read_message_dir(&path)?);
            }
        }
        messages.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        let mut seen = BTreeSet::new();
        messages.retain(|message| seen.insert(message.id.clone()));
        messages.truncate(limit.min(500));
        Ok(messages)
    }

    /// Writes a normalized message into one agent inbox using atomic replacement.
    pub fn deliver_message(
        &self,
        agent_id: &str,
        message_id: &str,
        message: &Value,
    ) -> Result<(), HiveStoreError> {
        let _guard = self.lock_writer()?;
        if !is_safe_segment(agent_id) || !is_safe_segment(message_id) {
            return Err(HiveStoreError::InvalidLedger);
        }
        let inbox = self.root.join("agents").join(agent_id).join("inbox");
        if !inbox.is_dir() {
            return Err(HiveStoreError::TaskNotFound);
        }
        write_json_atomic(&inbox.join(format!("{message_id}.json")), message)
    }

    fn lock_writer(&self) -> Result<MutexGuard<'_, ()>, HiveStoreError> {
        self.writer.lock().map_err(|_| HiveStoreError::LockPoisoned)
    }

    fn read_tasks_value(&self) -> Result<Value, HiveStoreError> {
        let path = self.root.join("tasks.json");
        if !path.exists() {
            return Ok(serde_json::json!({ "tasks": [] }));
        }
        let bytes = fs::read(path)?;
        serde_json::from_slice(&bytes).map_err(HiveStoreError::Json)
    }

    fn write_tasks_value(&self, ledger: &Value) -> Result<(), HiveStoreError> {
        fs::create_dir_all(&self.root)?;
        write_json_atomic(&self.root.join("tasks.json"), ledger)
    }
}

fn task_array_mut(ledger: &mut Value) -> Result<&mut Vec<Value>, HiveStoreError> {
    ledger
        .as_object_mut()
        .and_then(|object| object.get_mut("tasks"))
        .and_then(Value::as_array_mut)
        .ok_or(HiveStoreError::InvalidLedger)
}

fn task_id(task: &Value) -> Option<&str> {
    task.as_object()?.get("id")?.as_str()
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".." && !value.contains(['/', '\\'])
}

fn read_message_dir(path: &Path) -> Result<Vec<HiveMessage>, HiveStoreError> {
    if !path.is_dir() {
        return Ok(Vec::new());
    }
    let mut messages: Vec<HiveMessage> = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        if let Ok(message) = serde_json::from_slice(&bytes) {
            messages.push(message);
        }
    }
    messages.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(messages)
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<(), HiveStoreError> {
    let parent = path.parent().ok_or(HiveStoreError::InvalidRoot)?;
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.next");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), HiveStoreError> {
    let directory = File::open(path)?;
    directory.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    use md_web_contracts::{HiveTask, TaskStatus};
    use serde_json::{Map, Value};

    use super::{HiveStore, HiveStoreError};

    fn test_root(name: &str) -> Result<PathBuf, HiveStoreError> {
        let root = std::env::current_dir()?
            .join("target")
            .join("hive-task-tests")
            .join(name);
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(&root)?;
        Ok(root)
    }

    fn task() -> HiveTask {
        HiveTask {
            id: String::from("t-1"),
            title: String::from("Test"),
            description: None,
            assignee: None,
            status: TaskStatus::Todo,
            depends_on: Vec::new(),
            priority: 1,
            created_at: String::from("2026-08-25T00:00:00Z"),
            human_qa: Vec::new(),
            result: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn rejects_relative_root() {
        assert!(matches!(
            HiveStore::new(PathBuf::from("relative")),
            Err(HiveStoreError::InvalidRoot)
        ));
    }

    #[test]
    fn add_and_read_task() -> Result<(), HiveStoreError> {
        let store = HiveStore::new(test_root("add")?)?;
        store.add_task(&task())?;

        assert_eq!(store.tasks()?.tasks.len(), 1);
        Ok(())
    }

    #[test]
    fn patch_preserves_unknown_fields() -> Result<(), HiveStoreError> {
        let store = HiveStore::new(test_root("patch")?)?;
        let mut source = task();
        source
            .extra
            .insert(String::from("scope"), Value::String(String::from("keep")));
        store.add_task(&source)?;
        let mut patch = Map::new();
        patch.insert(String::from("status"), Value::String(String::from("doing")));
        store.patch_task("t-1", &patch)?;

        assert_eq!(
            store.tasks()?.tasks[0].extra.get("scope"),
            Some(&Value::String(String::from("keep")))
        );
        Ok(())
    }

    #[test]
    fn delete_missing_task_reports_error() -> Result<(), HiveStoreError> {
        let store = HiveStore::new(test_root("delete-missing")?)?;

        assert!(matches!(
            store.delete_task("none"),
            Err(HiveStoreError::TaskNotFound)
        ));
        Ok(())
    }

    #[test]
    fn empty_registry_is_default() -> Result<(), HiveStoreError> {
        let store = HiveStore::new(test_root("registry")?)?;

        assert!(store.registry()?.agents.is_empty());
        Ok(())
    }
}
