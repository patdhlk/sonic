//! Hosts an [`Executor`] on a dedicated OS thread.

use crate::context::Stoppable;
use crate::error::ExecutorError;
use crate::executor::Executor;
use bitflags::bitflags;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};

bitflags! {
    /// Behaviour flags for [`Runner`].
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct RunnerFlags: u32 {
        /// Don't auto-start; require an explicit `start()` call.
        const DEFERRED        = 1 << 0;
        /// Reserved — actual signal-on-error wiring lands in Task 22.
        const SIGNAL_ON_ERROR = 1 << 1;
    }
}

/// Hosts an [`Executor`] on a dedicated thread.
pub struct Runner {
    handle: Option<JoinHandle<Result<(), ExecutorError>>>,
    stop: Stoppable,
    deferred_start: Option<crossbeam_channel::Sender<()>>,
    /// Retains the last error emitted by the run loop so that `stop()` can
    /// re-throw it. The Arc is cloned into the thread closure; the field here
    /// keeps the allocation alive for the lifetime of the `Runner`.
    #[allow(dead_code)]
    captured_error: Arc<Mutex<Option<ExecutorError>>>,
    flags: RunnerFlags,
}

impl Runner {
    /// Spawn a runner thread; returns immediately. If `flags.DEFERRED` is set
    /// the run loop blocks until [`Runner::start`] is invoked.
    ///
    /// # Panics
    ///
    /// Panics if the internal error-capture mutex is poisoned, which can only
    /// happen if a previous holder panicked while holding it — an event that
    /// is not expected under normal use.
    #[allow(clippy::missing_panics_doc)] // unwrap on Mutex::lock; poisoning is unreachable in practice
    #[track_caller]
    pub fn new(exec: Executor, flags: RunnerFlags) -> Result<Self, ExecutorError> {
        let stop = exec.stoppable();
        let captured_error = Arc::new(Mutex::new(None::<ExecutorError>));
        let captured_clone = Arc::clone(&captured_error);
        let mut exec = exec;

        let (start_tx, start_rx) = crossbeam_channel::bounded::<()>(1);
        let deferred = flags.contains(RunnerFlags::DEFERRED);

        let handle = thread::Builder::new()
            .name("sonic-runner".to_owned())
            .spawn(move || -> Result<(), ExecutorError> {
                if deferred {
                    let _ = start_rx.recv();
                }
                let res = exec.run();
                if let Err(e) = &res {
                    *captured_clone.lock().unwrap() = Some(clone_executor_error(e));
                }
                res
            })
            .map_err(|e| ExecutorError::Builder(format!("spawn runner: {e}")))?;

        Ok(Self {
            handle: Some(handle),
            stop,
            deferred_start: if deferred { Some(start_tx) } else { None },
            captured_error,
            flags,
        })
    }

    /// Resume a deferred runner. No-op if the runner was not deferred.
    pub fn start(&mut self) -> Result<(), ExecutorError> {
        if let Some(tx) = self.deferred_start.take() {
            tx.send(()).map_err(|_| ExecutorError::RunnerJoin)?;
        }
        Ok(())
    }

    /// Stop the runner and re-throw any captured item error.
    pub fn stop(&mut self) -> Result<(), ExecutorError> {
        if self.deferred_start.is_some() {
            // Releasing the deferred wait so the thread can run + observe stop.
            let _ = self.start();
        }
        self.stop.stop();
        self.handle.take().map_or_else(
            || Ok(()),
            |handle| match handle.join() {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(ExecutorError::RunnerJoin),
            },
        )
    }

    /// Get a [`Stoppable`] for sharing into other threads.
    pub fn stoppable(&self) -> Stoppable {
        self.stop.clone()
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        // Best-effort: stop and surface error via tracing or stderr; never panic.
        match self.stop() {
            Ok(()) => {}
            Err(e) => {
                #[cfg(feature = "tracing")]
                tracing::error!(target: "sonic-executor", error = %e, "Runner dropped with error");
                #[cfg(not(feature = "tracing"))]
                eprintln!("[sonic-executor] runner dropped with error: {e}");
                let _ = self.flags;
            }
        }
    }
}

fn clone_executor_error(e: &ExecutorError) -> ExecutorError {
    // Errors don't impl Clone (ItemError is dyn Error); we synthesise an
    // equivalent description.
    match e {
        ExecutorError::Iceoryx2(s) => ExecutorError::Iceoryx2(s.clone()),
        ExecutorError::InvalidGraph(s) => ExecutorError::InvalidGraph(s.clone()),
        ExecutorError::DeclareTriggers(s) => ExecutorError::DeclareTriggers(s.clone()),
        ExecutorError::Item { task_id, source } => ExecutorError::Item {
            task_id: task_id.clone(),
            source: Box::new(StringError(source.to_string())),
        },
        ExecutorError::AlreadyRunning => ExecutorError::AlreadyRunning,
        ExecutorError::RunnerJoin => ExecutorError::RunnerJoin,
        ExecutorError::Builder(s) => ExecutorError::Builder(s.clone()),
    }
}

#[derive(Debug)]
struct StringError(String);
impl core::fmt::Display for StringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for StringError {}
