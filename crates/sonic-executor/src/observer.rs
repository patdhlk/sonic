//! `Observer` trait — lifecycle hooks invoked by the executor.

use crate::error::ExecutorError;
use crate::task_id::TaskId;

/// Generic user event carried by [`Observer::on_send_event`].
#[derive(Clone, Debug, Default)]
pub struct UserEvent {
    /// User-defined event kind.
    pub kind: u32,
    /// Numeric payload.
    pub int_data: i64,
    /// Optional string payload.
    pub string_data: Option<String>,
}

/// Lifecycle observer invoked by the executor at well-defined points.
///
/// All methods have no-op defaults. The executor never blocks on observer
/// callbacks — heavy work should be queued internally.
pub trait Observer: Send + Sync {
    /// Called once just before the dispatch loop begins.
    fn on_executor_up(&self) {}
    /// Called once just after the dispatch loop finishes cleanly.
    fn on_executor_down(&self) {}
    /// Called when the dispatch loop returns an error.
    fn on_executor_error(&self, _e: &ExecutorError) {}

    /// Called before an item with `app_id().is_some()` runs (per invocation).
    fn on_app_start(&self, _task: TaskId, _app: u32, _instance: Option<u32>) {}
    /// Called after such an item runs.
    fn on_app_stop(&self, _task: TaskId) {}
    /// Called when an item returns `Err` or panics.
    fn on_app_error(&self, _task: TaskId, _e: &(dyn std::error::Error + 'static)) {}

    /// Called when an item invokes `Context::send_event`.
    fn on_send_event(&self, _task: TaskId, _ev: UserEvent) {}
}

/// No-op observer used when the user does not configure one.
pub struct NoopObserver;
impl Observer for NoopObserver {}
