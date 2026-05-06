//! `Executor` and `ExecutorBuilder`. Run loop lives in Task 8.

// Fields consumed by the run loop (Task 8) and graph scheduler (Task 14).
#![allow(dead_code)]
// pub(crate) inside a private module — intentional, Task 8+ will use them.
#![allow(clippy::redundant_pub_crate)]

use crate::context::Stoppable;
use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::pool::Pool;
use crate::task_id::TaskId;
use crate::trigger::{TriggerDecl, TriggerDeclarer};
use crate::Channel;
use iceoryx2::node::Node;
use iceoryx2::prelude::ipc;
use iceoryx2::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// One registered task entry.
pub(crate) struct TaskEntry {
    /// Task identifier.
    pub(crate) id: TaskId,
    /// The executable item itself.
    pub(crate) item: Box<dyn ExecutableItem>,
    /// Trigger declarations recorded at `add` time.
    pub(crate) decls: Vec<TriggerDecl>,
}

/// Top-level executor. One per process is the typical case.
pub struct Executor {
    pub(crate) node: Node<ipc::Service>,
    pub(crate) pool: Arc<Pool>,
    pub(crate) tasks: Vec<TaskEntry>,
    pub(crate) running: Arc<AtomicBool>,
    pub(crate) stoppable: Stoppable,
    pub(crate) next_id: AtomicU64,
}

impl Executor {
    /// Start a new builder.
    #[must_use]
    pub fn builder() -> ExecutorBuilder {
        ExecutorBuilder::default()
    }

    /// Open or create a pub/sub channel bound to this executor's node.
    pub fn channel<T: ZeroCopySend + Default + core::fmt::Debug + 'static>(
        &mut self,
        name: &str,
    ) -> Result<Arc<Channel<T>>, ExecutorError> {
        Channel::open_or_create(&self.node, name)
    }

    /// Add an item to the executor with an auto-generated id.
    pub fn add(
        &mut self,
        item: impl ExecutableItem,
    ) -> Result<TaskId, ExecutorError> {
        let id = TaskId::new(format!("task-{}", self.next_id.fetch_add(1, Ordering::SeqCst)));
        self.add_with_id(id, item)
    }

    /// Add an item with a user-supplied id.
    pub fn add_with_id(
        &mut self,
        id: impl Into<TaskId>,
        mut item: impl ExecutableItem,
    ) -> Result<TaskId, ExecutorError> {
        let id: TaskId = id.into();
        let mut declarer = TriggerDeclarer::new_internal();
        item.declare_triggers(&mut declarer)?;
        let decls = declarer.into_decls();

        self.tasks.push(TaskEntry {
            id: id.clone(),
            item: Box::new(item),
            decls,
        });
        Ok(id)
    }

    /// Returns a [`Stoppable`] handle; clone before calling `run()`.
    #[must_use]
    pub fn stoppable(&self) -> Stoppable {
        self.stoppable.clone()
    }

    /// Borrow the underlying iceoryx2 node (escape hatch for power users).
    pub const fn iceoryx_node(&self) -> &Node<ipc::Service> {
        &self.node
    }
}

/// Builder for [`Executor`].
#[derive(Default)]
pub struct ExecutorBuilder {
    worker_threads: Option<usize>,
}

impl ExecutorBuilder {
    /// Number of worker threads. `0` → inline (no pool). Default → physical
    /// cores.
    #[must_use]
    pub const fn worker_threads(mut self, n: usize) -> Self {
        self.worker_threads = Some(n);
        self
    }

    /// Build the [`Executor`]. Creates a fresh iceoryx2 node.
    pub fn build(self) -> Result<Executor, ExecutorError> {
        let node = NodeBuilder::new()
            .create::<ipc::Service>()
            .map_err(ExecutorError::iceoryx2)?;

        let n_workers = self
            .worker_threads
            .unwrap_or_else(num_cpus::get_physical);
        let pool = Arc::new(Pool::new(n_workers)?);

        Ok(Executor {
            node,
            pool,
            tasks: Vec::new(),
            running: Arc::new(AtomicBool::new(false)),
            stoppable: Stoppable::new(),
            next_id: AtomicU64::new(0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{item, ControlFlow};

    #[test]
    fn add_returns_unique_ids() {
        let mut exec = Executor::builder().worker_threads(0).build().unwrap();
        let a = exec.add(item(|_| Ok(ControlFlow::Continue))).unwrap();
        let b = exec.add(item(|_| Ok(ControlFlow::Continue))).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn custom_id_is_preserved() {
        let mut exec = Executor::builder().worker_threads(0).build().unwrap();
        let id = exec
            .add_with_id("my-task", item(|_| Ok(ControlFlow::Continue)))
            .unwrap();
        assert_eq!(id.as_str(), "my-task");
    }

    #[test]
    fn declare_triggers_called_at_add_time() {
        let called = Arc::new(AtomicBool::new(false));
        let called_d = Arc::clone(&called);

        let it = crate::item::item_with_triggers(
            move |_d| {
                called_d.store(true, Ordering::SeqCst);
                Ok(())
            },
            |_| Ok(ControlFlow::Continue),
        );

        let mut exec = Executor::builder().worker_threads(0).build().unwrap();
        exec.add(it).unwrap();
        assert!(called.load(Ordering::SeqCst));
    }
}
