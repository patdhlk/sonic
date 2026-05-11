//! `Executor` and `ExecutorBuilder`. Run loop lives in Task 8.

// Fields consumed by the run loop (Task 8) and graph scheduler (Task 14).
#![allow(dead_code)]
// pub(crate) inside a private module — intentional, Task 8+ will use them.
#![allow(clippy::redundant_pub_crate)]

use crate::Channel;
use crate::context::Stoppable;
use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::monitor::{ExecutionMonitor, NoopMonitor};
use crate::observer::{NoopObserver, Observer};
use crate::payload::Payload;
use crate::pool::Pool;
use crate::task_id::TaskId;
use crate::task_kind::TaskKind;
use crate::thread_attrs::ThreadAttributes;
use crate::trigger::{TriggerDecl, TriggerDeclarer};
use iceoryx2::node::Node;
use iceoryx2::port::listener::Listener as IxListener;
use iceoryx2::prelude::ipc;
use iceoryx2::prelude::*;
use iceoryx2::waitset::WaitSetRunResult;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Monotonically increasing counter so multiple executors in the same process
/// each get a unique stop-event service name.
static EXEC_COUNTER: AtomicU64 = AtomicU64::new(0);

/// One registered task entry.
pub(crate) struct TaskEntry {
    /// Task identifier.
    pub(crate) id: TaskId,
    /// The kind of work this entry holds (single item or chain).
    pub(crate) kind: TaskKind,
    /// Trigger declarations recorded at `add` time.
    pub(crate) decls: Vec<TriggerDecl>,
    /// Pre-allocated dispatch closure. Built once at `add` / `add_chain`
    /// time and re-invoked on every dispatch iteration via
    /// `Pool::submit_borrowed`, avoiding the per-iteration `Box::new(closure)`
    /// that `Pool::submit<F>` requires in threaded mode. Required for
    /// `REQ_0060` (zero-alloc steady-state dispatch). `None` for
    /// `TaskKind::Graph`, which dispatches its vertices via a separate
    /// path and is handled by `REQ_0062` / `REQ_0063` follow-on work.
    pub(crate) job: Option<Box<dyn FnMut() + Send + 'static>>,
}

/// Top-level executor. One per process is the typical case.
pub struct Executor {
    pub(crate) node: Node<ipc::Service>,
    pub(crate) pool: Arc<Pool>,
    pub(crate) tasks: Vec<TaskEntry>,
    pub(crate) running: Arc<AtomicBool>,
    pub(crate) stoppable: Stoppable,
    pub(crate) next_id: AtomicU64,
    /// Listener for the internal stop event service. Held here so it outlives
    /// the `WaitSet` guard inside `dispatch_loop`. Created at `build()` time so
    /// any `Stoppable` clone (taken before or after `run()`) carries the waker.
    pub(crate) stop_listener: Arc<IxListener<ipc::Service>>,
    /// Lifecycle observer. Defaults to a no-op.
    pub(crate) observer: Arc<dyn Observer>,
    /// Execution monitor. Defaults to a no-op.
    pub(crate) monitor: Arc<dyn ExecutionMonitor>,
    /// Per-iteration error capture slot — allocated once at build time and
    /// reset to `None` at the top of each `dispatch_loop` iteration. Pool
    /// workers obtain a refcount-only `Arc::clone` of this slot, avoiding
    /// the per-iteration heap allocation that the previous design incurred.
    /// Required for `REQ_0060`.
    pub(crate) iter_err: Arc<std::sync::Mutex<Option<ExecutorError>>>,
}

// SAFETY: `IxListener<ipc::Service>` is `!Send` for the same Rc-based
// `SingleThreaded` reason as `IxNotifier`. After construction, the only
// per-iteration call is `listener.try_wait_one()`, which does not mutate the
// Rc. `Executor` is never shared across threads (it requires `&mut self` for
// `run()`), so there is no aliased concurrent mutation.
#[allow(unsafe_code, clippy::non_send_fields_in_send_ty)]
unsafe impl Send for Executor {}

impl Executor {
    /// Start a new builder.
    #[must_use]
    pub fn builder() -> ExecutorBuilder {
        ExecutorBuilder::default()
    }

    /// Open or create a pub/sub channel bound to this executor's node.
    pub fn channel<T: Payload>(&mut self, name: &str) -> Result<Arc<Channel<T>>, ExecutorError> {
        Channel::open_or_create(&self.node, name)
    }

    /// Open or create a request/response service bound to this executor's node.
    pub fn service<Req, Resp>(
        &mut self,
        name: &str,
    ) -> Result<Arc<crate::Service<Req, Resp>>, ExecutorError>
    where
        Req: Payload,
        Resp: Payload,
    {
        crate::Service::open_or_create(&self.node, name)
    }

    /// Add an item to the executor with an auto-generated id.
    pub fn add(&mut self, item: impl ExecutableItem) -> Result<TaskId, ExecutorError> {
        let id = TaskId::new(format!(
            "task-{}",
            self.next_id.fetch_add(1, Ordering::SeqCst)
        ));
        self.add_with_id(id, item)
    }

    /// Add an item with a user-supplied id.
    ///
    /// The item's [`ExecutableItem::task_id`] override takes precedence over
    /// the caller-supplied `id`, which itself takes precedence over the
    /// auto-generated id assigned by [`Executor::add`].
    pub fn add_with_id(
        &mut self,
        id: impl Into<TaskId>,
        mut item: impl ExecutableItem,
    ) -> Result<TaskId, ExecutorError> {
        let id_arg: TaskId = id.into();
        // The item's `task_id()` override wins over the user-supplied id.
        let id = item.task_id().map_or(id_arg, TaskId::new);
        let mut declarer = TriggerDeclarer::new_internal();
        item.declare_triggers(&mut declarer)?;
        let decls = declarer.into_decls();

        let mut item_box: Box<dyn ExecutableItem> = Box::new(item);
        let app_id = item_box.app_id();
        let app_inst = item_box.app_instance_id();
        // SAFETY: the raw pointer points into the heap allocation of
        // `item_box`. `Box` keeps that allocation at a stable address even
        // when the `Box` itself is moved (e.g. when `self.tasks` grows),
        // so the pointer remains valid for the lifetime of the
        // `TaskEntry`. See SendItemPtr safety doc for the rest of the
        // discipline (barrier() pairs with worker access).
        #[allow(unsafe_code)]
        let item_ptr =
            SendItemPtr::new(std::ptr::from_mut::<dyn ExecutableItem>(item_box.as_mut()));

        let job = build_single_job(
            id.clone(),
            self.stoppable.clone(),
            Arc::clone(&self.observer),
            Arc::clone(&self.monitor),
            Arc::clone(&self.iter_err),
            app_id,
            app_inst,
            item_ptr,
        );

        self.tasks.push(TaskEntry {
            id: id.clone(),
            kind: TaskKind::Single(item_box),
            decls,
            job: Some(job),
        });
        Ok(id)
    }

    /// Add a sequential chain of items. Only the head item's
    /// `declare_triggers` is consulted; non-head triggers are ignored with a
    /// tracing warn.
    pub fn add_chain<I, C>(&mut self, items: C) -> Result<TaskId, ExecutorError>
    where
        I: ExecutableItem,
        C: IntoIterator<Item = I>,
    {
        let id = TaskId::new(format!(
            "chain-{}",
            self.next_id.fetch_add(1, Ordering::SeqCst)
        ));
        let boxed: Vec<Box<dyn ExecutableItem>> = items
            .into_iter()
            .map(|i| Box::new(i) as Box<dyn ExecutableItem>)
            .collect();
        self.add_chain_with_id_boxed(id, boxed)
    }

    /// Like [`Executor::add_chain`] but with a user-supplied id.
    pub fn add_chain_with_id<I, C>(
        &mut self,
        id: impl Into<TaskId>,
        items: C,
    ) -> Result<TaskId, ExecutorError>
    where
        I: ExecutableItem,
        C: IntoIterator<Item = I>,
    {
        let boxed: Vec<Box<dyn ExecutableItem>> = items
            .into_iter()
            .map(|i| Box::new(i) as Box<dyn ExecutableItem>)
            .collect();
        self.add_chain_with_id_boxed(id.into(), boxed)
    }

    fn add_chain_with_id_boxed(
        &mut self,
        id: TaskId,
        mut items: Vec<Box<dyn ExecutableItem>>,
    ) -> Result<TaskId, ExecutorError> {
        if items.is_empty() {
            return Err(ExecutorError::Builder(
                "chain must contain at least one item".into(),
            ));
        }

        // Head item's `task_id()` override wins over the user-supplied id.
        let id = items[0].task_id().map_or(id, TaskId::new);

        // Head's triggers gate the chain.
        let mut head_declarer = TriggerDeclarer::new_internal();
        items[0].declare_triggers(&mut head_declarer)?;
        let decls = head_declarer.into_decls();

        // Warn if non-head items declared triggers (those will be ignored).
        for (i, body) in items.iter_mut().enumerate().skip(1) {
            let mut spurious = TriggerDeclarer::new_internal();
            let _ = body.declare_triggers(&mut spurious);
            if !spurious.is_empty() {
                #[cfg(feature = "tracing")]
                tracing::warn!(
                    target: "sonic-executor",
                    task = %id,
                    position = i,
                    "non-head chain item declared triggers; they will be ignored"
                );
                #[cfg(not(feature = "tracing"))]
                {
                    let _ = i;
                }
            }
        }

        let mut items = items;
        // SAFETY: pointer into the chain's `items` Vec. The Vec lives
        // inside `TaskKind::Chain` inside `TaskEntry`. The Vec's buffer
        // is stable once `add_chain` returns — `self.tasks` may grow
        // (moving the `Vec<Box<...>>` header itself), but the Vec's
        // heap buffer is referenced via the header's data pointer and
        // is unaffected by header moves. We never resize the chain Vec
        // after this point. See SendChainPtr safety doc for the rest.
        #[allow(unsafe_code)]
        let chain_ptr = SendChainPtr::new(std::ptr::from_mut::<Vec<Box<dyn ExecutableItem>>>(
            &mut items,
        ));
        // NB: the pointer above is to the local `items` Vec on the
        // stack — it's invalid after the `push` below moves items into
        // the TaskEntry. We rederive a stable pointer after the push.
        // (See the rebuild step below.)
        let _ = chain_ptr;

        self.tasks.push(TaskEntry {
            id: id.clone(),
            kind: TaskKind::Chain(items),
            decls,
            job: None, // populated in the rebuild step below
        });

        // After the push, the TaskEntry lives at a stable position in
        // `self.tasks` for the duration of this `add_chain_with_id_boxed`
        // call. Take a stable pointer to its chain Vec and build the
        // dispatch closure. If `self.tasks` later grows, the Vec header
        // inside the TaskEntry moves but the header's data pointer
        // (which addresses the chain's heap buffer) does not — and the
        // closure derefs that pointer per dispatch, so it re-reads the
        // current heap address each time. Sound under the same
        // discipline as `tasks_ptr` in dispatch_loop.
        let task_idx = self.tasks.len() - 1;
        let chain_vec_ptr: *mut Vec<Box<dyn ExecutableItem>> = match &mut self.tasks[task_idx].kind
        {
            TaskKind::Chain(v) => std::ptr::from_mut::<Vec<Box<dyn ExecutableItem>>>(v),
            // The push above used TaskKind::Chain, so this arm is
            // unreachable. Mark it explicitly to satisfy `match`.
            _ => unreachable!("just-pushed task is TaskKind::Chain"),
        };
        #[allow(unsafe_code)]
        let chain_ptr = SendChainPtr::new(chain_vec_ptr);
        let job = build_chain_job(
            id.clone(),
            self.stoppable.clone(),
            Arc::clone(&self.observer),
            Arc::clone(&self.monitor),
            Arc::clone(&self.iter_err),
            chain_ptr,
        );
        self.tasks[task_idx].job = Some(job);
        Ok(id)
    }

    /// Returns a [`Stoppable`] handle that is waker-aware from the moment the
    /// executor is built. Clone before calling `run()` — any clone taken at any
    /// time will wake the `WaitSet` when `stop()` is called.
    #[must_use]
    pub fn stoppable(&self) -> Stoppable {
        self.stoppable.clone()
    }

    /// Borrow the underlying iceoryx2 node (escape hatch for power users).
    pub const fn iceoryx_node(&self) -> &Node<ipc::Service> {
        &self.node
    }

    /// Begin building a graph. Call `.build()` on the returned builder to
    /// register the graph as a task.
    pub fn add_graph(&mut self) -> ExecutorGraphBuilder<'_> {
        ExecutorGraphBuilder {
            executor: self,
            builder: crate::graph::GraphBuilder::new(),
            custom_id: None,
        }
    }
}

/// Builder for [`Executor`].
pub struct ExecutorBuilder {
    worker_threads: Option<usize>,
    observer: Option<Arc<dyn Observer>>,
    monitor: Option<Arc<dyn ExecutionMonitor>>,
    worker_attrs: ThreadAttributes,
}

impl Default for ExecutorBuilder {
    fn default() -> Self {
        Self {
            worker_threads: None,
            observer: None,
            monitor: None,
            worker_attrs: ThreadAttributes::new(),
        }
    }
}

impl ExecutorBuilder {
    /// Number of worker threads. `0` → inline (no pool). Default → physical
    /// cores.
    #[must_use]
    pub const fn worker_threads(mut self, n: usize) -> Self {
        self.worker_threads = Some(n);
        self
    }

    /// Attach a lifecycle observer. If not called, a no-op observer is used.
    #[must_use]
    pub fn observer(mut self, obs: Arc<dyn Observer>) -> Self {
        self.observer = Some(obs);
        self
    }

    /// Attach an execution monitor. If not called, a no-op monitor is used.
    #[must_use]
    pub fn monitor(mut self, mon: Arc<dyn ExecutionMonitor>) -> Self {
        self.monitor = Some(mon);
        self
    }

    /// Set thread attributes (name prefix, CPU affinity, scheduling priority)
    /// for worker threads. Has no effect when `worker_threads` is `0` (inline
    /// mode). Requires the `thread_attrs` feature for non-default settings.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn worker_attrs(mut self, attrs: ThreadAttributes) -> Self {
        self.worker_attrs = attrs;
        self
    }

    /// Build the [`Executor`]. Creates a fresh iceoryx2 node and wires up the
    /// internal stop-event service so that any `Stoppable` clone (taken before
    /// or after `run()`) will wake the `WaitSet` when `stop()` is called.
    ///
    /// # Panics
    ///
    /// Panics if the internally-generated stop-event service name exceeds the
    /// iceoryx2 service name length limit (this cannot happen under normal use
    /// because the name is derived from the process id and a monotonic counter).
    #[allow(clippy::arc_with_non_send_sync)] // see SAFETY on `impl Send for Executor`
    #[track_caller]
    pub fn build(self) -> Result<Executor, ExecutorError> {
        let node = NodeBuilder::new()
            .create::<ipc::Service>()
            .map_err(ExecutorError::iceoryx2)?;

        let n_workers = self.worker_threads.unwrap_or_else(num_cpus::get_physical);
        let pool = Arc::new(Pool::new(n_workers, self.worker_attrs)?);

        // Build the internal stop event service with a unique-per-process name
        // so multiple executors in the same process don't collide.
        let exec_seq = EXEC_COUNTER.fetch_add(1, Ordering::Relaxed);
        let stop_topic = format!(
            "sonic.exec.stop.{}.{exec_seq}.__sonic_event",
            std::process::id()
        );
        let stop_event = node
            .service_builder(&stop_topic.as_str().try_into().unwrap())
            .event()
            .open_or_create()
            .map_err(ExecutorError::iceoryx2)?;

        let stop_notifier = Arc::new(
            stop_event
                .notifier_builder()
                .create()
                .map_err(ExecutorError::iceoryx2)?,
        );

        // SAFETY: see module-level note; Arc<IxListener> is held here and only
        // accessed on the executor thread.
        let stop_listener = Arc::new(
            stop_event
                .listener_builder()
                .create()
                .map_err(ExecutorError::iceoryx2)?,
        );

        // Wire the notifier into the Stoppable so every clone is waker-aware
        // from the moment the executor is built.
        let stoppable = Stoppable::with_waker(stop_notifier);

        let observer: Arc<dyn Observer> = self.observer.unwrap_or_else(|| Arc::new(NoopObserver));

        let monitor: Arc<dyn ExecutionMonitor> =
            self.monitor.unwrap_or_else(|| Arc::new(NoopMonitor));

        let exec = Executor {
            node,
            pool,
            tasks: Vec::new(),
            running: Arc::new(AtomicBool::new(false)),
            stoppable,
            next_id: AtomicU64::new(0),
            stop_listener,
            observer,
            monitor,
            iter_err: Arc::new(std::sync::Mutex::new(None)),
        };

        Ok(exec)
    }
}

// ── Run loop ──────────────────────────────────────────────────────────────────

impl Executor {
    /// Run the executor until [`Stoppable::stop`] is called or a task signals
    /// stop via [`crate::Context::stop_executor`].
    ///
    /// # Errors
    ///
    /// Returns the **first** [`ExecutorError`] surfaced during dispatch:
    ///
    /// * [`ExecutorError::Item`] if any item returns `Err` or panics.
    /// * [`ExecutorError::Iceoryx2`] if a `WaitSet` operation fails.
    /// * [`ExecutorError::AlreadyRunning`] if the executor is already running.
    ///
    /// If multiple items error in the same dispatch iteration, only the first
    /// is preserved; subsequent errors are discarded silently. To observe
    /// every error, attach an [`Observer`](crate::Observer) and read errors
    /// via [`Observer::on_app_error`](crate::Observer::on_app_error).
    pub fn run(&mut self) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Forever)
    }

    /// Run for at most `max` wall-clock duration, then return.
    ///
    /// # Errors
    ///
    /// Returns the **first** [`ExecutorError`] surfaced during dispatch:
    ///
    /// * [`ExecutorError::Item`] if any item returns `Err` or panics.
    /// * [`ExecutorError::Iceoryx2`] if a `WaitSet` operation fails.
    /// * [`ExecutorError::AlreadyRunning`] if the executor is already running.
    ///
    /// If multiple items error in the same dispatch iteration, only the first
    /// is preserved; subsequent errors are discarded silently. To observe
    /// every error, attach an [`Observer`](crate::Observer) and read errors
    /// via [`Observer::on_app_error`](crate::Observer::on_app_error).
    pub fn run_for(&mut self, max: Duration) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Until(Instant::now() + max))
    }

    /// Run until `n` full barrier-cycles (`WaitSet` wakeups) have completed.
    ///
    /// # Errors
    ///
    /// Returns the **first** [`ExecutorError`] surfaced during dispatch:
    ///
    /// * [`ExecutorError::Item`] if any item returns `Err` or panics.
    /// * [`ExecutorError::Iceoryx2`] if a `WaitSet` operation fails.
    /// * [`ExecutorError::AlreadyRunning`] if the executor is already running.
    ///
    /// If multiple items error in the same dispatch iteration, only the first
    /// is preserved; subsequent errors are discarded silently. To observe
    /// every error, attach an [`Observer`](crate::Observer) and read errors
    /// via [`Observer::on_app_error`](crate::Observer::on_app_error).
    pub fn run_n(&mut self, n: usize) -> Result<(), ExecutorError> {
        self.run_inner(RunMode::Iterations(n))
    }

    /// Run until `predicate()` returns true. Checked after each `WaitSet`
    /// wakeup.
    ///
    /// # Errors
    ///
    /// Returns the **first** [`ExecutorError`] surfaced during dispatch:
    ///
    /// * [`ExecutorError::Item`] if any item returns `Err` or panics.
    /// * [`ExecutorError::Iceoryx2`] if a `WaitSet` operation fails.
    /// * [`ExecutorError::AlreadyRunning`] if the executor is already running.
    ///
    /// If multiple items error in the same dispatch iteration, only the first
    /// is preserved; subsequent errors are discarded silently. To observe
    /// every error, attach an [`Observer`](crate::Observer) and read errors
    /// via [`Observer::on_app_error`](crate::Observer::on_app_error).
    pub fn run_until<F: FnMut() -> bool>(&mut self, mut predicate: F) -> Result<(), ExecutorError> {
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
        // NOTE: Once `Stoppable::stop()` has been called, `self.stoppable.is_stopped()`
        // remains true permanently. Calling `run()` again after a stop will return
        // promptly without doing any meaningful work (it blocks until the first
        // trigger fires, then immediately exits the dispatch loop). Task 10's
        // Runner accommodates this by treating an Executor as one-shot: each
        // Runner owns the Executor and consumes it.
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(ExecutorError::AlreadyRunning);
        }

        self.observer.on_executor_up();
        let result = self.dispatch_loop(&mut mode);
        match &result {
            Ok(()) => self.observer.on_executor_down(),
            Err(e) => self.observer.on_executor_error(e),
        }

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
                        let l_ref: &crate::trigger::RawListener = unsafe { &*(l_ref as *const _) };
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
                        let l_ref: &crate::trigger::RawListener = unsafe { &*(l_ref as *const _) };
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
                        let l_ref: &crate::trigger::RawListener = unsafe { &*(l_ref as *const _) };
                        let guard = waitset
                            .attach_notification(l_ref)
                            .map_err(ExecutorError::iceoryx2)?;
                        guards.push(guard);
                        attachment_to_task.push(task_idx);
                    }
                }
            }
        }

        // Attach the internal stop listener so the WaitSet wakes when
        // stop() is called. We hold `self.stop_listener` (Arc) in the Executor
        // struct which is valid for the lifetime of dispatch_loop. We use the
        // same raw-pointer-cast pattern as user listeners above.
        //
        // SAFETY: `self.stop_listener` is an Arc stored on `self`, which is
        // exclusively borrowed for the duration of `run_inner` (which calls
        // `dispatch_loop`). The listener is not freed while the guard is alive
        // because the Arc keeps it alive and `self` outlives this function.
        let stop_listener_ref: &IxListener<ipc::Service> =
            unsafe { &*(self.stop_listener.as_ref() as *const _) };
        let _stop_guard = waitset
            .attach_notification(stop_listener_ref)
            .map_err(ExecutorError::iceoryx2)?;

        let iterations_done = AtomicUsize::new(0);
        let stop_flag = self.stoppable.clone();

        loop {
            // Reset the pre-allocated per-iteration error slot (REQ_0060):
            // the slot is owned by `self.iter_err`, allocated once at build
            // time. Pool worker closures obtain a refcount-only clone of
            // the `Arc`; the slot itself is reused across iterations.
            *self.iter_err.lock().unwrap() = None;

            // SAFETY: we capture &mut self.tasks via a raw pointer because
            // wait_and_process expects FnMut and Rust can't see the closure
            // outlives `self`. The discipline that makes this sound:
            //   1. The closure body on the executor thread is the *only* code that
            //      reads `tasks_ptr`. The pool jobs it submits hold borrowed
            //      `*mut dyn ExecutableItem` slices into individual TaskEntries,
            //      not into the Vec itself, so they don't race with the Vec.
            //   2. `pool.barrier()` at the end of this callback ensures every
            //      submitted pool job has completed (and dropped its raw pointer)
            //      before the callback returns. The next iteration of the WaitSet
            //      loop is therefore the sole user of `tasks_ptr` again.
            //   3. The Vec is never resized inside this loop (no `push` / `remove`
            //      after dispatch starts), so the underlying buffer addresses are
            //      stable for the lifetime of `dispatch_loop`.
            let tasks_ptr = &mut self.tasks as *mut Vec<TaskEntry>;
            let pool = &self.pool;
            // Refcount-only clone of the pre-allocated error slot. Pool jobs
            // need a `'static` handle, and an `Arc::clone` does not allocate.
            // The Single/Chain paths use the closure baked into `task.job`,
            // which already captured stable Arc clones at `add`-time; the
            // Graph path uses closures pre-built by `prepare_dispatch`. Only
            // the error-aggregation logic on the WaitSet thread still needs
            // the slot here.
            let iter_err_inner = Arc::clone(&self.iter_err);
            // Raw pointer to the stop listener for draining inside the callback.
            // SAFETY: same as stop_listener_ref above — the Arc is alive for
            // the lifetime of dispatch_loop.
            let stop_listener_ptr = self.stop_listener.as_ref() as *const IxListener<ipc::Service>;

            let cb_result = waitset.wait_and_process_once(
                |attachment_id: WaitSetAttachmentId<ipc::Service>| {
                    // Drain stop notifications first (no dispatch — the stop_flag
                    // check after the callback returns handles termination).
                    // SAFETY: stop_listener_ptr is valid for the duration of the
                    // closure; the Arc in self.stop_listener keeps it alive.
                    let stop_l = unsafe { &*stop_listener_ptr };
                    while let Ok(Some(_)) = stop_l.try_wait_one() {}

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

                        match &mut task.kind {
                            TaskKind::Single(_) | TaskKind::Chain(_) => {
                                // The dispatch closure was pre-allocated at
                                // task-add time and stashed on `task.job`.
                                // Submit it via `submit_borrowed` — no
                                // per-iteration Box allocation. Required by
                                // REQ_0060.
                                let job_box = task
                                    .job
                                    .as_deref_mut()
                                    .expect("Single/Chain tasks carry a pre-built job");
                                let job_ptr: *mut (dyn FnMut() + Send) =
                                    job_box as *mut (dyn FnMut() + Send);
                                // SAFETY: the closure lives in
                                // `task.job` which is owned by
                                // `self.tasks[task_idx]`; `tasks_ptr` is
                                // sound for the duration of this
                                // callback. `pool.barrier()` below
                                // finishes the closure invocation before
                                // we re-enter the next iteration's
                                // callback. The WaitSet thread does not
                                // touch the closure between this submit
                                // and that barrier.
                                #[allow(unsafe_code)]
                                unsafe {
                                    pool.submit_borrowed(crate::pool::BorrowedJob::new(job_ptr));
                                }
                            }
                            TaskKind::Graph(graph) => {
                                // Outer driver runs on the WaitSet thread; vertices run on the
                                // pool. The graph holds its own pre-built per-vertex closures
                                // and SPSC ready ring (REQ_0060), so dispatch is allocation-free
                                // in steady state.
                                let outcome = graph.run_once_borrowed(pool);
                                if let Some(source) = outcome.error {
                                    let mut g = iter_err_inner.lock().unwrap();
                                    if g.is_none() {
                                        *g = Some(ExecutorError::Item {
                                            task_id: task.id.clone(),
                                            source,
                                        });
                                    }
                                }
                                let _ = outcome.stopped_chain; // chain-abort semantics: no extra bookkeeping at task level
                            }
                        }
                    }

                    // Wait for all submitted jobs to finish before leaving
                    // the callback scope (validates item_ptr safety contract).
                    pool.barrier();
                    CallbackProgression::Continue
                },
            );

            let cb_result = cb_result.map_err(ExecutorError::iceoryx2)?;

            // iceoryx2's WaitSet catches SIGINT/SIGTERM internally; honor that
            // here for a clean exit.
            if matches!(
                cb_result,
                WaitSetRunResult::Interrupt | WaitSetRunResult::TerminationRequest
            ) {
                return Ok(());
            }

            // Extract the error before dropping the MutexGuard — avoids
            // holding the lock across the return (clippy::significant_drop_in_scrutinee).
            let maybe_err = self.iter_err.lock().unwrap().take();
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

    /// Returns the raw pointer. Takes `&self` so the wrapper can be invoked
    /// repeatedly from an `FnMut` dispatch closure (`REQ_0060` requires the
    /// dispatch closure to be reusable across iterations without allocation).
    fn get(&self) -> *mut dyn ExecutableItem {
        self.ptr
    }
}

// SAFETY: see doc comment above. `Sync` is required so the FnMut dispatch
// closure can borrow `&SendItemPtr` per invocation without making the
// closure itself `!Send`.
#[allow(unsafe_code)]
unsafe impl Send for SendItemPtr {}
#[allow(unsafe_code)]
unsafe impl Sync for SendItemPtr {}

/// Wraps a `*mut Vec<Box<dyn ExecutableItem>>` so a chain dispatch
/// closure can iterate the chain's items in place without first
/// collecting them into a freshly-allocated `Vec`. The send is safe
/// for the same reason as [`SendItemPtr`] (see above): the executor
/// holds `&mut self` for the duration of `dispatch_loop`, and the
/// `pool.barrier()` at the end of each callback ensures the closure
/// has finished using this pointer before the Vec could be touched
/// from the `WaitSet` thread again. The Vec is never resized after
/// dispatch begins. Required for `REQ_0060` — chain dispatch must not
/// allocate per iteration.
#[allow(unsafe_code)]
struct SendChainPtr {
    ptr: *mut Vec<Box<dyn ExecutableItem>>,
}

impl SendChainPtr {
    fn new(ptr: *mut Vec<Box<dyn ExecutableItem>>) -> Self {
        Self { ptr }
    }

    fn get(&self) -> *mut Vec<Box<dyn ExecutableItem>> {
        self.ptr
    }
}

// SAFETY: see doc comment above. `Sync` lets the FnMut dispatch closure
// borrow `&SendChainPtr` per invocation while staying `Send`.
#[allow(unsafe_code)]
unsafe impl Send for SendChainPtr {}
#[allow(unsafe_code)]
unsafe impl Sync for SendChainPtr {}

/// Build the per-iteration dispatch closure for a `TaskKind::Single`.
///
/// The returned closure is stored on `TaskEntry::job` and invoked once
/// per dispatch via `Pool::submit_borrowed`, which (unlike `submit`)
/// performs no allocation. The closure captures Arc clones of the
/// executor's shared state — those clones are refcount-only at build
/// time and are reused on every dispatch. Required for `REQ_0060`.
#[allow(clippy::too_many_arguments)]
fn build_single_job(
    id: TaskId,
    stop: Stoppable,
    obs: Arc<dyn Observer>,
    mon: Arc<dyn ExecutionMonitor>,
    err_slot: Arc<std::sync::Mutex<Option<ExecutorError>>>,
    app_id: Option<u32>,
    app_inst: Option<u32>,
    item_ptr: SendItemPtr,
) -> Box<dyn FnMut() + Send + 'static> {
    Box::new(move || {
        let mut ctx = crate::context::Context::new(&id, &stop, obs.as_ref());
        if let Some(aid) = app_id {
            obs.on_app_start(id.clone(), aid, app_inst);
        }
        let raw = item_ptr.get();
        let started = std::time::Instant::now();
        mon.pre_execute(id.clone(), started);
        // SAFETY: barrier() pairs with this invocation; the WaitSet
        // thread does not touch the item between `submit_borrowed` and
        // the matching `barrier()`. See SendItemPtr safety doc.
        #[allow(unsafe_code)]
        let res = run_item_catch_unwind(unsafe { &mut *raw }, &mut ctx);
        let took = started.elapsed();
        mon.post_execute(id.clone(), started, took, res.is_ok());
        if let Err(ref e) = res {
            obs.on_app_error(id.clone(), e.as_ref());
        }
        if app_id.is_some() {
            obs.on_app_stop(id.clone());
        }
        record_first_err(&err_slot, &id, res);
    })
}

/// Build the per-iteration dispatch closure for a `TaskKind::Chain`.
fn build_chain_job(
    id: TaskId,
    stop: Stoppable,
    obs: Arc<dyn Observer>,
    mon: Arc<dyn ExecutionMonitor>,
    err_slot: Arc<std::sync::Mutex<Option<ExecutorError>>>,
    chain_ptr: SendChainPtr,
) -> Box<dyn FnMut() + Send + 'static> {
    Box::new(move || {
        let mut ctx = crate::context::Context::new(&id, &stop, obs.as_ref());
        // SAFETY: barrier() pairs with this invocation; the chain Vec
        // and the items it owns are not touched by the WaitSet thread
        // until barrier() returns. See SendChainPtr safety doc.
        #[allow(unsafe_code)]
        let chain_items = unsafe { &mut *chain_ptr.get() };
        for item_box in chain_items.iter_mut() {
            let app_id = item_box.app_id();
            let app_inst = item_box.app_instance_id();
            if let Some(aid) = app_id {
                obs.on_app_start(id.clone(), aid, app_inst);
            }
            let raw = std::ptr::from_mut::<dyn ExecutableItem>(item_box.as_mut());
            let started = std::time::Instant::now();
            mon.pre_execute(id.clone(), started);
            #[allow(unsafe_code)]
            let res = run_item_catch_unwind(unsafe { &mut *raw }, &mut ctx);
            let took = started.elapsed();
            mon.post_execute(id.clone(), started, took, res.is_ok());
            if let Err(ref e) = res {
                obs.on_app_error(id.clone(), e.as_ref());
            }
            if app_id.is_some() {
                obs.on_app_stop(id.clone());
            }
            match res {
                Ok(crate::ControlFlow::Continue) => {}
                Ok(crate::ControlFlow::StopChain) => break,
                Err(_) => {
                    record_first_err(&err_slot, &id, res);
                    break;
                }
            }
        }
    })
}

#[derive(Debug)]
struct PanickedTask(String);

impl core::fmt::Display for PanickedTask {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "task panicked: {}", self.0)
    }
}

impl std::error::Error for PanickedTask {}

/// Execute `item` inside `catch_unwind`, converting any panic into an `Err`.
#[allow(clippy::option_if_let_else)]
fn run_item_catch_unwind(
    item: &mut dyn ExecutableItem,
    ctx: &mut crate::context::Context<'_>,
) -> crate::ExecuteResult {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| item.execute(ctx))).unwrap_or_else(
        |payload| {
            let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = payload.downcast_ref::<String>() {
                s.clone()
            } else {
                "panicked task".to_string()
            };
            Err::<crate::ControlFlow, crate::ItemError>(Box::new(PanickedTask(msg)))
        },
    )
}

/// Public-within-crate wrapper so `graph.rs` can call `run_item_catch_unwind`
/// without depending on its private name.
pub(crate) fn run_item_catch_unwind_external(
    item: &mut dyn ExecutableItem,
    ctx: &mut crate::context::Context<'_>,
) -> crate::ExecuteResult {
    run_item_catch_unwind(item, ctx)
}

/// Record the first error into `slot`. Subsequent errors are silently dropped.
fn record_first_err(
    slot: &Arc<std::sync::Mutex<Option<ExecutorError>>>,
    id: &TaskId,
    res: crate::ExecuteResult,
) {
    if let Err(source) = res {
        let mut g = slot.lock().unwrap();
        if g.is_none() {
            *g = Some(ExecutorError::Item {
                task_id: id.clone(),
                source,
            });
        }
    }
}

// ── ExecutorGraphBuilder ──────────────────────────────────────────────────────

/// Borrowed wrapper that finalises a [`GraphBuilder`](crate::graph::GraphBuilder)
/// into a registered task.
pub struct ExecutorGraphBuilder<'e> {
    executor: &'e mut Executor,
    builder: crate::graph::GraphBuilder,
    custom_id: Option<TaskId>,
}

impl ExecutorGraphBuilder<'_> {
    /// Add a vertex to the graph; returns its handle.
    pub fn vertex<I: ExecutableItem>(&mut self, item: I) -> crate::graph::Vertex {
        self.builder.vertex(item)
    }

    /// Add a directed edge from one vertex to another.
    pub fn edge(&mut self, from: crate::graph::Vertex, to: crate::graph::Vertex) -> &mut Self {
        self.builder.edge(from, to);
        self
    }

    /// Designate the root vertex (its triggers gate the graph).
    pub const fn root(&mut self, v: crate::graph::Vertex) -> &mut Self {
        self.builder.root(v);
        self
    }

    /// Override the auto-generated id with a custom one.
    pub fn id(&mut self, id: impl Into<TaskId>) -> &mut Self {
        self.custom_id = Some(id.into());
        self
    }

    /// Validate and register the graph. Returns the task id.
    ///
    /// The root vertex's [`ExecutableItem::task_id`] override takes precedence
    /// over any id set via [`ExecutorGraphBuilder::id`], which itself takes
    /// precedence over the auto-generated id.
    pub fn build(self) -> Result<TaskId, ExecutorError> {
        let g = self.builder.finish()?;
        // Root vertex's task_id() override wins over the custom id, which wins
        // over the auto-generated fallback.
        let auto_id = || {
            TaskId::new(format!(
                "graph-{}",
                self.executor.next_id.fetch_add(1, Ordering::SeqCst)
            ))
        };
        let id = g
            .root_task_id()
            .map(TaskId::new)
            .or(self.custom_id)
            .unwrap_or_else(auto_id);
        let decls = g.decls.clone();

        // Box the graph for address stability — per-vertex dispatch
        // closures capture `*const Graph` and must not see it move.
        let mut graph_box: Box<crate::graph::Graph> = Box::new(g);
        // Pre-build the per-vertex closures now that we know the
        // task_id and have access to the executor's shared state.
        graph_box.prepare_dispatch(
            id.clone(),
            self.executor.stoppable.clone(),
            Arc::clone(&self.executor.observer),
            Arc::clone(&self.executor.monitor),
            Arc::clone(&self.executor.iter_err),
        );

        self.executor.tasks.push(TaskEntry {
            id: id.clone(),
            kind: TaskKind::Graph(graph_box),
            decls,
            // Graph tasks dispatch their vertices via `vertex_jobs`
            // stored inside the `Graph`; the per-task `job` slot
            // is unused for graphs.
            job: None,
        });
        Ok(id)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlFlow, item};

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
