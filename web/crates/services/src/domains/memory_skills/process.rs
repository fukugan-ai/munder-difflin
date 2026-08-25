use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use super::DomainError;

#[derive(Clone)]
pub struct ProcessControl {
    state: Arc<ProcessState>,
    timeout: Duration,
}

struct ProcessState {
    cancelled: AtomicBool,
    active: Mutex<usize>,
    drained: Condvar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessDrainStatus {
    pub active_before: usize,
    pub active_after: usize,
    pub drained: bool,
}

impl ProcessControl {
    pub fn new(timeout: Duration) -> Self {
        Self {
            state: Arc::new(ProcessState {
                cancelled: AtomicBool::new(false),
                active: Mutex::new(0),
                drained: Condvar::new(),
            }),
            timeout,
        }
    }

    pub fn cancel(&self) {
        if let Ok(_gate) = self.state.active.lock() {
            self.state.cancelled.store(true, Ordering::Release);
        }
    }

    pub fn cancel_and_wait(&self, timeout: Duration) -> ProcessDrainStatus {
        let Ok(mut active) = self.state.active.lock() else {
            self.state.cancelled.store(true, Ordering::Release);
            return ProcessDrainStatus {
                active_before: 0,
                active_after: 0,
                drained: false,
            };
        };
        self.state.cancelled.store(true, Ordering::Release);
        let active_before = *active;
        let deadline = Instant::now() + timeout;
        while *active > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let Ok((next, wait)) = self.state.drained.wait_timeout(active, remaining) else {
                return ProcessDrainStatus {
                    active_before,
                    active_after: 0,
                    drained: false,
                };
            };
            active = next;
            if wait.timed_out() {
                break;
            }
        }
        ProcessDrainStatus {
            active_before,
            active_after: *active,
            drained: *active == 0,
        }
    }

    pub fn run(&self, command: &mut Command) -> Result<Output, DomainError> {
        let _active = ActiveProcess::begin(&self.state)?;
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_reader = std::thread::spawn(move || read_pipe(stdout));
        let stderr_reader = std::thread::spawn(move || read_pipe(stderr));
        let started = Instant::now();
        let status = loop {
            if self.state.cancelled.load(Ordering::Acquire) || started.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(DomainError::Unavailable(
                    "process execution timed out or was cancelled",
                ));
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        let stdout = stdout_reader.join().unwrap_or_default();
        let stderr = stderr_reader.join().unwrap_or_default();
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

struct ActiveProcess {
    state: Arc<ProcessState>,
}

impl ActiveProcess {
    fn begin(state: &Arc<ProcessState>) -> Result<Self, DomainError> {
        let mut active = state
            .active
            .lock()
            .map_err(|_| DomainError::Unavailable("process control is unavailable"))?;
        if state.cancelled.load(Ordering::Acquire) {
            return Err(DomainError::Unavailable(
                "process execution is shutting down",
            ));
        }
        *active = active.saturating_add(1);
        Ok(Self {
            state: Arc::clone(state),
        })
    }
}

impl Drop for ActiveProcess {
    fn drop(&mut self) {
        if let Ok(mut active) = self.state.active.lock() {
            *active = active.saturating_sub(1);
            if *active == 0 {
                self.state.drained.notify_all();
            }
        }
    }
}

impl Default for ProcessControl {
    fn default() -> Self {
        Self::new(Duration::from_secs(120))
    }
}

fn read_pipe(pipe: Option<impl Read>) -> Vec<u8> {
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe {
        let _ = pipe.take(8 * 1024 * 1024).read_to_end(&mut bytes);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::ProcessControl;

    #[test]
    fn cancelled_control_refuses_to_start_processes() {
        let control = ProcessControl::default();
        control.cancel();
        assert!(
            control
                .run(&mut std::process::Command::new("never-started"))
                .is_err()
        );
    }

    #[test]
    fn cancelled_idle_control_drains_immediately() {
        let control = ProcessControl::default();
        let receipt = control.cancel_and_wait(Duration::from_millis(10));

        assert_eq!(receipt.active_before, 0);
        assert!(receipt.drained);
    }
}
