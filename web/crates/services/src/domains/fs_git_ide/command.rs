use std::io::{Read, Result as IoResult};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::DomainError;

const MAX_COMMAND_OUTPUT: u64 = 4 * 1024 * 1024;
const SERVER_PRIVATE_ENV: [&str; 7] = [
    "MD_PG_PASSWORD",
    "MD_PG_HOST",
    "MD_PG_PORT",
    "MD_PG_DATABASE",
    "MD_PG_USER",
    "MD_PG_NAMESPACE",
    "MD_PG_TLS_CA",
];

pub(crate) struct CommandOutput {
    pub stdout: String,
}

pub(crate) fn run_command(
    program: &str,
    cwd: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<CommandOutput, DomainError> {
    let mut child = prepared_command(program, cwd, args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| DomainError::CommandUnavailable)?;
    let stdout = child.stdout.take().ok_or(DomainError::CommandFailed)?;
    let stderr = child.stderr.take().ok_or(DomainError::CommandFailed)?;
    let stdout_reader = thread::spawn(move || read_output(stdout));
    let stderr_reader = thread::spawn(move || read_output(stderr));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|_| DomainError::CommandFailed)? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(DomainError::CommandTimedOut);
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };
    let stdout = join_output(stdout_reader)?;
    let _stderr = join_output(stderr_reader)?;
    if !status.success() {
        return Err(DomainError::CommandFailed);
    }
    Ok(CommandOutput {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
    })
}

fn prepared_command(program: &str, cwd: &Path, args: &[String]) -> Command {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    for key in SERVER_PRIVATE_ENV {
        command.env_remove(key);
    }
    command
}

fn read_output(reader: impl Read) -> IoResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_COMMAND_OUTPUT + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join_output(reader: thread::JoinHandle<IoResult<Vec<u8>>>) -> Result<Vec<u8>, DomainError> {
    let bytes = reader
        .join()
        .map_err(|_| DomainError::CommandFailed)?
        .map_err(|_| DomainError::CommandFailed)?;
    if u64::try_from(bytes.len()).map_err(|_| DomainError::OutputTooLarge)? > MAX_COMMAND_OUTPUT {
        return Err(DomainError::OutputTooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use super::{SERVER_PRIVATE_ENV, prepared_command, run_command};

    #[test]
    fn child_command_removes_postgres_private_environment_without_global_mutation() {
        let command = prepared_command("git", &std::env::temp_dir(), &[]);
        let overrides = command
            .get_envs()
            .map(|(key, value)| (key.to_string_lossy().into_owned(), value.is_none()))
            .collect::<BTreeMap<_, _>>();

        for key in SERVER_PRIVATE_ENV {
            assert_eq!(overrides.get(key), Some(&true));
        }
    }

    #[test]
    fn missing_command_is_reported() {
        let result = run_command(
            "munder-command-that-does-not-exist",
            &std::env::temp_dir(),
            &[],
            Duration::from_millis(100),
        );
        assert!(result.is_err());
    }
}
