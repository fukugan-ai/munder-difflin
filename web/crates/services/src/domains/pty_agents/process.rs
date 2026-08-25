use std::fmt::Display;
use std::io::{BufReader, Read, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use md_web_contracts::domains::pty_agents::{AgentProvider, PtyDimensions, SpawnAgentRequest};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

use super::error::PtyServiceError;
use super::hook::{
    AgentHookLaunch, HOOK_AGENT_ID_ENV, HOOK_CAPABILITY_ENV, HOOK_ENV_KEYS, HOOK_URL_ENV,
};
use super::provision::AgentHookRuntime;
use super::registry::OutputBuffer;

const SERVER_ONLY_ENV_KEYS: [&str; 7] = [
    "MD_PG_PASSWORD",
    "MD_PG_HOST",
    "MD_PG_PORT",
    "MD_PG_DATABASE",
    "MD_PG_USER",
    "MD_PG_NAMESPACE",
    "MD_PG_TLS_CA",
];

/// Native PTY handles returned to the registry after a successful local spawn.
pub(crate) struct SpawnedProcess {
    pub child: Box<dyn Child + Send + Sync>,
    pub input: Box<dyn Write + Send>,
    pub master: Box<dyn MasterPty + Send>,
    pub pid: Option<u32>,
    pub process_group_id: Option<i64>,
    pub resumed: bool,
    pub hook_supported: bool,
    hook_runtime: Option<AgentHookRuntime>,
}

impl SpawnedProcess {
    pub(crate) fn resize(&self, dimensions: PtyDimensions) -> Result<(), PtyServiceError> {
        self.master
            .resize(pty_size(dimensions))
            .map_err(|error| PtyServiceError::Io(native_io(error)))
    }

    pub(crate) fn terminate(&mut self) -> Result<(), PtyServiceError> {
        if self
            .child
            .try_wait()
            .map_err(PtyServiceError::Io)?
            .is_some()
        {
            return Ok(());
        }
        terminate_process_group(self.process_group_id)?;
        for _ in 0..5 {
            if self
                .child
                .try_wait()
                .map_err(PtyServiceError::Io)?
                .is_some()
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        force_process_group(self.process_group_id)?;
        #[cfg(unix)]
        if self.process_group_id.is_none() {
            self.child.kill().map_err(PtyServiceError::Io)?;
        }
        #[cfg(not(unix))]
        self.child.kill().map_err(PtyServiceError::Io)?;
        self.child.wait().map(|_| ()).map_err(PtyServiceError::Io)
    }

    pub(crate) fn cleanup_hook_runtime(&mut self) -> Result<(), PtyServiceError> {
        if let Some(runtime) = &mut self.hook_runtime {
            runtime.cleanup()?;
        }
        self.hook_runtime = None;
        Ok(())
    }
}

/// Server-side native pseudo-terminal backend.
#[derive(Clone, Copy, Debug, Default)]
pub struct NativePtyBackend;

impl NativePtyBackend {
    /// Spawns exactly one executable with argv inside a native PTY, never through a shell.
    pub(crate) fn spawn(
        &self,
        request: &SpawnAgentRequest,
        hook: Option<&AgentHookLaunch>,
        output: Arc<Mutex<OutputBuffer>>,
        generation: u64,
        pty_id: &str,
    ) -> Result<SpawnedProcess, PtyServiceError> {
        let pair = native_pty_system()
            .openpty(pty_size(PtyDimensions {
                cols: request.cols,
                rows: request.rows,
            }))
            .map_err(|error| PtyServiceError::Spawn(native_io(error)))?;

        let (mut args, resumed) = launch_args(request)?;
        let effective_hook = hook.filter(|_| AgentHookRuntime::supports(request.provider));
        let hook_runtime = effective_hook
            .map(|hook| {
                AgentHookRuntime::provision(
                    request.provider,
                    &request.command,
                    hook,
                    generation,
                    pty_id,
                )
            })
            .transpose()?;
        if let Some(runtime) = &hook_runtime {
            runtime.apply_args(&mut args);
        }
        let mut command = CommandBuilder::new(&request.command);
        command.args(&args);
        command.cwd(&request.cwd);
        strip_server_only_environment(&mut command);
        apply_hook_environment(&mut command, effective_hook);
        if let Some(runtime) = &hook_runtime {
            for (key, value) in runtime.environment() {
                command.env(key, value);
            }
        }

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| PtyServiceError::Spawn(native_io(error)))?;
        drop(pair.slave);

        let pid = child.process_id();
        let process_group_id = process_group_id(pair.master.as_ref());
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                return Err(PtyServiceError::Io(native_io(error)));
            }
        };
        let input = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(error) => {
                let _ = child.kill();
                return Err(PtyServiceError::Io(native_io(error)));
            }
        };
        if let Err(error) = spawn_reader(reader, output, generation, pty_id) {
            let _ = child.kill();
            return Err(error);
        }

        Ok(SpawnedProcess {
            child,
            input,
            master: pair.master,
            pid,
            process_group_id,
            resumed,
            hook_supported: effective_hook.is_some(),
            hook_runtime,
        })
    }
}

fn launch_args(request: &SpawnAgentRequest) -> Result<(Vec<String>, bool), PtyServiceError> {
    let mut args = request.args.clone();
    if let Some(model) = request
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty())
        && request.provider != AgentProvider::Custom
        && !args.iter().any(|argument| argument == "--model")
    {
        args.push(String::from("--model"));
        args.push(String::from(model));
    }
    if !request.resume {
        return Ok((args, false));
    }
    let Some(session_id) = request
        .resume_session_id
        .as_deref()
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
    else {
        return if request.require_resume {
            Err(PtyServiceError::ResumeUnavailable)
        } else {
            Ok((args, false))
        };
    };

    let resumed = match request.provider {
        AgentProvider::Codex => {
            if args.first().is_none_or(|argument| argument != "resume") {
                args.insert(0, String::from(session_id));
                args.insert(0, String::from("resume"));
            }
            true
        }
        AgentProvider::Antigravity => {
            push_flag_value(&mut args, "--conversation", session_id);
            true
        }
        AgentProvider::Crush | AgentProvider::Pi => {
            push_flag_value(&mut args, "--session", session_id);
            true
        }
        AgentProvider::Claude
        | AgentProvider::Grok
        | AgentProvider::Gemini
        | AgentProvider::Copilot
        | AgentProvider::Cursor => {
            push_flag_value(&mut args, "--resume", session_id);
            true
        }
        AgentProvider::Kimi
        | AgentProvider::Qwen
        | AgentProvider::OpenCode
        | AgentProvider::Custom => {
            if request.require_resume {
                return Err(PtyServiceError::ResumeUnavailable);
            }
            false
        }
    };
    Ok((args, resumed))
}

fn push_flag_value(args: &mut Vec<String>, flag: &str, value: &str) {
    if !args.iter().any(|argument| argument == flag) {
        args.push(String::from(flag));
        args.push(String::from(value));
    }
}

fn strip_server_only_environment(command: &mut CommandBuilder) {
    for key in SERVER_ONLY_ENV_KEYS {
        command.env_remove(key);
    }
}

fn apply_hook_environment(command: &mut CommandBuilder, hook: Option<&AgentHookLaunch>) {
    // Never trust inherited hook credentials. A validated lease is the only injection source.
    for key in HOOK_ENV_KEYS {
        command.env_remove(key);
    }
    if let Some(hook) = hook {
        command.env(HOOK_URL_ENV, hook.endpoint_url());
        command.env(HOOK_AGENT_ID_ENV, hook.agent_id());
        command.env(HOOK_CAPABILITY_ENV, hook.capability());
    }
}

fn spawn_reader(
    stream: impl Read + Send + 'static,
    output: Arc<Mutex<OutputBuffer>>,
    generation: u64,
    pty_id: &str,
) -> Result<(), PtyServiceError> {
    let thread_name = format!("md-{pty_id}-pty");
    let pty_id = String::from(pty_id);
    thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut bytes = [0_u8; 8192];
            let mut decoder = Utf8StreamDecoder::default();
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => {
                        let _ = push_decoded_output(&output, &pty_id, generation, decoder.finish());
                        return;
                    }
                    Ok(read) => {
                        let text = decoder.push(&bytes[..read]);
                        if !push_decoded_output(&output, &pty_id, generation, text) {
                            return;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        let _ = push_decoded_output(&output, &pty_id, generation, decoder.finish());
                        return;
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(PtyServiceError::Io)
}

#[derive(Default)]
struct Utf8StreamDecoder {
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut decoded = String::new();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            let remaining = &self.pending[consumed..];
            match std::str::from_utf8(remaining) {
                Ok(text) => {
                    decoded.push_str(text);
                    consumed = self.pending.len();
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    decoded.push_str(&String::from_utf8_lossy(&remaining[..valid]));
                    consumed = consumed.saturating_add(valid);
                    let Some(invalid_length) = error.error_len() else {
                        break;
                    };
                    decoded.push('\u{fffd}');
                    consumed = consumed.saturating_add(invalid_length);
                }
            }
        }
        if consumed > 0 {
            self.pending.drain(..consumed);
        }
        decoded
    }

    fn finish(self) -> String {
        String::from_utf8_lossy(&self.pending).into_owned()
    }
}

fn push_decoded_output(
    output: &Arc<Mutex<OutputBuffer>>,
    pty_id: &str,
    generation: u64,
    text: String,
) -> bool {
    if text.is_empty() {
        return true;
    }
    if let Ok(mut buffer) = output.lock() {
        buffer.push_output(pty_id, generation, &text);
        true
    } else {
        false
    }
}

const fn pty_size(dimensions: PtyDimensions) -> PtySize {
    PtySize {
        rows: dimensions.rows,
        cols: dimensions.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(unix)]
fn process_group_id(master: &dyn MasterPty) -> Option<i64> {
    master.process_group_leader().map(i64::from)
}

#[cfg(not(unix))]
fn process_group_id(_master: &dyn MasterPty) -> Option<i64> {
    None
}

fn native_io(error: impl Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

#[cfg(unix)]
fn terminate_process_group(process_group_id: Option<i64>) -> Result<(), PtyServiceError> {
    signal_process_group(process_group_id, nix::sys::signal::Signal::SIGHUP)
}

#[cfg(unix)]
fn force_process_group(process_group_id: Option<i64>) -> Result<(), PtyServiceError> {
    signal_process_group(process_group_id, nix::sys::signal::Signal::SIGKILL)
}

#[cfg(unix)]
fn signal_process_group(
    process_group_id: Option<i64>,
    signal: nix::sys::signal::Signal,
) -> Result<(), PtyServiceError> {
    let Some(raw_id) = process_group_id.and_then(|id| i32::try_from(id).ok()) else {
        return Ok(());
    };
    match nix::sys::signal::killpg(nix::unistd::Pid::from_raw(raw_id), signal) {
        Ok(()) | Err(nix::errno::Errno::ESRCH) => Ok(()),
        Err(error) => Err(PtyServiceError::Io(native_io(error))),
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_process_group_id: Option<i64>) -> Result<(), PtyServiceError> {
    Ok(())
}

#[cfg(not(unix))]
fn force_process_group(_process_group_id: Option<i64>) -> Result<(), PtyServiceError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::pty_agents::{
        AgentProvider, AgentRole, PtyDimensions, SpawnAgentRequest,
    };
    use std::ffi::OsStr;

    use portable_pty::CommandBuilder;

    use super::{
        NativePtyBackend, SERVER_ONLY_ENV_KEYS, Utf8StreamDecoder, apply_hook_environment,
        launch_args, pty_size, strip_server_only_environment,
    };
    use crate::domains::pty_agents::hook::{
        AgentHookLaunch, HOOK_AGENT_ID_ENV, HOOK_CAPABILITY_ENV, HOOK_ENV_KEYS, HOOK_URL_ENV,
    };

    fn request(provider: AgentProvider) -> SpawnAgentRequest {
        SpawnAgentRequest {
            id: String::from("dev-1"),
            name: String::from("Dev 1"),
            provider,
            role: AgentRole::default(),
            description: String::new(),
            cwd: String::from("/tmp"),
            command: String::from("agent"),
            args: vec![String::from("--existing")],
            model: Some(String::from("model-1")),
            cols: 80,
            rows: 24,
            isolate: false,
            resume: true,
            require_resume: true,
            resume_session_id: Some(String::from("session-1")),
        }
    }

    #[test]
    fn backend_is_zero_sized() {
        assert_eq!(std::mem::size_of::<NativePtyBackend>(), 0);
    }

    #[test]
    fn pty_dimensions_preserve_row_column_order() {
        let size = pty_size(PtyDimensions {
            cols: 132,
            rows: 43,
        });
        assert_eq!((size.cols, size.rows), (132, 43));
    }

    #[test]
    fn codex_resume_subcommand_precedes_other_arguments() {
        assert!(matches!(
            launch_args(&request(AgentProvider::Codex)),
            Ok((args, true)) if args.starts_with(&[String::from("resume"), String::from("session-1")])
        ));
    }

    #[test]
    fn provider_without_resume_support_fails_before_spawn() {
        assert!(launch_args(&request(AgentProvider::OpenCode)).is_err());
    }

    #[test]
    fn child_command_drops_every_server_only_postgres_variable() {
        let mut command = CommandBuilder::new("agent");
        for key in SERVER_ONLY_ENV_KEYS {
            command.env(key, "must-not-reach-child");
        }

        strip_server_only_environment(&mut command);

        assert!(
            SERVER_ONLY_ENV_KEYS
                .iter()
                .all(|key| command.get_env(*key).is_none())
        );
    }

    #[test]
    fn validated_hook_is_the_only_source_of_child_hook_environment()
    -> Result<(), crate::domains::pty_agents::PtyServiceError> {
        let mut command = CommandBuilder::new("agent");
        for key in HOOK_ENV_KEYS {
            command.env(key, "inherited-must-not-survive");
        }
        let hook = AgentHookLaunch::new(
            "http://127.0.0.1:5001/internal/hive-hook",
            "dev-1",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            std::env::current_dir().map_err(crate::domains::pty_agents::PtyServiceError::Io)?,
        )?;

        apply_hook_environment(&mut command, Some(&hook));

        assert_eq!(
            command.get_env(HOOK_URL_ENV),
            Some(OsStr::new(hook.endpoint_url()))
        );
        assert_eq!(
            command.get_env(HOOK_AGENT_ID_ENV),
            Some(OsStr::new(hook.agent_id()))
        );
        assert_eq!(
            command.get_env(HOOK_CAPABILITY_ENV),
            Some(OsStr::new(hook.capability()))
        );
        Ok(())
    }

    #[test]
    fn absent_hook_removes_all_inherited_hook_environment() {
        let mut command = CommandBuilder::new("agent");
        for key in HOOK_ENV_KEYS {
            command.env(key, "inherited-must-not-survive");
        }

        apply_hook_environment(&mut command, None);

        assert!(
            HOOK_ENV_KEYS
                .iter()
                .all(|key| command.get_env(*key).is_none())
        );
    }

    #[test]
    fn hook_injection_cannot_reintroduce_postgres_secrets()
    -> Result<(), crate::domains::pty_agents::PtyServiceError> {
        let mut command = CommandBuilder::new("agent");
        for key in SERVER_ONLY_ENV_KEYS {
            command.env(key, "db-secret");
        }
        let hook = AgentHookLaunch::new(
            "http://127.0.0.1:5001/internal/hive-hook",
            "dev-1",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            std::env::current_dir().map_err(crate::domains::pty_agents::PtyServiceError::Io)?,
        )?;

        strip_server_only_environment(&mut command);
        apply_hook_environment(&mut command, Some(&hook));

        assert!(
            SERVER_ONLY_ENV_KEYS
                .iter()
                .all(|key| command.get_env(*key).is_none())
        );
        Ok(())
    }

    #[test]
    fn utf8_decoder_carries_split_japanese_code_point_between_reads() {
        let mut decoder = Utf8StreamDecoder::default();
        let bytes = "日本語".as_bytes();

        let first = decoder.push(&bytes[..2]);
        let second = decoder.push(&bytes[2..5]);
        let third = decoder.push(&bytes[5..]);

        assert_eq!(first, "");
        assert_eq!(format!("{second}{third}"), "日本語");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn utf8_decoder_flushes_incomplete_suffix_as_replacement_character() {
        let mut decoder = Utf8StreamDecoder::default();
        assert_eq!(decoder.push(&[0xe6, 0x97]), "");
        assert_eq!(decoder.finish(), "\u{fffd}");
    }

    #[test]
    fn utf8_decoder_recovers_after_invalid_byte_without_losing_japanese_text() {
        let mut decoder = Utf8StreamDecoder::default();
        let mut bytes = vec![0xff];
        bytes.extend_from_slice("正常".as_bytes());

        assert_eq!(decoder.push(&bytes), "\u{fffd}正常");
        assert_eq!(decoder.finish(), "");
    }
}
