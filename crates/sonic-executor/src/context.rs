//! Per-invocation context handed to [`ExecutableItem::execute`].

use crate::task_id::TaskId;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Shared stop flag passed via [`Context::stoppable`].
///
/// Cloneable, thread-safe. Setting it asks the executor to terminate the
/// run loop after the current iteration completes.
#[derive(Clone, Debug, Default)]
pub struct Stoppable(Arc<AtomicBool>);

impl Stoppable {
    /// Create a fresh, un-stopped handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request stop.
    pub fn stop(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Check whether stop has been requested.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Per-invocation context. Borrowed view; not stored across calls.
pub struct Context<'a> {
    task_id: &'a TaskId,
    stop: &'a Stoppable,
    // Future: observer hook lands here in Task 19. Keep the struct opaque so
    // we can grow it without breaking ExecutableItem implementors.
    _private: (),
}

impl<'a> Context<'a> {
    /// Internal constructor used by the executor and the test harness.
    #[doc(hidden)]
    pub const fn new(task_id: &'a TaskId, stop: &'a Stoppable) -> Self {
        Self { task_id, stop, _private: () }
    }

    /// Identifier of the task currently executing.
    pub const fn task_id(&self) -> &TaskId {
        self.task_id
    }

    /// Request the enclosing executor to stop.
    pub fn stop_executor(&self) {
        self.stop.stop();
    }

    /// Get a clonable [`Stoppable`] handle that other threads may hold.
    pub fn stoppable(&self) -> Stoppable {
        self.stop.clone()
    }
}

/// Test-only harness for constructing a `Context` outside an executor.
#[cfg(test)]
pub struct ContextHarness {
    task_id: TaskId,
    stop: Stoppable,
}

#[cfg(test)]
impl ContextHarness {
    pub(crate) fn new(id: impl Into<TaskId>) -> Self {
        Self {
            task_id: id.into(),
            stop: Stoppable::new(),
        }
    }

    pub(crate) fn context(&self) -> Context<'_> {
        Context::new(&self.task_id, &self.stop)
    }
}
