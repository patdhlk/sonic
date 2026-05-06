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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

// ── Run loop ──────────────────────────────────────────────────────────────────

impl Executor {
    /// Run the executor until [`Stoppable::stop`] is called or a task signals
    /// stop via [`Context::stop_executor`].
    pub fn run(&mut self) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Forever)
    }

    /// Run for at most `max` wall-clock duration, then return.
    pub fn run_for(&mut self, max: Duration) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Until(Instant::now() + max))
    }

    /// Run until `n` full barrier-cycles (`WaitSet` wakeups) have completed.
    pub fn run_n(&mut self, n: usize) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Iterations(n))
    }

    /// Run until `predicate()` returns true. Checked after each `WaitSet`
    /// wakeup.
    pub fn run_until<F: FnMut() -> bool>(
        &mut self,
        mut predicate: F,
    ) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Predicate(&mut predicate))
    }
}

enum RunMode<'a> {
    Forever,
    Until(Instant),
    Iterations(usize),
    Predicate(&'a mut dyn FnMut() -> bool),
}

impl Executor {
    fn run_inner(&mut self, mut mode: RunMode<'_>) -> Result<(), ExecutorError> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(ExecutorError::AlreadyRunning);
        }
        // Reset stop flag for re-runs.
        // NOTE: Stoppable handles obtained before run() via stoppable() are
        // NOT bound to this run's stop flag. Task 9 will re-architect this to
        // propagate waker-aware Stoppable handles to existing clones.
        self.stoppable = Stoppable::new();

        let result = self.dispatch_loop(&mut mode);

        self.running.store(false, Ordering::SeqCst);
        result
    }

    #[allow(
        unsafe_code,
        clippy::too_many_lines,
        clippy::ref_as_ptr,
        clippy::borrow_as_ptr
    )]
    fn dispatch_loop(&mut self, mode: &mut RunMode<'_>) -> Result<(), ExecutorError> {
        let waitset: WaitSet<ipc::Service> = WaitSetBuilder::new()
            .create()
            .map_err(ExecutorError::iceoryx2)?;

        // Keep Arc<RawListener> alive for at least as long as the WaitSet
        // guards — the guard borrows the listener via 'attachment lifetime.
        let mut listener_storage: Vec<Arc<crate::trigger::RawListener>> = Vec::new();
        // Guards must outlive the run loop.
        let mut guards: Vec<WaitSetGuard<'_, '_, ipc::Service>> = Vec::new();
        // Maps guard index → task index.
        let mut attachment_to_task: Vec<usize> = Vec::new();

        for (task_idx, task) in self.tasks.iter().enumerate() {
            for decl in &task.decls {
                match decl {
                    TriggerDecl::Subscriber { listener } => {
                        // Clone Arc to extend listener lifetime to this scope.
                        let l = Arc::clone(listener);
                        listener_storage.push(l);
                        let l_ref = listener_storage.last().unwrap().as_ref();
                        // SAFETY: we cast the reference lifetime to match
                        // 'waitset / 'attachment; both listener_storage and
                        // waitset are stack-local and dropped together at the
                        // end of dispatch_loop.  Guards are dropped before
                        // listener_storage below.
                        let l_ref: &crate::trigger::RawListener =
                            unsafe { &*(l_ref as *const _) };
                        let guard = waitset
                            .attach_notification(l_ref)
                            .map_err(ExecutorError::iceoryx2)?;
                        guards.push(guard);
                        attachment_to_task.push(task_idx);
                    }
                    TriggerDecl::Interval(d) => {
                        let guard = waitset
                            .attach_interval(*d)
                            .map_err(ExecutorError::iceoryx2)?;
                        guards.push(guard);
                        attachment_to_task.push(task_idx);
                    }
                    TriggerDecl::Deadline { listener, deadline } => {
                        let l = Arc::clone(listener);
                        listener_storage.push(l);
                        let l_ref = listener_storage.last().unwrap().as_ref();
                        let l_ref: &crate::trigger::RawListener =
                            unsafe { &*(l_ref as *const _) };
                        let guard = waitset
                            .attach_deadline(l_ref, *deadline)
                            .map_err(ExecutorError::iceoryx2)?;
                        guards.push(guard);
                        attachment_to_task.push(task_idx);
                    }
                    TriggerDecl::RawListener(listener) => {
                        let l = Arc::clone(listener);
                        listener_storage.push(l);
                        let l_ref = listener_storage.last().unwrap().as_ref();
                        let l_ref: &crate::trigger::RawListener =
                            unsafe { &*(l_ref as *const _) };
                        let guard = waitset
                            .attach_notification(l_ref)
                            .map_err(ExecutorError::iceoryx2)?;
                        guards.push(guard);
                        attachment_to_task.push(task_idx);
                    }
                }
            }
        }

        let iterations_done = AtomicUsize::new(0);
        let stop_flag = self.stoppable.clone();

        loop {
            let iter_err: Arc<std::sync::Mutex<Option<ExecutorError>>> =
                Arc::new(std::sync::Mutex::new(None));

            // SAFETY: we capture &mut self.tasks via a raw pointer because
            // wait_and_process_once expects FnMut and Rust cannot see that
            // the closure doesn't outlive `self`. We ensure:
            //   1. The closure runs synchronously within this call.
            //   2. We call pool.barrier() before returning, so all pool
            //      workers that dereference item_ptr have completed.
            //   3. self.tasks is not mutated while the run loop is active.
            let tasks_ptr = &mut self.tasks as *mut Vec<TaskEntry>;
            let pool = &self.pool;
            let stoppable_inner = self.stoppable.clone();

            let cb_result = waitset.wait_and_process_once(
                |attachment_id: WaitSetAttachmentId<ipc::Service>| {
                    for (i, guard) in guards.iter().enumerate() {
                        let fired = attachment_id.has_event_from(guard)
                            || attachment_id.has_missed_deadline(guard);
                        if !fired {
                            continue;
                        }
                        let task_idx = attachment_to_task[i];

                        // SAFETY: we are the only thread that may touch
                        // `self` during the callback. wait_and_process_once
                        // is single-threaded; we hold &mut self in
                        // dispatch_loop. The pointer is valid for the
                        // duration of this closure.
                        let task = unsafe { &mut (&mut *tasks_ptr)[task_idx] };
                        let id = task.id.clone();
                        // SAFETY: SendItemPtr safety doc above. barrier()
                        // guarantees exclusive access within each iteration.
                        let item_ptr =
                            SendItemPtr::new(task.item.as_mut() as *mut _);
                        let stop = stoppable_inner.clone();
                        let err_slot = Arc::clone(&iter_err);

                        pool.submit(move || {
                            // item_ptr.get() is a method call — Rust 2021
                            // per-field capture analysis captures the whole
                            // SendItemPtr (Send) rather than the inner raw
                            // pointer field (not Send by default).
                            let raw = item_ptr.get();
                            let mut ctx = crate::context::Context::new(&id, &stop);
                            // SAFETY: pool.barrier() (below) is called
                            // before we leave the callback scope, so
                            // the borrow of task.item is bounded to this
                            // iteration.
                            #[allow(clippy::option_if_let_else)]
                            let res: crate::ExecuteResult = std::panic::catch_unwind(
                                std::panic::AssertUnwindSafe(|| unsafe {
                                    (*raw).execute(&mut ctx)
                                }),
                            )
                            .unwrap_or_else(|payload| {
                                let msg =
                                    if let Some(s) = payload.downcast_ref::<&str>() {
                                        (*s).to_string()
                                    } else if let Some(s) =
                                        payload.downcast_ref::<String>()
                                    {
                                        s.clone()
                                    } else {
                                        "panicked task".to_string()
                                    };
                                Err::<crate::ControlFlow, crate::ItemError>(Box::new(
                                    PanickedTask(msg),
                                ))
                            });
                            if let Err(source) = res {
                                let mut slot = err_slot.lock().unwrap();
                                if slot.is_none() {
                                    *slot = Some(ExecutorError::Item {
                                        task_id: id,
                                        source,
                                    });
                                }
                            }
                        });
                    }

                    // Wait for all submitted jobs to finish before leaving
                    // the callback scope (validates item_ptr safety contract).
                    pool.barrier();
                    CallbackProgression::Continue
                },
            );

            cb_result.map_err(ExecutorError::iceoryx2)?;

            // Extract the error before dropping the MutexGuard — avoids
            // holding the lock across the return (clippy::significant_drop_in_scrutinee).
            let maybe_err = iter_err.lock().unwrap().take();
            if let Some(err) = maybe_err {
                return Err(err);
            }
            if stop_flag.is_stopped() {
                return Ok(());
            }

            iterations_done.fetch_add(1, Ordering::SeqCst);
            match mode {
                RunMode::Forever => {}
                RunMode::Iterations(n) => {
                    if iterations_done.load(Ordering::SeqCst) >= *n {
                        return Ok(());
                    }
                }
                RunMode::Until(deadline) => {
                    if Instant::now() >= *deadline {
                        return Ok(());
                    }
                }
                RunMode::Predicate(p) => {
                    if (p)() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Wraps a `*mut dyn ExecutableItem` so it can cross thread boundaries inside
/// `Pool::submit`. The send is safe because:
///   1. The executor guarantees at most one invocation of a given item at a
///      time (via `pool.barrier()` before the pointer is reused).
///   2. `ExecutableItem: Send`, so moving the pointee across threads is sound
///      when no aliasing exists.
#[allow(unsafe_code)]
struct SendItemPtr {
    ptr: *mut dyn ExecutableItem,
}

impl SendItemPtr {
    fn new(ptr: *mut dyn ExecutableItem) -> Self {
        Self { ptr }
    }

    /// Returns the raw pointer. Call inside the closure so that Rust 2021
    /// per-field capture analysis captures `self` (the whole `SendItemPtr`,
    /// which is `Send`) rather than `self.ptr` (which is not `Send`).
    fn get(self) -> *mut dyn ExecutableItem {
        self.ptr
    }
}

// SAFETY: see doc comment above.
#[allow(unsafe_code)]
unsafe impl Send for SendItemPtr {}

#[derive(Debug)]
struct PanickedTask(String);

impl core::fmt::Display for PanickedTask {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "task panicked: {}", self.0)
    }
}

impl std::error::Error for PanickedTask {}

// ── Unit tests ────────────────────────────────────────────────────────────────

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
