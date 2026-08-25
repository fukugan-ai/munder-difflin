use std::collections::BTreeMap;
use std::sync::Mutex;

use md_web_contracts::domains::hive_tasks::WorkerTeardownReceipt;
use md_web_contracts::{PreservedWorktreeSnapshot, WorkerSnapshot, WorkerStatus};

/// Failure to access worker lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerRegistryError {
    InvalidLimit,
    DuplicateWorker,
    WorkerNotFound,
    LockPoisoned,
}

#[derive(Default)]
struct WorkerState {
    live: BTreeMap<String, WorkerSnapshot>,
    preserved: BTreeMap<String, PreservedWorktreeSnapshot>,
}

/// Process-local projection of live and preserved ephemeral workers.
pub struct WorkerRegistry {
    max_workers: usize,
    state: Mutex<WorkerState>,
}

impl WorkerRegistry {
    /// Creates a registry with a positive concurrency limit.
    pub fn new(max_workers: usize) -> Result<Self, WorkerRegistryError> {
        if max_workers == 0 {
            return Err(WorkerRegistryError::InvalidLimit);
        }
        Ok(Self {
            max_workers,
            state: Mutex::new(WorkerState::default()),
        })
    }

    /// Registers a newly spawned worker.
    pub fn insert(&self, worker: WorkerSnapshot) -> Result<(), WorkerRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkerRegistryError::LockPoisoned)?;
        if state.live.contains_key(&worker.worker_id) {
            return Err(WorkerRegistryError::DuplicateWorker);
        }
        state.live.insert(worker.worker_id.clone(), worker);
        Ok(())
    }

    /// Marks a live worker as releasing. The PTY domain performs actual teardown.
    pub fn request_stop(&self, worker_id: &str) -> Result<WorkerSnapshot, WorkerRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkerRegistryError::LockPoisoned)?;
        let worker = state
            .live
            .get_mut(worker_id)
            .ok_or(WorkerRegistryError::WorkerNotFound)?;
        worker.status = WorkerStatus::Releasing;
        Ok(worker.clone())
    }

    /// Returns stable snapshots for the Workers view.
    pub fn snapshot(
        &self,
    ) -> Result<(Vec<WorkerSnapshot>, Vec<PreservedWorktreeSnapshot>, usize), WorkerRegistryError>
    {
        let state = self
            .state
            .lock()
            .map_err(|_| WorkerRegistryError::LockPoisoned)?;
        Ok((
            state.live.values().cloned().collect(),
            state.preserved.values().cloned().collect(),
            self.max_workers,
        ))
    }

    /// Removes a stopped worker and records any server-resolved worktree for integration.
    pub fn complete_stop(
        &self,
        worker_id: &str,
        worktree_path: Option<String>,
        completed_at: i64,
    ) -> Result<WorkerTeardownReceipt, WorkerRegistryError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkerRegistryError::LockPoisoned)?;
        let worker = state
            .live
            .remove(worker_id)
            .ok_or(WorkerRegistryError::WorkerNotFound)?;
        let preserved_path = worktree_path.filter(|path| !path.trim().is_empty());
        if let Some(path) = &preserved_path {
            state.preserved.insert(
                String::from(worker_id),
                PreservedWorktreeSnapshot {
                    worker_id: String::from(worker_id),
                    worktree_path: path.clone(),
                    base_branch: worker.base_branch,
                    preserved_at: completed_at,
                },
            );
        }
        Ok(WorkerTeardownReceipt {
            worker_id: String::from(worker_id),
            pty_stopped: true,
            worktree_preserved: preserved_path.is_some(),
            preserved_path,
            completed_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::{WorkerSnapshot, WorkerStatus};

    use super::{WorkerRegistry, WorkerRegistryError};

    fn worker() -> WorkerSnapshot {
        WorkerSnapshot {
            worker_id: String::from("worker-1"),
            request_id: String::from("1"),
            name: String::from("Worker 1"),
            base_branch: String::from("main"),
            spawned_at: 0,
            age_ms: 0,
            idle_ms: None,
            tokens_used: 0,
            token_cap: None,
            has_slack: false,
            status: WorkerStatus::Working,
        }
    }

    #[test]
    fn rejects_zero_limit() {
        assert!(matches!(
            WorkerRegistry::new(0),
            Err(WorkerRegistryError::InvalidLimit)
        ));
    }

    #[test]
    fn duplicate_worker_is_rejected() -> Result<(), WorkerRegistryError> {
        let registry = WorkerRegistry::new(1)?;
        registry.insert(worker())?;

        assert!(matches!(
            registry.insert(worker()),
            Err(WorkerRegistryError::DuplicateWorker)
        ));
        Ok(())
    }

    #[test]
    fn stop_marks_worker_releasing() -> Result<(), WorkerRegistryError> {
        let registry = WorkerRegistry::new(1)?;
        registry.insert(worker())?;

        assert_eq!(
            registry.request_stop("worker-1")?.status,
            WorkerStatus::Releasing
        );
        Ok(())
    }

    #[test]
    fn snapshot_returns_configured_limit() -> Result<(), WorkerRegistryError> {
        let registry = WorkerRegistry::new(4)?;

        assert_eq!(registry.snapshot()?.2, 4);
        Ok(())
    }

    #[test]
    fn completed_stop_removes_live_worker_and_preserves_worktree() -> Result<(), WorkerRegistryError>
    {
        let registry = WorkerRegistry::new(1)?;
        registry.insert(worker())?;
        registry.request_stop("worker-1")?;

        let receipt =
            registry.complete_stop("worker-1", Some(String::from("/worktrees/worker-1")), 9)?;

        assert!(receipt.pty_stopped);
        assert!(receipt.worktree_preserved);
        assert!(registry.snapshot()?.0.is_empty());
        assert_eq!(registry.snapshot()?.1.len(), 1);
        Ok(())
    }
}
