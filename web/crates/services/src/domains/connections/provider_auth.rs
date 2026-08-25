use std::collections::HashMap;
use std::fmt::Display;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::connections::{
    CliAuthPhase, CliAuthProvider, CliAuthSnapshot, CliAuthView,
};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

use super::ConnectionsServiceError;

const OUTPUT_LIMIT: usize = 64 * 1024;
const START_TIMEOUT: Duration = Duration::from_secs(20);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const STATUS_TIMEOUT: Duration = Duration::from_secs(8);
const TERMINATE_GRACE: Duration = Duration::from_millis(500);
const SERVER_ONLY_ENV_KEYS: &[&str] = &[
    "MD_PG_PASSWORD",
    "MD_PG_HOST",
    "MD_PG_PORT",
    "MD_PG_DATABASE",
    "MD_PG_USER",
    "MD_PG_NAMESPACE",
    "MD_PG_TLS_CA",
    "MD_HIVE_HOOK_URL",
    "MD_HIVE_AGENT_ID",
    "MD_HIVE_HOOK_CAPABILITY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "CLAUDE_CODE_OAUTH_TOKEN",
];

#[derive(Clone, Copy)]
struct Timeouts {
    start: Duration,
    login: Duration,
    status: Duration,
    terminate_grace: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            start: START_TIMEOUT,
            login: LOGIN_TIMEOUT,
            status: STATUS_TIMEOUT,
            terminate_grace: TERMINATE_GRACE,
        }
    }
}

struct AuthSession {
    view: Mutex<CliAuthView>,
    child: Mutex<Option<Box<dyn Child + Send + Sync>>>,
    input: Mutex<Option<Box<dyn Write + Send>>>,
    process_group_id: Option<i64>,
    executable: PathBuf,
    provider: CliAuthProvider,
    cancelled: std::sync::atomic::AtomicBool,
    timeouts: Timeouts,
}

struct SpawnedAuthProcess {
    child: Box<dyn Child + Send + Sync>,
    input: Box<dyn Write + Send>,
    reader: Box<dyn Read + Send>,
    process_group_id: Option<i64>,
}

impl AuthSession {
    fn view(&self) -> Result<CliAuthView, ConnectionsServiceError> {
        Ok(lock(&self.view)?.clone())
    }

    fn set_phase(
        &self,
        phase: CliAuthPhase,
        detail_ja: &'static str,
    ) -> Result<(), ConnectionsServiceError> {
        let mut view = lock(&self.view)?;
        view.phase = phase;
        view.detail_ja = String::from(detail_ja);
        view.can_cancel = matches!(
            phase,
            CliAuthPhase::Starting | CliAuthPhase::AwaitingUser | CliAuthPhase::Verifying
        );
        view.retryable = matches!(
            phase,
            CliAuthPhase::Failed
                | CliAuthPhase::Cancelled
                | CliAuthPhase::TimedOut
                | CliAuthPhase::SignedOut
                | CliAuthPhase::StatusUnknown
        );
        if !matches!(phase, CliAuthPhase::AwaitingUser) {
            view.verification_uri = None;
            view.user_code = None;
            view.accepts_code_input = false;
        }
        Ok(())
    }

    fn terminate(&self, phase: CliAuthPhase) -> Result<(), ConnectionsServiceError> {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        let _ = self.set_phase(phase, phase_detail(phase));
        let mut child = lock(&self.child)?;
        let Some(child) = child.as_mut() else {
            return Ok(());
        };
        signal_group(self.process_group_id, nix::sys::signal::Signal::SIGHUP)?;
        let deadline = Instant::now() + self.timeouts.terminate_grace;
        while Instant::now() < deadline {
            if child
                .try_wait()
                .map_err(|error| ConnectionsServiceError::Runtime(error.to_string()))?
                .is_some()
            {
                *lock(&self.input)? = None;
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
        signal_group(self.process_group_id, nix::sys::signal::Signal::SIGKILL)?;
        child
            .kill()
            .map_err(|error| ConnectionsServiceError::Runtime(error.to_string()))?;
        let _ = child.wait();
        *lock(&self.input)? = None;
        Ok(())
    }
}

/// Process-lifetime owner for short-lived CLI browser registration processes.
/// Only structured, sanitized state can leave this service.
pub struct ProviderAuthRegistry {
    sessions: Mutex<HashMap<CliAuthProvider, Arc<AuthSession>>>,
    generations: Mutex<HashMap<CliAuthProvider, u64>>,
    timeouts: Timeouts,
}

impl Default for ProviderAuthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderAuthRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            generations: Mutex::new(HashMap::new()),
            timeouts: Timeouts::default(),
        }
    }

    #[cfg(test)]
    fn with_timeouts(timeouts: Timeouts) -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            generations: Mutex::new(HashMap::new()),
            timeouts,
        }
    }

    pub fn snapshot(
        &self,
        executables: &[(CliAuthProvider, Option<PathBuf>)],
    ) -> Result<CliAuthSnapshot, ConnectionsServiceError> {
        let sessions = lock(&self.sessions)?;
        let mut providers = Vec::with_capacity(executables.len());
        for (provider, executable) in executables {
            if let Some(session) = sessions.get(provider) {
                providers.push(session.view()?);
            } else if let Some(executable) = executable {
                providers.push(status_view(*provider, executable, 0, self.timeouts.status));
            } else {
                providers.push(base_view(
                    *provider,
                    CliAuthPhase::NotInstalled,
                    0,
                    "CLIが見つかりません",
                ));
            }
        }
        Ok(CliAuthSnapshot { providers })
    }

    pub fn start(
        &self,
        provider: CliAuthProvider,
        executable: &Path,
    ) -> Result<CliAuthView, ConnectionsServiceError> {
        let mut sessions = lock(&self.sessions)?;
        if let Some(existing) = sessions.get(&provider) {
            let phase = existing.view()?.phase;
            if matches!(
                phase,
                CliAuthPhase::Starting | CliAuthPhase::AwaitingUser | CliAuthPhase::Verifying
            ) {
                return Err(ConnectionsServiceError::InvalidInput(
                    "provider auth already active",
                ));
            }
        }

        let generation = {
            let mut generations = lock(&self.generations)?;
            let next = generations
                .get(&provider)
                .copied()
                .unwrap_or(0)
                .saturating_add(1);
            generations.insert(provider, next);
            next
        };
        let spawned = spawn_login(provider, executable)?;
        let now = epoch_ms();
        let deadline =
            now.saturating_add(u64::try_from(self.timeouts.login.as_millis()).unwrap_or(u64::MAX));
        let session = Arc::new(AuthSession {
            view: Mutex::new(CliAuthView {
                provider,
                phase: CliAuthPhase::Starting,
                generation,
                verification_uri: None,
                user_code: None,
                deadline_at_ms: Some(deadline),
                can_cancel: true,
                accepts_code_input: false,
                retryable: false,
                detail_ja: String::from("認証を開始しています"),
            }),
            child: Mutex::new(Some(spawned.child)),
            input: Mutex::new(Some(spawned.input)),
            process_group_id: spawned.process_group_id,
            executable: executable.to_path_buf(),
            provider,
            cancelled: std::sync::atomic::AtomicBool::new(false),
            timeouts: self.timeouts,
        });
        sessions.insert(provider, Arc::clone(&session));
        spawn_reader(Arc::clone(&session), spawned.reader)?;
        spawn_timeouts(session)?;
        let current = sessions
            .get(&provider)
            .ok_or(ConnectionsServiceError::StateUnavailable)?;
        current.view()
    }

    pub fn poll(
        &self,
        provider: CliAuthProvider,
        generation: u64,
    ) -> Result<CliAuthView, ConnectionsServiceError> {
        let sessions = lock(&self.sessions)?;
        let session = sessions
            .get(&provider)
            .ok_or(ConnectionsServiceError::NotFound("provider auth session"))?;
        let view = session.view()?;
        if view.generation != generation {
            return Err(ConnectionsServiceError::InvalidInput(
                "stale auth generation",
            ));
        }
        Ok(view)
    }

    pub fn submit_code(
        &self,
        provider: CliAuthProvider,
        generation: u64,
        code: &str,
    ) -> Result<CliAuthView, ConnectionsServiceError> {
        if !valid_device_code(code.trim()) {
            return Err(ConnectionsServiceError::InvalidInput("device code"));
        }
        let sessions = lock(&self.sessions)?;
        let session = sessions
            .get(&provider)
            .ok_or(ConnectionsServiceError::NotFound("provider auth session"))?;
        let view = session.view()?;
        if view.generation != generation || !view.accepts_code_input {
            return Err(ConnectionsServiceError::InvalidInput(
                "device code not requested",
            ));
        }
        let mut input = lock(&session.input)?;
        let writer = input
            .as_mut()
            .ok_or(ConnectionsServiceError::StateUnavailable)?;
        writer
            .write_all(format!("{}\r", code.trim()).as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|error| ConnectionsServiceError::Runtime(error.to_string()))?;
        drop(input);
        session.set_phase(CliAuthPhase::Verifying, "CLIで接続を確認しています")?;
        session.view()
    }

    pub fn cancel(
        &self,
        provider: CliAuthProvider,
        generation: u64,
    ) -> Result<CliAuthView, ConnectionsServiceError> {
        let sessions = lock(&self.sessions)?;
        let session = sessions
            .get(&provider)
            .ok_or(ConnectionsServiceError::NotFound("provider auth session"))?;
        if session.view()?.generation != generation {
            return Err(ConnectionsServiceError::InvalidInput(
                "stale auth generation",
            ));
        }
        session.terminate(CliAuthPhase::Cancelled)?;
        session.view()
    }

    pub fn shutdown_all(&self) -> Result<(), ConnectionsServiceError> {
        for session in lock(&self.sessions)?.values() {
            let phase = session.view()?.phase;
            if matches!(
                phase,
                CliAuthPhase::Starting | CliAuthPhase::AwaitingUser | CliAuthPhase::Verifying
            ) {
                session.terminate(CliAuthPhase::Cancelled)?;
            }
        }
        Ok(())
    }
}

pub fn provider_auth_registry() -> &'static ProviderAuthRegistry {
    static REGISTRY: OnceLock<ProviderAuthRegistry> = OnceLock::new();
    REGISTRY.get_or_init(ProviderAuthRegistry::new)
}

fn spawn_login(
    provider: CliAuthProvider,
    executable: &Path,
) -> Result<SpawnedAuthProcess, ConnectionsServiceError> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(runtime_error)?;
    let command = login_command(provider, executable);
    let child = pair.slave.spawn_command(command).map_err(runtime_error)?;
    drop(pair.slave);
    let reader = pair.master.try_clone_reader().map_err(runtime_error)?;
    let input = pair.master.take_writer().map_err(runtime_error)?;
    #[cfg(unix)]
    let process_group_id = pair.master.process_group_leader().map(i64::from);
    #[cfg(not(unix))]
    let process_group_id = None;
    Ok(SpawnedAuthProcess {
        child,
        input,
        reader,
        process_group_id,
    })
}

fn login_command(provider: CliAuthProvider, executable: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(executable);
    command.args(login_args(provider));
    for key in SERVER_ONLY_ENV_KEYS {
        command.env_remove(key);
    }
    command
}

fn spawn_reader(
    session: Arc<AuthSession>,
    mut reader: Box<dyn Read + Send>,
) -> Result<(), ConnectionsServiceError> {
    thread::Builder::new()
        .name(format!(
            "md-{}-auth",
            session.provider.label().to_lowercase()
        ))
        .spawn(move || {
            let mut parser = AuthOutputParser::new(session.provider);
            let mut bytes = [0_u8; 2048];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(read) => {
                        if let Some(parsed) = parser.push(&bytes[..read])
                            && let Ok(mut view) = session.view.lock()
                        {
                            view.phase = CliAuthPhase::AwaitingUser;
                            view.verification_uri = parsed.uri;
                            view.user_code = parsed.code;
                            view.accepts_code_input = parsed.accepts_code_input;
                            view.can_cancel = true;
                            view.detail_ja = String::from("ブラウザーでサインインを続けてください");
                        }
                    }
                    Err(_) => {
                        let _ = session
                            .set_phase(CliAuthPhase::Failed, "CLIの応答を読み取れませんでした");
                        return;
                    }
                }
            }
            if session.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let successful = lock(&session.child)
                .ok()
                .and_then(|mut child| child.as_mut().and_then(|child| child.wait().ok()))
                .is_some_and(|status| status.success());
            if !successful {
                let _ = session.set_phase(CliAuthPhase::Failed, "CLI認証を完了できませんでした");
                return;
            }
            let _ = session.set_phase(CliAuthPhase::Verifying, "CLIで接続を確認しています");
            let view = status_view(
                session.provider,
                &session.executable,
                session.view().map_or(0, |view| view.generation),
                session.timeouts.status,
            );
            if let Ok(mut current) = session.view.lock() {
                *current = view;
            }
        })
        .map(|_| ())
        .map_err(|error| ConnectionsServiceError::Runtime(error.to_string()))
}

fn spawn_timeouts(session: Arc<AuthSession>) -> Result<(), ConnectionsServiceError> {
    thread::Builder::new()
        .name(format!(
            "md-{}-auth-timeout",
            session.provider.label().to_lowercase()
        ))
        .spawn(move || {
            let started = Instant::now();
            loop {
                let phase = session.view().map(|view| view.phase);
                if !matches!(
                    phase,
                    Ok(CliAuthPhase::Starting
                        | CliAuthPhase::AwaitingUser
                        | CliAuthPhase::Verifying)
                ) {
                    return;
                }
                if started.elapsed() >= session.timeouts.login
                    || (started.elapsed() >= session.timeouts.start
                        && matches!(phase, Ok(CliAuthPhase::Starting)))
                {
                    let _ = session.terminate(CliAuthPhase::TimedOut);
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
        .map(|_| ())
        .map_err(|error| ConnectionsServiceError::Runtime(error.to_string()))
}

fn status_view(
    provider: CliAuthProvider,
    executable: &Path,
    generation: u64,
    timeout: Duration,
) -> CliAuthView {
    let mut command = Command::new(executable);
    command
        .args(status_args(provider))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    for key in SERVER_ONLY_ENV_KEYS {
        command.env_remove(key);
    }
    let Ok(mut child) = command.spawn() else {
        return base_view(
            provider,
            CliAuthPhase::StatusUnknown,
            generation,
            "接続状態を確認できません",
        );
    };
    #[cfg(unix)]
    let status_process_group_id = Some(i64::from(child.id()));
    let output_reader = child.stdout.take().and_then(|stdout| {
        thread::Builder::new()
            .name(format!(
                "md-{}-auth-status",
                provider.label().to_lowercase()
            ))
            .spawn(move || read_bounded(stdout))
            .ok()
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                #[cfg(unix)]
                {
                    let _ = signal_group(status_process_group_id, nix::sys::signal::Signal::SIGHUP);
                    thread::sleep(Duration::from_millis(25));
                    let _ =
                        signal_group(status_process_group_id, nix::sys::signal::Signal::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = output_reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    match status {
        Some(status) if status.success() && parse_status_output(&output) == Some(true) => {
            base_view(provider, CliAuthPhase::Connected, generation, "接続済み")
        }
        Some(status) if !status.success() || parse_status_output(&output) == Some(false) => {
            base_view(provider, CliAuthPhase::SignedOut, generation, "未接続")
        }
        _ => base_view(
            provider,
            CliAuthPhase::StatusUnknown,
            generation,
            "接続状態を確認できません",
        ),
    }
}

fn base_view(
    provider: CliAuthProvider,
    phase: CliAuthPhase,
    generation: u64,
    detail_ja: &'static str,
) -> CliAuthView {
    CliAuthView {
        provider,
        phase,
        generation,
        verification_uri: None,
        user_code: None,
        deadline_at_ms: None,
        can_cancel: false,
        accepts_code_input: false,
        retryable: matches!(
            phase,
            CliAuthPhase::SignedOut
                | CliAuthPhase::StatusUnknown
                | CliAuthPhase::Failed
                | CliAuthPhase::Cancelled
                | CliAuthPhase::TimedOut
        ),
        detail_ja: String::from(detail_ja),
    }
}

const fn login_args(provider: CliAuthProvider) -> &'static [&'static str] {
    match provider {
        CliAuthProvider::Codex => &["login", "--device-auth"],
        CliAuthProvider::Claude => &["auth", "login"],
    }
}

const fn status_args(provider: CliAuthProvider) -> &'static [&'static str] {
    match provider {
        CliAuthProvider::Codex => &["login", "status"],
        CliAuthProvider::Claude => &["auth", "status"],
    }
}

struct ParsedAuthOutput {
    uri: Option<String>,
    code: Option<String>,
    accepts_code_input: bool,
}

struct AuthOutputParser {
    provider: CliAuthProvider,
    bytes: Vec<u8>,
}

impl AuthOutputParser {
    fn new(provider: CliAuthProvider) -> Self {
        Self {
            provider,
            bytes: Vec::with_capacity(4096),
        }
    }

    fn push(&mut self, bytes: &[u8]) -> Option<ParsedAuthOutput> {
        let remaining = OUTPUT_LIMIT.saturating_sub(self.bytes.len());
        self.bytes
            .extend_from_slice(&bytes[..bytes.len().min(remaining)]);
        let text = String::from_utf8_lossy(&self.bytes);
        let clean = strip_ansi(&text);
        let uri = extract_https_uri(self.provider, &text)
            .or_else(|| extract_https_uri(self.provider, &clean));
        let code = extract_device_code(&clean);
        let lower = clean.to_ascii_lowercase();
        let accepts_code_input = lower.contains("enter code")
            || lower.contains("paste code")
            || lower.contains("input code");
        (uri.is_some() || code.is_some() || accepts_code_input).then_some(ParsedAuthOutput {
            uri,
            code,
            accepts_code_input,
        })
    }
}

fn strip_ansi(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            if !character.is_control() || matches!(character, '\n' | '\r' | '\t') {
                output.push(character);
            }
            continue;
        }
        match chars.next() {
            Some(']') => {
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        let _ = chars.next();
                        break;
                    }
                }
            }
            Some('[') => {
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    output
}

fn extract_https_uri(provider: CliAuthProvider, text: &str) -> Option<String> {
    let mut offset = 0;
    while let Some(relative) = text[offset..].find("https://") {
        let start = offset + relative;
        let end = text[start..]
            .find(|character: char| {
                character.is_control()
                    || character.is_whitespace()
                    || matches!(character, '<' | '>' | '"' | '\'')
            })
            .map_or(text.len(), |end| start + end);
        let candidate = text[start..end].trim_end_matches([')', ']', ',', '.', ';']);
        if valid_provider_uri(provider, candidate) {
            return Some(String::from(candidate));
        }
        offset = end.max(start.saturating_add(8));
    }
    None
}

fn valid_provider_uri(provider: CliAuthProvider, value: &str) -> bool {
    if value.len() > 4096 || value.contains('@') || value.chars().any(char::is_control) {
        return false;
    }
    let Some(authority) = value.strip_prefix("https://") else {
        return false;
    };
    let host = authority
        .split(['/', ':', '?', '#'])
        .next()
        .unwrap_or_default();
    let allowed = match provider {
        CliAuthProvider::Codex => {
            ["auth.openai.com", "platform.openai.com", "chatgpt.com"].as_slice()
        }
        CliAuthProvider::Claude => {
            ["claude.ai", "console.anthropic.com", "anthropic.com"].as_slice()
        }
    };
    allowed
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

fn extract_device_code(text: &str) -> Option<String> {
    text.split(|character: char| {
        character.is_whitespace() || matches!(character, ':' | '=' | '(' | ')' | '[' | ']')
    })
    .map(|token| {
        token.trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '-')
    })
    .find(|token| valid_device_code(token))
    .map(String::from)
}

fn valid_device_code(value: &str) -> bool {
    (4..=32).contains(&value.len())
        && value.contains('-')
        && value.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '-'
        })
        && value
            .chars()
            .any(|character| character.is_ascii_alphanumeric())
}

fn parse_status_output(output: &[u8]) -> Option<bool> {
    let lower = String::from_utf8_lossy(output).to_ascii_lowercase();
    let compact: String = lower
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect();
    if compact.contains("\"loggedin\":false")
        || compact.contains("\"authenticated\":false")
        || lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("unauthenticated")
    {
        Some(false)
    } else if compact.contains("\"loggedin\":true")
        || compact.contains("\"authenticated\":true")
        || (lower.contains("logged in") && !lower.contains("not logged in"))
        || (lower.contains("authenticated")
            && !lower.contains("not authenticated")
            && !lower.contains("unauthenticated"))
    {
        Some(true)
    } else {
        None
    }
}

fn read_bounded(mut reader: impl Read) -> Vec<u8> {
    let mut retained = Vec::with_capacity(4096);
    let mut bytes = [0_u8; 2048];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) | Err(_) => return retained,
            Ok(read) => {
                let remaining = OUTPUT_LIMIT.saturating_sub(retained.len());
                retained.extend_from_slice(&bytes[..read.min(remaining)]);
            }
        }
    }
}

fn phase_detail(phase: CliAuthPhase) -> &'static str {
    match phase {
        CliAuthPhase::Cancelled => "認証をキャンセルしました",
        CliAuthPhase::TimedOut => "認証がタイムアウトしました",
        _ => "CLI認証を終了しました",
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, ConnectionsServiceError> {
    mutex
        .lock()
        .map_err(|_| ConnectionsServiceError::StateUnavailable)
}

fn runtime_error(error: impl Display) -> ConnectionsServiceError {
    ConnectionsServiceError::Runtime(error.to_string())
}

#[cfg(unix)]
fn signal_group(
    process_group_id: Option<i64>,
    signal: nix::sys::signal::Signal,
) -> Result<(), ConnectionsServiceError> {
    let Some(raw_id) = process_group_id.and_then(|id| i32::try_from(id).ok()) else {
        return Ok(());
    };
    match nix::sys::signal::killpg(nix::unistd::Pid::from_raw(raw_id), signal) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(error) => Err(runtime_error(error)),
    }
}

#[cfg(not(unix))]
fn signal_group(
    _process_group_id: Option<i64>,
    _signal: (),
) -> Result<(), ConnectionsServiceError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[cfg(unix)]
    struct FakeCli {
        root: PathBuf,
        executable: PathBuf,
    }

    #[cfg(unix)]
    impl FakeCli {
        fn new(script: &str) -> Result<Self, std::io::Error> {
            static NEXT_FAKE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let unique = format!(
                "md-provider-auth-{}-{}-{}",
                std::process::id(),
                epoch_ms(),
                NEXT_FAKE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let root = std::env::temp_dir().join(unique);
            fs::create_dir(&root)?;
            let executable = root.join("fake-cli");
            fs::write(&executable, script)?;
            let mut permissions = fs::metadata(&executable)?.permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&executable, permissions)?;
            Ok(Self { root, executable })
        }
    }

    #[cfg(unix)]
    impl Drop for FakeCli {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(unix)]
    fn wait_for_phase(
        registry: &ProviderAuthRegistry,
        provider: CliAuthProvider,
        generation: u64,
        phase: CliAuthPhase,
    ) -> Result<CliAuthView, ConnectionsServiceError> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let view = registry.poll(provider, generation)?;
            if view.phase == phase || Instant::now() >= deadline {
                return Ok(view);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn parser_accepts_split_utf8_and_plain_official_uri() {
        let mut parser = AuthOutputParser::new(CliAuthProvider::Codex);
        assert!(
            parser
                .push(b"Open https://auth.openai.com/device and use AB")
                .is_some()
        );
        let parsed = parser.push(b"CD-1234");
        assert!(parsed.is_some_and(|value| value.code.as_deref() == Some("ABCD-1234")));
    }

    #[test]
    fn login_command_uses_exact_argv_and_preserves_credential_home() {
        let command = login_command(CliAuthProvider::Codex, Path::new("/opt/bin/codex"));
        let argv: Vec<_> = command
            .get_argv()
            .iter()
            .filter_map(|value| value.to_str())
            .collect();
        assert_eq!(argv, ["/opt/bin/codex", "login", "--device-auth"]);
        assert!(
            SERVER_ONLY_ENV_KEYS
                .iter()
                .all(|key| command.get_env(*key).is_none())
        );
        assert_eq!(command.get_env("HOME"), std::env::var_os("HOME").as_deref());
        assert_eq!(
            command.get_env("CODEX_HOME"),
            std::env::var_os("CODEX_HOME").as_deref()
        );
    }

    #[test]
    fn parser_rejects_unofficial_or_userinfo_uri() {
        assert_eq!(
            extract_https_uri(CliAuthProvider::Codex, "https://evil.test/device"),
            None
        );
        assert_eq!(
            extract_https_uri(
                CliAuthProvider::Codex,
                "https://user@auth.openai.com/device"
            ),
            None
        );
    }

    #[test]
    fn parser_accepts_osc8_official_uri() {
        let mut parser = AuthOutputParser::new(CliAuthProvider::Claude);
        let parsed = parser
            .push(b"\x1b]8;;https://claude.ai/oauth/authorize\x1b\\Open browser\x1b]8;;\x1b\\");
        assert!(parsed.is_some_and(|value| {
            value.uri.as_deref() == Some("https://claude.ai/oauth/authorize")
        }));
    }

    #[test]
    fn parser_window_is_bounded() {
        let mut parser = AuthOutputParser::new(CliAuthProvider::Codex);
        let oversized = vec![b'x'; OUTPUT_LIMIT + 1024];
        let _ = parser.push(&oversized);
        assert_eq!(parser.bytes.len(), OUTPUT_LIMIT);
    }

    #[test]
    fn letters_only_device_code_is_supported() {
        assert!(valid_device_code("ABCD-EFGH"));
    }

    #[test]
    fn status_parser_returns_only_boolean_auth_state() {
        assert_eq!(
            parse_status_output(br#"{"authenticated":true,"email":"private@example.test"}"#),
            Some(true)
        );
        assert_eq!(
            parse_status_output(br#"{"authenticated":false}"#),
            Some(false)
        );
        assert_eq!(parse_status_output(br#"{"loggedIn":false}"#), Some(false));
        assert_eq!(
            parse_status_output(br#"{"email":"private@example.test"}"#),
            None
        );
        assert_eq!(parse_status_output(b"not authenticated"), Some(false));
    }

    #[test]
    fn stale_generation_is_rejected() {
        let registry = ProviderAuthRegistry::with_timeouts(Timeouts {
            start: Duration::from_millis(20),
            login: Duration::from_millis(40),
            status: Duration::from_millis(20),
            terminate_grace: Duration::from_millis(5),
        });
        assert!(matches!(
            registry.poll(CliAuthProvider::Codex, 99),
            Err(ConnectionsServiceError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn fake_codex_login_requires_code_then_verifies_status()
    -> Result<(), Box<dyn std::error::Error>> {
        let cli = FakeCli::new(
            "#!/bin/sh\ncase \"$1:$2\" in\nlogin:--device-auth) printf 'Open https://auth.openai.com/device\\nEnter code ABCD-1234\\n'; read answer; test \"$answer\" = 'ABCD-1234';;\nlogin:status) printf '{\"authenticated\": true}';;\n*) exit 9;;\nesac\n",
        )?;
        let registry = ProviderAuthRegistry::with_timeouts(Timeouts {
            start: Duration::from_secs(1),
            login: Duration::from_secs(2),
            status: Duration::from_secs(1),
            terminate_grace: Duration::from_millis(20),
        });
        let started = registry.start(CliAuthProvider::Codex, &cli.executable)?;
        let waiting = wait_for_phase(
            &registry,
            CliAuthProvider::Codex,
            started.generation,
            CliAuthPhase::AwaitingUser,
        )?;
        assert_eq!(waiting.user_code.as_deref(), Some("ABCD-1234"));
        registry.submit_code(CliAuthProvider::Codex, started.generation, "ABCD-1234")?;
        let connected = wait_for_phase(
            &registry,
            CliAuthProvider::Codex,
            started.generation,
            CliAuthPhase::Connected,
        )?;
        assert_eq!(connected.phase, CliAuthPhase::Connected);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn fake_claude_login_can_be_cancelled_and_reaped() -> Result<(), Box<dyn std::error::Error>> {
        let cli = FakeCli::new(
            "#!/bin/sh\ncase \"$1:$2\" in\nauth:login) printf 'Open https://claude.ai/oauth/authorize\\n'; sleep 5;;\nauth:status) exit 1;;\n*) exit 9;;\nesac\n",
        )?;
        let registry = ProviderAuthRegistry::with_timeouts(Timeouts {
            start: Duration::from_secs(1),
            login: Duration::from_secs(2),
            status: Duration::from_millis(50),
            terminate_grace: Duration::from_millis(20),
        });
        let started = registry.start(CliAuthProvider::Claude, &cli.executable)?;
        let waiting = wait_for_phase(
            &registry,
            CliAuthProvider::Claude,
            started.generation,
            CliAuthPhase::AwaitingUser,
        )?;
        assert_eq!(waiting.phase, CliAuthPhase::AwaitingUser);
        let cancelled = registry.cancel(CliAuthProvider::Claude, started.generation)?;
        assert_eq!(cancelled.phase, CliAuthPhase::Cancelled);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_start_for_same_provider_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let cli = FakeCli::new(
            "#!/bin/sh\nprintf 'Open https://claude.ai/oauth/authorize\\n'\nsleep 5\n",
        )?;
        let registry = ProviderAuthRegistry::with_timeouts(Timeouts {
            start: Duration::from_secs(1),
            login: Duration::from_secs(2),
            status: Duration::from_millis(50),
            terminate_grace: Duration::from_millis(20),
        });
        let started = registry.start(CliAuthProvider::Claude, &cli.executable)?;
        assert!(matches!(
            registry.start(CliAuthProvider::Claude, &cli.executable),
            Err(ConnectionsServiceError::InvalidInput(
                "provider auth already active"
            ))
        ));
        let _ = registry.cancel(CliAuthProvider::Claude, started.generation)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn fake_login_without_auth_prompt_times_out() -> Result<(), Box<dyn std::error::Error>> {
        let cli = FakeCli::new("#!/bin/sh\nsleep 5\n")?;
        let registry = ProviderAuthRegistry::with_timeouts(Timeouts {
            start: Duration::from_millis(40),
            login: Duration::from_millis(80),
            status: Duration::from_millis(20),
            terminate_grace: Duration::from_millis(10),
        });
        let started = registry.start(CliAuthProvider::Codex, &cli.executable)?;
        let timed_out = wait_for_phase(
            &registry,
            CliAuthProvider::Codex,
            started.generation,
            CliAuthPhase::TimedOut,
        )?;
        assert_eq!(timed_out.phase, CliAuthPhase::TimedOut);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn fake_status_timeout_is_unknown_without_output_leak() -> Result<(), Box<dyn std::error::Error>>
    {
        let cli = FakeCli::new("#!/bin/sh\nsleep 5\nprintf 'private@example.test'\n")?;
        let started = Instant::now();
        let view = status_view(
            CliAuthProvider::Claude,
            &cli.executable,
            7,
            Duration::from_millis(20),
        );
        assert_eq!(view.phase, CliAuthPhase::StatusUnknown);
        assert!(!format!("{view:?}").contains("private@example.test"));
        assert!(started.elapsed() < Duration::from_secs(1));
        Ok(())
    }
}
