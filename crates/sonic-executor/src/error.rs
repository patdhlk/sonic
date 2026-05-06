//! Error types surfaced by the executor.

use crate::task_id::TaskId;

/// Type alias for user-supplied item errors. Boxed `dyn Error` so callers can
/// plug in any error type without forcing this crate to know about it.
pub type ItemError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Top-level error type for the executor.
#[derive(thiserror::Error, Debug)]
pub enum ExecutorError {
    /// An iceoryx2 operation failed. The original error is rendered with
    /// `{}` because iceoryx2's error types do not collapse into a single
    /// `From` source.
    #[error("iceoryx2: {0}")]
    Iceoryx2(String),

    /// Graph validation failed at `build()` time.
    #[error("invalid graph: {0}")]
    InvalidGraph(String),

    /// An item's `declare_triggers` call returned an error or the executor
    /// rejected it (e.g. a duplicate subscriber attachment).
    #[error("trigger declaration failed: {0}")]
    DeclareTriggers(String),

    /// An item returned `Err(...)` or panicked. The original error is wrapped.
    #[error("item error in task {task_id}: {source}")]
    Item {
        /// The task that produced the error.
        task_id: TaskId,
        /// The underlying error from the item.
        #[source]
        source: ItemError,
    },

    /// `Executor::run` was called while the executor was already running.
    #[error("executor already running")]
    AlreadyRunning,

    /// The runner thread panicked or could not be joined.
    #[error("runner thread join failed")]
    RunnerJoin,

    /// Builder API used incorrectly (e.g. missing required field).
    #[error("builder misuse: {0}")]
    Builder(String),
}

impl ExecutorError {
    /// Convenience constructor for wrapping arbitrary iceoryx2 error values.
    #[must_use]
    pub fn iceoryx2(err: impl core::fmt::Display) -> Self {
        Self::Iceoryx2(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_error_roundtrip() {
        let source: ItemError = Box::new(std::io::Error::other("boom"));
        let err = ExecutorError::Item {
            task_id: "task-1".into(),
            source,
        };
        let s = format!("{err}");
        assert!(s.contains("task-1"));
        assert!(s.contains("boom"));
    }

    #[test]
    fn iceoryx2_helper_renders_display() {
        #[derive(Debug, thiserror::Error)]
        #[error("whatever happened")]
        struct Whatever;
        let e = ExecutorError::iceoryx2(Whatever);
        assert!(matches!(e, ExecutorError::Iceoryx2(_)));
        assert!(format!("{e}").contains("whatever happened"));
    }
}
