//! `ExecutableItem` trait and the closure adapter.

use crate::context::Context;
use crate::control_flow::ExecuteResult;
use crate::error::ExecutorError;
use crate::trigger::TriggerDeclarer;

/// Trait implemented by every unit of work the executor schedules.
///
/// Implementors are moved into the executor and dispatched to pool workers.
/// `Send + 'static` is required; `Sync` is **not** — the executor guarantees
/// at most one thread at a time invokes `execute` on a given item.
pub trait ExecutableItem: Send + 'static {
    /// Called once when the item is added to an executor. The implementor
    /// registers its trigger handles via the [`TriggerDeclarer`].
    fn declare_triggers(
        &mut self,
        d: &mut TriggerDeclarer<'_>,
    ) -> Result<(), ExecutorError> {
        let _ = d;
        Ok(())
    }

    /// Called by the executor when any declared trigger fires.
    fn execute(&mut self, ctx: &mut Context<'_>) -> ExecuteResult;

    /// Optional human-readable id used for monitor/observer correlation.
    /// `None` means "use the auto-assigned id".
    fn task_id(&self) -> Option<&str> {
        None
    }

    /// Optional application id; `Some(_)` enables Observer per-app callbacks.
    fn app_id(&self) -> Option<u32> {
        None
    }

    /// Optional application instance id.
    fn app_instance_id(&self) -> Option<u32> {
        None
    }
}

// ── Closure adapter ────────────────────────────────────────────────────────

/// Adapter turning a closure into an [`ExecutableItem`] with no triggers
/// declared. Use [`item_with_triggers`] when triggers are needed.
pub struct FnItem<F>(F);

impl<F> ExecutableItem for FnItem<F>
where
    F: FnMut(&mut Context<'_>) -> ExecuteResult + Send + 'static,
{
    fn execute(&mut self, ctx: &mut Context<'_>) -> ExecuteResult {
        (self.0)(ctx)
    }
}

/// Wrap a closure as an [`ExecutableItem`].
pub const fn item<F>(f: F) -> FnItem<F>
where
    F: FnMut(&mut Context<'_>) -> ExecuteResult + Send + 'static,
{
    FnItem(f)
}

/// Wrap a pair of closures (`declare`, `execute`) as an [`ExecutableItem`].
pub struct FnItemWithTriggers<D, E> {
    /// `Some` until the first `declare_triggers` call; `None` thereafter.
    /// `Option::take` enforces the once-only guarantee — do not unwrap directly.
    declare: Option<D>,
    /// User-supplied execute closure invoked on every dispatch.
    execute: E,
}

impl<D, E> ExecutableItem for FnItemWithTriggers<D, E>
where
    D: FnOnce(&mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> + Send + 'static,
    E: FnMut(&mut Context<'_>) -> ExecuteResult + Send + 'static,
{
    fn declare_triggers(
        &mut self,
        d: &mut TriggerDeclarer<'_>,
    ) -> Result<(), ExecutorError> {
        self.declare.take().map_or_else(|| Ok(()), |decl| decl(d))
    }

    fn execute(&mut self, ctx: &mut Context<'_>) -> ExecuteResult {
        (self.execute)(ctx)
    }
}

/// Wrap a `(declare, execute)` pair as an [`ExecutableItem`].
pub const fn item_with_triggers<D, E>(declare: D, execute: E) -> FnItemWithTriggers<D, E>
where
    D: FnOnce(&mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> + Send + 'static,
    E: FnMut(&mut Context<'_>) -> ExecuteResult + Send + 'static,
{
    FnItemWithTriggers {
        declare: Some(declare),
        execute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextHarness;
    use crate::control_flow::ControlFlow;

    #[test]
    fn closure_item_runs() {
        let mut counter = 0_u32;
        let cell = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cell_clone = std::sync::Arc::clone(&cell);

        let mut it = item(move |_ctx| {
            cell_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ControlFlow::Continue)
        });

        let harness = ContextHarness::new("test-task");
        for _ in 0..3 {
            it.execute(&mut harness.context()).unwrap();
            counter += 1;
        }

        assert_eq!(counter, 3);
        assert_eq!(cell.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[test]
    fn item_with_triggers_calls_declare_once() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let calls_d = std::sync::Arc::clone(&calls);

        let mut it = item_with_triggers(
            move |_d| {
                calls_d.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
            |_ctx| Ok(ControlFlow::Continue),
        );

        let mut declarer_storage = TriggerDeclarer::new_test();
        it.declare_triggers(&mut declarer_storage).unwrap();
        it.declare_triggers(&mut declarer_storage).unwrap();
        it.declare_triggers(&mut declarer_storage).unwrap();

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1,
                   "declare closure must be invoked at most once");
    }
}
