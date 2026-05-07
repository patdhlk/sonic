//! `tracing`-based [`Observer`](sonic_executor::Observer).
//!
//! Pass `Arc::new(TracingObserver::default())` to
//! [`ExecutorBuilder::observer`](sonic_executor::ExecutorBuilder)
//! to forward all executor lifecycle events to the global `tracing`
//! subscriber.

#![doc(html_root_url = "https://docs.rs/sonic-executor-tracing/0.1.0")]

use sonic_executor::{ExecutorError, Observer, TaskId, UserEvent};

/// Observer that forwards every callback to the global `tracing` subscriber.
#[derive(Debug, Default)]
pub struct TracingObserver;

impl Observer for TracingObserver {
    fn on_executor_up(&self) {
        tracing::info!(target: "sonic.executor", "executor.up");
    }
    fn on_executor_down(&self) {
        tracing::info!(target: "sonic.executor", "executor.down");
    }
    fn on_executor_error(&self, e: &ExecutorError) {
        tracing::error!(target: "sonic.executor", error = %e, "executor.error");
    }

    fn on_app_start(&self, task: TaskId, app: u32, instance: Option<u32>) {
        tracing::info!(
            target: "sonic.app",
            task = %task,
            app,
            ?instance,
            "app.start"
        );
    }
    fn on_app_stop(&self, task: TaskId) {
        tracing::info!(target: "sonic.app", task = %task, "app.stop");
    }
    fn on_app_error(&self, task: TaskId, e: &(dyn std::error::Error + 'static)) {
        tracing::error!(
            target: "sonic.app",
            task = %task,
            error = %e,
            "app.error"
        );
    }

    fn on_send_event(&self, task: TaskId, ev: UserEvent) {
        tracing::event!(
            target: "sonic.user",
            tracing::Level::INFO,
            task = %task,
            kind = ev.kind,
            int_data = ev.int_data,
            string_data = ev.string_data.as_deref().unwrap_or(""),
            "user.event"
        );
    }
}
