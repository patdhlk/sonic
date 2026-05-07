//! `wrap_with_condition` — a helper that gates an [`ExecutableItem`] on a
//! user-supplied predicate.

use crate::context::Context;
use crate::control_flow::{ControlFlow, ExecuteResult};
use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::trigger::TriggerDeclarer;

/// Wraps `item` so that, on each invocation, `cond()` runs first. If `cond`
/// returns `false`, the wrapper returns `Ok(StopChain)` and `item.execute`
/// is **not** called.
///
/// The wrapper forwards `declare_triggers` to the inner item, so triggers
/// are inherited.
pub const fn wrap_with_condition<I, F>(item: I, cond: F) -> Conditional<I, F>
where
    I: ExecutableItem,
    F: FnMut() -> bool + Send + 'static,
{
    Conditional { item, cond }
}

/// Conditional wrapper produced by [`wrap_with_condition`].
pub struct Conditional<I, F> {
    item: I,
    cond: F,
}

impl<I, F> ExecutableItem for Conditional<I, F>
where
    I: ExecutableItem,
    F: FnMut() -> bool + Send + 'static,
{
    fn declare_triggers(&mut self, d: &mut TriggerDeclarer<'_>) -> Result<(), ExecutorError> {
        self.item.declare_triggers(d)
    }

    fn execute(&mut self, ctx: &mut Context<'_>) -> ExecuteResult {
        if (self.cond)() {
            self.item.execute(ctx)
        } else {
            Ok(ControlFlow::StopChain)
        }
    }

    fn task_id(&self) -> Option<&str> {
        self.item.task_id()
    }

    fn app_id(&self) -> Option<u32> {
        self.item.app_id()
    }

    fn app_instance_id(&self) -> Option<u32> {
        self.item.app_instance_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ContextHarness;
    use crate::item::item;
    use crate::ControlFlow;

    #[test]
    fn condition_true_runs_inner() {
        let mut wrapped = wrap_with_condition(item(|_| Ok(ControlFlow::Continue)), || true);
        let h = ContextHarness::new("t");
        let res = wrapped.execute(&mut h.context()).unwrap();
        assert_eq!(res, ControlFlow::Continue);
    }

    #[test]
    fn condition_false_stops_chain() {
        let mut wrapped = wrap_with_condition(item(|_| Ok(ControlFlow::Continue)), || false);
        let h = ContextHarness::new("t");
        let res = wrapped.execute(&mut h.context()).unwrap();
        assert_eq!(res, ControlFlow::StopChain);
    }
}
