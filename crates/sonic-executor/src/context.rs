//! Per-invocation context handed to [`ExecutableItem::execute`].

use crate::observer::{Observer, UserEvent};
use crate::task_id::TaskId;
use iceoryx2::port::notifier::Notifier as IxNotifier;
use iceoryx2::prelude::ipc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared stop flag passed via [`Context::stoppable`].
///
/// Cloneable, thread-safe. Setting it asks the executor to terminate the
/// run loop after the current iteration completes — and, if the handle
/// was bound to a running executor, also wakes the `WaitSet` immediately.
#[derive(Clone)]
pub struct Stoppable {
    flag: Arc<AtomicBool>,
    waker: Option<Arc<IxNotifier<ipc::Service>>>,
}

// SAFETY: `IxNotifier<ipc::Service>` is `!Send` only because `ipc::Service`
// uses `SingleThreaded` (an `Rc`-backed arc policy) which is mutated only at
// port-construction time.  After the notifier is created and wrapped in `Arc`,
// the only operation we perform on it from any thread is `notifier.notify()`,
// which does not touch the `Rc` refcount — it writes into a lock-free shared
// memory ring.  We never expose a `&mut Notifier` across thread boundaries and
// we do not implement `Sync` (Arc<Stoppable> is only Clone, not Deref-to-mut),
// so concurrent mutation of the Rc is impossible.  Moving the Arc across
// threads is therefore sound.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl Send for Stoppable {}

impl Default for Stoppable {
    fn default() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            waker: None,
        }
    }
}

impl Stoppable {
    /// Create a fresh, un-stopped handle with no wakeup wired.
    /// Useful for tests; the executor uses `with_waker` to bind a notifier.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Internal constructor — the executor injects a notifier so `stop()`
    /// wakes the WaitSet thread.
    #[doc(hidden)]
    pub(crate) fn with_waker(waker: Arc<IxNotifier<ipc::Service>>) -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            waker: Some(waker),
        }
    }

    /// Request stop. Flips the flag (Release) and, if a waker was bound,
    /// notifies the `WaitSet` so it returns from `wait_and_process` promptly.
    pub fn stop(&self) {
        self.flag.store(true, Ordering::Release);
        if let Some(w) = &self.waker {
            let _ = w.notify();
        }
    }

    /// Check whether stop has been requested.
    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

impl core::fmt::Debug for Stoppable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stoppable")
            .field("flag", &self.is_stopped())
            .field("waker", &self.waker.is_some())
            .finish()
    }
}

/// Per-invocation context. Borrowed view; not stored across calls.
#[non_exhaustive]
pub struct Context<'a> {
    task_id: &'a TaskId,
    stop: &'a Stoppable,
    observer: &'a dyn Observer,
}

impl<'a> Context<'a> {
    /// Internal constructor used by the executor and the test harness.
    #[doc(hidden)]
    pub fn new(task_id: &'a TaskId, stop: &'a Stoppable, observer: &'a dyn Observer) -> Self {
        Self {
            task_id,
            stop,
            observer,
        }
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

    /// Forward a user event to the observer (no-op if no observer is configured).
    pub fn send_event(&self, ev: UserEvent) {
        self.observer.on_send_event(self.task_id.clone(), ev);
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
        static NOOP: crate::observer::NoopObserver = crate::observer::NoopObserver;
        Context::new(&self.task_id, &self.stop, &NOOP)
    }
}
