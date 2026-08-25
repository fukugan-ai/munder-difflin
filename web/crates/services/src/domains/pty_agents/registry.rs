use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::pty_agents::{
    AgentHookRequest, ProcessExit, PtyDimensions, PtyExitEvent, PtySummary, SpawnAgentRequest,
    SpawnAgentResult, TerminalPresence, TerminalServerFrame,
};

use super::error::PtyServiceError;
use super::hook::{AgentHookCapabilities, AgentHookLaunch};
use super::process::{NativePtyBackend, SpawnedProcess};
use super::provision::AgentHookRuntime;

const FRAME_CAPACITY: usize = 4096;
const MAX_ID_BYTES: usize = 128;
const MAX_ARGS: usize = 512;
const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const QUEUE_ENTER_DELAY: Duration = Duration::from_millis(140);
const EXIT_MONITOR_INTERVAL: Duration = Duration::from_millis(40);

pub(crate) struct OutputBuffer {
    frames: VecDeque<TerminalServerFrame>,
    next_seq: u64,
    last_output_at_ms: i64,
    has_output: bool,
}

impl OutputBuffer {
    fn new() -> Self {
        Self {
            frames: VecDeque::with_capacity(FRAME_CAPACITY),
            next_seq: 1,
            last_output_at_ms: 0,
            has_output: false,
        }
    }

    pub(crate) fn push_output(&mut self, pty_id: &str, generation: u64, data: &str) {
        self.has_output = true;
        self.last_output_at_ms = now_ms();
        let frame = TerminalServerFrame::Output {
            pty_id: String::from(pty_id),
            generation,
            seq: self.next_seq,
            data: String::from(data),
        };
        self.next_seq = self.next_seq.saturating_add(1);
        self.push(frame);
    }

    fn push(&mut self, frame: TerminalServerFrame) {
        if self.frames.len() == FRAME_CAPACITY {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }
}

struct ProcessSession {
    id: String,
    cwd: String,
    command: String,
    dimensions: PtyDimensions,
    generation: u64,
    process: SpawnedProcess,
    output: Arc<Mutex<OutputBuffer>>,
    exit_events: Arc<Mutex<VecDeque<PtyExitEvent>>>,
    hook_capabilities: Arc<AgentHookCapabilities>,
    hook: Option<AgentHookLaunch>,
    presence: TerminalPresence,
    exit_emitted: bool,
}

impl ProcessSession {
    fn summary(&self) -> Result<PtySummary, PtyServiceError> {
        let output = self
            .output
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        Ok(PtySummary {
            id: self.id.clone(),
            cwd: self.cwd.clone(),
            command: self.command.clone(),
            pid: self.process.pid,
            process_group_id: self.process.process_group_id,
            dimensions: self.dimensions,
            last_output_at_ms: output.last_output_at_ms,
            has_output: output.has_output,
        })
    }

    fn refresh_exit(&mut self) -> Result<bool, PtyServiceError> {
        if self.exit_emitted {
            return Ok(true);
        }
        let Some(status) = self.process.child.try_wait().map_err(PtyServiceError::Io)? else {
            return Ok(false);
        };
        self.exit_emitted = true;
        let mut output = self
            .output
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let exit = ProcessExit {
            exit_code: i32::try_from(status.exit_code()).ok(),
            signal: status.signal().map(String::from),
        };
        let frame = TerminalServerFrame::Exited {
            pty_id: self.id.clone(),
            generation: self.generation,
            seq: output.next_seq,
            exit: exit.clone(),
        };
        output.next_seq = output.next_seq.saturating_add(1);
        output.push(frame);
        self.exit_events
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?
            .push_back(PtyExitEvent {
                pty_id: self.id.clone(),
                generation: self.generation,
                exit,
            });
        if let Some(hook) = &self.hook {
            self.hook_capabilities
                .remove_if_matches(hook.agent_id(), hook.capability())?;
        }
        self.process.cleanup_hook_runtime()?;
        Ok(true)
    }
}

/// Process-lifetime registry for locally spawned agent terminals.
pub struct PtyRegistry {
    sessions: Mutex<BTreeMap<String, ProcessSession>>,
    generations: Mutex<BTreeMap<String, u64>>,
    delivery_locks: Mutex<BTreeMap<String, Arc<Mutex<()>>>>,
    exit_events: Arc<Mutex<VecDeque<PtyExitEvent>>>,
    hook_capabilities: Arc<AgentHookCapabilities>,
    exit_monitor_started: AtomicBool,
    backend: NativePtyBackend,
}

impl Default for PtyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PtyRegistry {
    /// Creates an empty process registry.
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(BTreeMap::new()),
            generations: Mutex::new(BTreeMap::new()),
            delivery_locks: Mutex::new(BTreeMap::new()),
            exit_events: Arc::new(Mutex::new(VecDeque::new())),
            hook_capabilities: Arc::new(AgentHookCapabilities::new()),
            exit_monitor_started: AtomicBool::new(false),
            backend: NativePtyBackend,
        }
    }

    /// Spawns one process after validating its stable identity, cwd, argv, and resume contract.
    pub fn spawn(&self, request: SpawnAgentRequest) -> Result<SpawnAgentResult, PtyServiceError> {
        self.spawn_with_hook(request, None)
    }

    /// Spawns one process with optional server-only, per-generation Hive hook credentials.
    pub fn spawn_with_hook(
        &self,
        request: SpawnAgentRequest,
        hook: Option<AgentHookLaunch>,
    ) -> Result<SpawnAgentResult, PtyServiceError> {
        validate_request(&request)?;
        if let Some(hook) = &hook
            && normalized_agent_id(&request.id) != normalized_agent_id(hook.agent_id())
        {
            return Err(PtyServiceError::InvalidRequest(
                "hookのエージェントIDが起動対象と一致しません。",
            ));
        }
        if request.require_resume
            && (!request.resume
                || request
                    .resume_session_id
                    .as_deref()
                    .is_none_or(str::is_empty))
        {
            return Err(PtyServiceError::ResumeUnavailable);
        }
        let pty_id = if request.id.starts_with("pty-") {
            request.id.clone()
        } else {
            format!("pty-{}", request.id)
        };
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        if sessions
            .get_mut(&pty_id)
            .map(ProcessSession::refresh_exit)
            .transpose()?
            == Some(false)
        {
            return Err(PtyServiceError::Conflict);
        }
        // A completed generation remains available for replay until its stable id is reused.
        // Keep it until replacement spawn succeeds, so a failed restore loses no replay state.
        let canonical_cwd = canonical_cwd(&request.cwd)?;
        let generation = {
            let mut generations = self
                .generations
                .lock()
                .map_err(|_| PtyServiceError::StatePoisoned)?;
            let next = generations
                .get(&pty_id)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            generations.insert(pty_id.clone(), next);
            next
        };
        let output = Arc::new(Mutex::new(OutputBuffer::new()));
        let hook = hook.filter(|_| AgentHookRuntime::supports(request.provider));
        if let Some(hook) = &hook {
            // Install immediately before spawn so a fast child can authenticate its first hook.
            self.hook_capabilities.rotate(hook)?;
        }
        let process = match self.backend.spawn(
            &request,
            hook.as_ref(),
            Arc::clone(&output),
            generation,
            &pty_id,
        ) {
            Ok(process) => process,
            Err(error) => {
                if let Some(hook) = &hook {
                    self.hook_capabilities
                        .remove_if_matches(hook.agent_id(), hook.capability())?;
                }
                return Err(error);
            }
        };
        let resumed = process.resumed;
        let hook_supported = process.hook_supported;
        sessions.insert(
            pty_id.clone(),
            ProcessSession {
                id: pty_id.clone(),
                cwd: canonical_cwd.clone(),
                command: request.command,
                dimensions: PtyDimensions {
                    cols: request.cols,
                    rows: request.rows,
                },
                generation,
                process,
                output,
                exit_events: Arc::clone(&self.exit_events),
                hook_capabilities: Arc::clone(&self.hook_capabilities),
                hook,
                presence: TerminalPresence::default(),
                exit_emitted: false,
            },
        );
        Ok(SpawnAgentResult {
            pty_id,
            cwd: canonical_cwd,
            worktree_path: None,
            resumed,
            resume_not_found: false,
            seed_prompt: None,
            hook_supported,
        })
    }

    /// Verifies the identity-bound bearer capability before integration invokes Hive policy.
    pub fn verify_hook_request(&self, request: &AgentHookRequest) -> Result<bool, PtyServiceError> {
        self.hook_capabilities.verify_request(request)
    }

    /// Updates browser interaction ownership for automated-delivery readiness.
    pub fn update_presence(
        &self,
        pty_id: &str,
        presence: TerminalPresence,
    ) -> Result<(), PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        if session.refresh_exit()? {
            return Err(PtyServiceError::ProcessExited);
        }
        session.presence = presence;
        Ok(())
    }

    pub fn presence(&self, pty_id: &str) -> Result<TerminalPresence, PtyServiceError> {
        self.sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?
            .get(pty_id)
            .map(|session| session.presence)
            .ok_or(PtyServiceError::NotFound)
    }

    /// Writes raw terminal bytes to a live process.
    pub fn write(&self, pty_id: &str, data: &str) -> Result<(), PtyServiceError> {
        let generation = self.current_generation(pty_id)?;
        self.write_generation(pty_id, generation, data)
    }

    fn write_generation(
        &self,
        pty_id: &str,
        generation: u64,
        data: &str,
    ) -> Result<(), PtyServiceError> {
        if data.len() > MAX_INPUT_BYTES || data.contains('\0') {
            return Err(PtyServiceError::InvalidRequest(
                "ターミナル入力が大きすぎるか、NUL文字を含んでいます。",
            ));
        }
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        if session.generation != generation {
            return Err(PtyServiceError::Conflict);
        }
        if session.refresh_exit()? {
            return Err(PtyServiceError::ProcessExited);
        }
        session
            .process
            .input
            .write_all(data.as_bytes())
            .and_then(|()| session.process.input.flush())
            .map_err(PtyServiceError::Io)
    }

    /// Delivers one queued prompt atomically for this PTY generation, then presses Enter.
    pub fn deliver_queued_message(&self, pty_id: &str, text: &str) -> Result<(), PtyServiceError> {
        let payload = terminal_delivery_payload(text)?;
        let generation = self.current_generation(pty_id)?;
        let delivery_lock = self
            .delivery_locks
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?
            .entry(String::from(pty_id))
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _delivery = delivery_lock
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        self.write_generation(pty_id, generation, &payload)?;
        thread::sleep(QUEUE_ENTER_DELAY);
        self.write_generation(pty_id, generation, "\r")
    }

    /// Returns the generation currently owning a stable PTY id.
    pub fn current_generation(&self, pty_id: &str) -> Result<u64, PtyServiceError> {
        self.sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?
            .get(pty_id)
            .map(|session| session.generation)
            .ok_or(PtyServiceError::NotFound)
    }

    /// Records the current grid. The native PTY adapter consumes this value during integration.
    pub fn resize(&self, pty_id: &str, dimensions: PtyDimensions) -> Result<(), PtyServiceError> {
        if dimensions.cols == 0 || dimensions.rows == 0 {
            return Err(PtyServiceError::InvalidRequest(
                "ターミナルの行数と列数は1以上にしてください。",
            ));
        }
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        if session.refresh_exit()? {
            return Err(PtyServiceError::ProcessExited);
        }
        session.process.resize(dimensions)?;
        session.dimensions = dimensions;
        Ok(())
    }

    /// Requests a fresh TUI frame by re-applying the current dimensions.
    pub fn redraw(&self, pty_id: &str) -> Result<PtyDimensions, PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        if session.refresh_exit()? {
            return Err(PtyServiceError::ProcessExited);
        }
        session.process.resize(session.dimensions)?;
        Ok(session.dimensions)
    }

    /// Stops one process and removes its live registry entry.
    pub fn kill(&self, pty_id: &str) -> Result<(), PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        sessions
            .get_mut(pty_id)
            .ok_or(PtyServiceError::NotFound)?
            .process
            .terminate()?;
        if let Some(hook) = sessions
            .get(pty_id)
            .and_then(|session| session.hook.as_ref())
        {
            self.hook_capabilities
                .remove_if_matches(hook.agent_id(), hook.capability())?;
        }
        sessions.remove(pty_id);
        drop(sessions);
        let _ = self
            .delivery_locks
            .lock()
            .map(|mut locks| locks.remove(pty_id));
        Ok(())
    }

    /// Kills a replacement only if it still owns the expected generation.
    pub fn kill_generation(&self, pty_id: &str, generation: u64) -> Result<bool, PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        if session.generation != generation {
            return Ok(false);
        }
        session.process.terminate()?;
        if let Some(hook) = &session.hook {
            self.hook_capabilities
                .remove_if_matches(hook.agent_id(), hook.capability())?;
        }
        sessions.remove(pty_id);
        drop(sessions);
        let _ = self
            .delivery_locks
            .lock()
            .map(|mut locks| locks.remove(pty_id));
        Ok(true)
    }

    /// Returns deterministic summaries for live processes only.
    pub fn list(&self) -> Result<Vec<PtySummary>, PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let mut summaries = Vec::with_capacity(sessions.len());
        for session in sessions.values_mut() {
            if !session.refresh_exit()? {
                summaries.push(session.summary()?);
            }
        }
        Ok(summaries)
    }

    /// Reports whether a socket frame belongs to the generation currently owning this PTY id.
    pub fn is_current_generation(
        &self,
        pty_id: &str,
        generation: u64,
    ) -> Result<bool, PtyServiceError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get(pty_id).ok_or(PtyServiceError::NotFound)?;
        Ok(session.generation == generation)
    }

    /// Drains ordered frames newer than `after_seq`, retaining no duplicate browser delivery.
    pub fn drain_frames(
        &self,
        pty_id: &str,
        after_seq: u64,
    ) -> Result<Vec<TerminalServerFrame>, PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        let _ = session.refresh_exit()?;
        let output = session
            .output
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        Ok(output
            .frames
            .iter()
            .filter(|frame| frame_seq(frame).is_none_or(|seq| seq > after_seq))
            .cloned()
            .collect())
    }

    /// Returns an attach receipt followed by the retained ordered replay window.
    pub fn attach_frames(
        &self,
        pty_id: &str,
        after_seq: u64,
    ) -> Result<Vec<TerminalServerFrame>, PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get_mut(pty_id).ok_or(PtyServiceError::NotFound)?;
        let _ = session.refresh_exit()?;
        let summary = session.summary()?;
        let output = session
            .output
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let oldest_seq = output
            .frames
            .iter()
            .find_map(frame_seq)
            .unwrap_or(output.next_seq);
        let mut frames = Vec::with_capacity(output.frames.len().saturating_add(1));
        frames.push(TerminalServerFrame::Attached {
            pty: summary,
            generation: session.generation,
            oldest_seq,
            next_seq: output.next_seq,
            truncated: after_seq > 0 && after_seq.saturating_add(1) < oldest_seq,
        });
        frames.extend(
            output
                .frames
                .iter()
                .filter(|frame| frame_seq(frame).is_some_and(|seq| seq > after_seq))
                .cloned(),
        );
        Ok(frames)
    }

    /// Emits an ordered generation marker before an integration-owned restart.
    pub fn mark_relaunching(&self, pty_id: &str) -> Result<(), PtyServiceError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let session = sessions.get(pty_id).ok_or(PtyServiceError::NotFound)?;
        let mut output = session
            .output
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        let frame = TerminalServerFrame::Relaunching {
            pty_id: String::from(pty_id),
            generation: session.generation,
            seq: output.next_seq,
        };
        output.next_seq = output.next_seq.saturating_add(1);
        output.push(frame);
        Ok(())
    }

    /// Returns IDs whose child exited, emitting each exit frame once.
    pub fn collect_exits(&self) -> Result<Vec<String>, PtyServiceError> {
        Ok(self
            .collect_exit_events()?
            .into_iter()
            .map(|event| event.pty_id)
            .collect())
    }

    /// Returns natural exits queued by any registry operation since the previous collection.
    pub fn collect_exit_events(&self) -> Result<Vec<PtyExitEvent>, PtyServiceError> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?;
        for session in sessions.values_mut() {
            let _ = session.refresh_exit()?;
        }
        drop(sessions);
        Ok(self
            .exit_events
            .lock()
            .map_err(|_| PtyServiceError::StatePoisoned)?
            .drain(..)
            .collect())
    }

    /// Starts the single process-owned natural-exit monitor; no browser connection is required.
    pub fn start_exit_monitor<F>(self: &Arc<Self>, callback: F) -> Result<(), PtyServiceError>
    where
        F: Fn(PtyExitEvent) + Send + Sync + 'static,
    {
        if self
            .exit_monitor_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(PtyServiceError::Conflict);
        }
        let registry = Arc::downgrade(self);
        let callback = Arc::new(callback);
        let spawned = thread::Builder::new()
            .name(String::from("md-pty-exit-monitor"))
            .spawn(move || {
                loop {
                    let Some(registry) = registry.upgrade() else {
                        return;
                    };
                    if let Ok(events) = registry.collect_exit_events() {
                        for event in events {
                            callback(event);
                        }
                    }
                    drop(registry);
                    thread::sleep(EXIT_MONITOR_INTERVAL);
                }
            });
        if let Err(error) = spawned {
            self.exit_monitor_started.store(false, Ordering::Release);
            return Err(PtyServiceError::Io(error));
        }
        Ok(())
    }
}

fn terminal_delivery_payload(text: &str) -> Result<String, PtyServiceError> {
    if text.trim().is_empty() || text.len() > MAX_INPUT_BYTES || text.contains('\0') {
        return Err(PtyServiceError::InvalidRequest(
            "ターミナル入力が大きすぎるか、NUL文字を含んでいます。",
        ));
    }
    if !text.contains(['\r', '\n']) {
        return Ok(String::from(text));
    }
    let normalized = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r");
    Ok(format!("\u{1b}[200~{normalized}\u{1b}[201~"))
}

fn validate_request(request: &SpawnAgentRequest) -> Result<(), PtyServiceError> {
    let id = request.id.strip_prefix("pty-").unwrap_or(&request.id);
    if id.is_empty()
        || id.len() > MAX_ID_BYTES
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(PtyServiceError::InvalidRequest(
            "エージェントIDの形式が正しくありません。",
        ));
    }
    if request.command.trim().is_empty() || request.command.contains('\0') {
        return Err(PtyServiceError::InvalidRequest(
            "起動コマンドを指定してください。",
        ));
    }
    if request.args.len() > MAX_ARGS || request.args.iter().any(|arg| arg.contains('\0')) {
        return Err(PtyServiceError::InvalidRequest(
            "起動引数の形式が正しくありません。",
        ));
    }
    if request.cols == 0 || request.rows == 0 {
        return Err(PtyServiceError::InvalidRequest(
            "ターミナルの行数と列数は1以上にしてください。",
        ));
    }
    let path = Path::new(&request.cwd);
    if !path.is_absolute() || !path.is_dir() {
        return Err(PtyServiceError::InvalidRequest(
            "作業フォルダーが存在しません。",
        ));
    }
    Ok(())
}

fn canonical_cwd(cwd: &str) -> Result<String, PtyServiceError> {
    let path = PathBuf::from(cwd)
        .canonicalize()
        .map_err(PtyServiceError::Io)?;
    path.into_os_string()
        .into_string()
        .map_err(|_| PtyServiceError::InvalidRequest("作業フォルダーをUTF-8で表現できません。"))
}

fn normalized_agent_id(id: &str) -> &str {
    id.strip_prefix("pty-").unwrap_or(id)
}

fn frame_seq(frame: &TerminalServerFrame) -> Option<u64> {
    match frame {
        TerminalServerFrame::Output { seq, .. }
        | TerminalServerFrame::Exited { seq, .. }
        | TerminalServerFrame::Relaunching { seq, .. } => Some(*seq),
        TerminalServerFrame::Attached { .. }
        | TerminalServerFrame::Readiness { .. }
        | TerminalServerFrame::Error { .. } => None,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::sync::Arc;
    #[cfg(unix)]
    use std::sync::mpsc;
    #[cfg(unix)]
    use std::thread;
    #[cfg(unix)]
    use std::time::Duration;

    use md_web_contracts::domains::pty_agents::{
        AgentHookEvent, AgentHookRequest, AgentProvider, AgentRole, PtyDimensions,
        SpawnAgentRequest, SpawnAgentResult, TerminalServerFrame,
    };
    use serde_json::Value;

    use super::{PtyRegistry, terminal_delivery_payload};
    use crate::domains::pty_agents::{AgentHookLaunch, PtyServiceError};

    fn request(id: &str) -> SpawnAgentRequest {
        SpawnAgentRequest {
            id: String::from(id),
            name: String::from("Dev"),
            provider: AgentProvider::Custom,
            role: AgentRole::default(),
            description: String::new(),
            cwd: std::env::current_dir()
                .ok()
                .and_then(|path| path.into_os_string().into_string().ok())
                .unwrap_or_else(|| String::from("/")),
            command: String::from("missing-command-for-validation-only"),
            args: Vec::new(),
            model: None,
            cols: 100,
            rows: 30,
            isolate: false,
            resume: false,
            require_resume: false,
            resume_session_id: None,
        }
    }

    fn attached_generation(registry: &PtyRegistry, pty_id: &str) -> Option<u64> {
        registry
            .attach_frames(pty_id, 0)
            .ok()?
            .into_iter()
            .find_map(|frame| match frame {
                TerminalServerFrame::Attached { generation, .. } => Some(generation),
                _ => None,
            })
    }

    fn hook_launch(agent_id: &str, capability: &str) -> Result<AgentHookLaunch, PtyServiceError> {
        AgentHookLaunch::new(
            "http://127.0.0.1:5001/internal/hive-hook",
            agent_id,
            capability,
            std::env::current_dir().map_err(PtyServiceError::Io)?,
        )
    }

    fn hook_request(agent_id: &str, capability: &str) -> AgentHookRequest {
        AgentHookRequest {
            agent_id: String::from(agent_id),
            capability: String::from(capability),
            event_id: String::from("evt-1"),
            event: AgentHookEvent::PreToolUse,
            tool_name: Some(String::from("Bash")),
            payload: Value::Null,
        }
    }

    #[test]
    fn new_registry_is_empty() {
        let registry = PtyRegistry::new();
        assert!(matches!(registry.list(), Ok(items) if items.is_empty()));
    }

    #[test]
    fn spawn_rejects_empty_id_before_process_launch() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.spawn(request("")),
            Err(PtyServiceError::InvalidRequest(_))
        ));
    }

    #[test]
    fn spawn_rejects_hook_identity_mismatch_before_process_launch() -> Result<(), PtyServiceError> {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.spawn_with_hook(
                request("dev-1"),
                Some(hook_launch(
                    "other-agent",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )?)
            ),
            Err(PtyServiceError::InvalidRequest(_))
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failed_native_spawn_removes_preinstalled_hook_capability() -> Result<(), PtyServiceError> {
        let registry = PtyRegistry::new();
        let capability = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut spawn = request("hook-spawn-failure");
        spawn.provider = AgentProvider::Claude;
        assert!(
            registry
                .spawn_with_hook(spawn, Some(hook_launch("hook-spawn-failure", capability)?))
                .is_err()
        );
        assert!(matches!(
            registry.verify_hook_request(&hook_request("hook-spawn-failure", capability)),
            Ok(false)
        ));
        Ok(())
    }

    #[test]
    fn write_rejects_unknown_terminal() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.write("pty-missing", "x"),
            Err(PtyServiceError::NotFound)
        ));
    }

    #[test]
    fn resize_rejects_zero_dimensions() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.resize("pty-missing", PtyDimensions { cols: 0, rows: 1 }),
            Err(PtyServiceError::InvalidRequest(_))
        ));
    }

    #[test]
    fn redraw_rejects_unknown_terminal() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.redraw("pty-missing"),
            Err(PtyServiceError::NotFound)
        ));
    }

    #[test]
    fn kill_rejects_unknown_terminal() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.kill("pty-missing"),
            Err(PtyServiceError::NotFound)
        ));
    }

    #[test]
    fn drain_rejects_unknown_terminal() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.drain_frames("pty-missing", 0),
            Err(PtyServiceError::NotFound)
        ));
    }

    #[test]
    fn attach_rejects_unknown_terminal() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.attach_frames("pty-missing", 0),
            Err(PtyServiceError::NotFound)
        ));
    }

    #[test]
    fn relaunch_marker_rejects_unknown_terminal() {
        let registry = PtyRegistry::new();
        assert!(matches!(
            registry.mark_relaunching("pty-missing"),
            Err(PtyServiceError::NotFound)
        ));
    }

    #[test]
    fn collect_exits_is_empty_without_sessions() {
        let registry = PtyRegistry::new();
        assert!(matches!(registry.collect_exits(), Ok(items) if items.is_empty()));
    }

    #[test]
    fn multiline_queue_delivery_uses_bracketed_paste_and_normalized_carriage_returns() {
        assert!(matches!(
            terminal_delivery_payload("一行目\n二行目\r\n三行目"),
            Ok(payload) if payload == "\u{1b}[200~一行目\r二行目\r三行目\u{1b}[201~"
        ));
    }

    #[test]
    fn single_line_queue_delivery_is_not_wrapped_as_paste() {
        assert!(matches!(
            terminal_delivery_payload("日本語の指示"),
            Ok(payload) if payload == "日本語の指示"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn native_pty_streams_input_output_and_resizes() {
        let registry = PtyRegistry::new();
        let mut spawn = request("native-smoke");
        spawn.command = String::from("/bin/sh");
        spawn.args = vec![
            String::from("-c"),
            String::from("printf ready; read line; printf 'got:%s' \"$line\""),
        ];
        assert!(registry.spawn(spawn).is_ok());
        let generation = attached_generation(&registry, "pty-native-smoke").unwrap_or(0);
        assert!(generation > 0);
        assert!(matches!(
            registry.is_current_generation("pty-native-smoke", generation),
            Ok(true)
        ));
        assert!(
            registry
                .resize(
                    "pty-native-smoke",
                    PtyDimensions {
                        cols: 132,
                        rows: 43
                    }
                )
                .is_ok()
        );
        assert!(registry.write("pty-native-smoke", "ping\n").is_ok());

        let mut saw_ready = false;
        let mut saw_ping = false;
        for _ in 0..200 {
            let frames = registry
                .drain_frames("pty-native-smoke", 0)
                .unwrap_or_default();
            let text = frames
                .iter()
                .filter_map(|frame| match frame {
                    TerminalServerFrame::Output { data, .. } => Some(data.as_str()),
                    _ => None,
                })
                .collect::<String>();
            saw_ready |= text.contains("ready");
            saw_ping |= text.contains("ping") || text.contains("got:ping");
            if saw_ready && saw_ping {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_ready && saw_ping);
        let _ = registry.kill("pty-native-smoke");
    }

    #[cfg(unix)]
    #[test]
    fn queued_delivery_preserves_fifo_text_and_enter_order() {
        let registry = PtyRegistry::new();
        let mut spawn = request("queue-order");
        spawn.command = String::from("/bin/sh");
        spawn.args = vec![
            String::from("-c"),
            String::from(
                "IFS= read -r first; IFS= read -r second; printf 'LINES:%s|%s' \"$first\" \"$second\"",
            ),
        ];
        assert!(registry.spawn(spawn).is_ok());
        assert!(
            registry
                .deliver_queued_message("pty-queue-order", "最初")
                .is_ok()
        );
        assert!(
            registry
                .deliver_queued_message("pty-queue-order", "次")
                .is_ok()
        );

        let mut ordered = false;
        for _ in 0..200 {
            let frames = registry
                .drain_frames("pty-queue-order", 0)
                .unwrap_or_default();
            let text = frames
                .iter()
                .filter_map(|frame| match frame {
                    TerminalServerFrame::Output { data, .. } => Some(data.as_str()),
                    _ => None,
                })
                .collect::<String>();
            if text.contains("LINES:最初|次") {
                ordered = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(ordered);
    }

    #[cfg(unix)]
    #[test]
    fn exited_generation_can_be_restored_under_the_same_stable_id() {
        let registry = PtyRegistry::new();
        let mut first = request("restore-smoke");
        first.command = String::from("/bin/sh");
        first.args = vec![String::from("-c"), String::from("exit 0")];
        let first_result = registry.spawn(first);
        assert!(matches!(
            &first_result,
            Ok(result) if result.pty_id == "pty-restore-smoke"
        ));
        let exited_generation = attached_generation(&registry, "pty-restore-smoke").unwrap_or(0);
        assert!(exited_generation > 0);

        let mut saw_exit = false;
        for _ in 0..200 {
            if matches!(
                registry.collect_exits(),
                Ok(ids) if ids.iter().any(|id| id == "pty-restore-smoke")
            ) {
                saw_exit = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_exit);

        assert!(registry.spawn(request("restore-smoke")).is_err());
        assert!(registry.attach_frames("pty-restore-smoke", 0).is_ok());

        let mut replacement = request("restore-smoke");
        replacement.command = String::from("/bin/sh");
        replacement.args = vec![String::from("-c"), String::from("read line")];
        let replacement_result = registry.spawn(replacement);
        assert!(matches!(
            &replacement_result,
            Ok(result) if result.pty_id == "pty-restore-smoke"
        ));
        let current_generation = attached_generation(&registry, "pty-restore-smoke").unwrap_or(0);
        assert!(current_generation > exited_generation);
        assert!(matches!(
            registry.is_current_generation("pty-restore-smoke", exited_generation),
            Ok(false)
        ));
        assert!(matches!(
            registry.is_current_generation("pty-restore-smoke", current_generation),
            Ok(true)
        ));
        assert!(registry.kill("pty-restore-smoke").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn exit_monitor_reports_natural_exit_without_websocket_polling() {
        let registry = Arc::new(PtyRegistry::new());
        let (sender, receiver) = mpsc::channel();
        assert!(
            registry
                .start_exit_monitor(move |event| {
                    let _ = sender.send(event);
                })
                .is_ok()
        );
        let mut spawn = request("no-ws-exit");
        spawn.command = String::from("/bin/sh");
        spawn.args = vec![String::from("-c"), String::from("exit 7")];
        assert!(registry.spawn(spawn).is_ok());

        let event = receiver.recv_timeout(Duration::from_secs(3));
        assert!(matches!(
            event,
            Ok(event)
                if event.pty_id == "pty-no-ws-exit"
                    && event.generation > 0
                    && event.exit.exit_code == Some(7)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn natural_exit_removes_generation_hook_capability() -> Result<(), PtyServiceError> {
        let registry = PtyRegistry::new();
        let capability = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut spawn = request("hook-natural-exit");
        spawn.provider = AgentProvider::Claude;
        spawn.command = String::from("/bin/sh");
        spawn.args = vec![String::from("-c"), String::from("exit 0")];
        assert!(
            registry
                .spawn_with_hook(spawn, Some(hook_launch("hook-natural-exit", capability)?))
                .is_ok()
        );
        assert!(matches!(
            registry.verify_hook_request(&hook_request("hook-natural-exit", capability)),
            Ok(true)
        ));

        let mut cleaned = false;
        for _ in 0..200 {
            let _ = registry.collect_exits();
            if matches!(
                registry.verify_hook_request(&hook_request("hook-natural-exit", capability)),
                Ok(false)
            ) {
                cleaned = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(cleaned);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn explicit_kill_removes_hook_capability() -> Result<(), PtyServiceError> {
        let registry = PtyRegistry::new();
        let capability = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut spawn = request("hook-kill");
        spawn.provider = AgentProvider::Claude;
        spawn.command = String::from("/bin/sh");
        spawn.args = vec![String::from("-c"), String::from("read line")];
        assert!(
            registry
                .spawn_with_hook(spawn, Some(hook_launch("hook-kill", capability)?))
                .is_ok()
        );
        assert!(registry.kill("pty-hook-kill").is_ok());
        assert!(matches!(
            registry.verify_hook_request(&hook_request("hook-kill", capability)),
            Ok(false)
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unsupported_hook_providers_still_spawn_with_control_limitation()
    -> Result<(), PtyServiceError> {
        for (name, provider) in [
            ("grok", AgentProvider::Grok),
            ("kimi", AgentProvider::Kimi),
            ("antigravity", AgentProvider::Antigravity),
            ("qwen", AgentProvider::Qwen),
            ("opencode", AgentProvider::OpenCode),
            ("crush", AgentProvider::Crush),
            ("pi", AgentProvider::Pi),
            ("copilot", AgentProvider::Copilot),
            ("cursor", AgentProvider::Cursor),
            ("custom", AgentProvider::Custom),
        ] {
            let id = format!("unsupported-{name}");
            let mut spawn = request(&id);
            spawn.provider = provider;
            spawn.command = String::from("/bin/sh");
            spawn.args = vec![String::from("-c"), String::from("read line")];
            let result = registry_spawn_unsupported(&spawn, &id)?;
            assert!(!result.hook_supported);
        }
        Ok(())
    }

    #[cfg(unix)]
    fn registry_spawn_unsupported(
        spawn: &SpawnAgentRequest,
        id: &str,
    ) -> Result<SpawnAgentResult, PtyServiceError> {
        let registry = PtyRegistry::new();
        let result = registry.spawn_with_hook(
            spawn.clone(),
            Some(hook_launch(id, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?),
        )?;
        registry.kill(&result.pty_id)?;
        Ok(result)
    }

    #[cfg(unix)]
    #[test]
    fn cas_failure_cleanup_kills_only_the_expected_replacement_generation() {
        let registry = PtyRegistry::new();
        let mut spawn = request("restart-cas");
        spawn.command = String::from("/bin/sh");
        spawn.args = vec![String::from("-c"), String::from("read line")];
        assert!(registry.spawn(spawn).is_ok());
        let generation = attached_generation(&registry, "pty-restart-cas").unwrap_or(0);
        assert!(generation > 0);

        assert!(matches!(
            registry.kill_generation("pty-restart-cas", generation.saturating_add(1)),
            Ok(false)
        ));
        assert!(matches!(registry.list(), Ok(ptys) if ptys.len() == 1));
        assert!(matches!(
            registry.kill_generation("pty-restart-cas", generation),
            Ok(true)
        ));
        assert!(matches!(registry.list(), Ok(ptys) if ptys.is_empty()));
    }
}
