//! `Executor` and `ExecutorBuilder`. Run loop lives in Task 8.

// Fields consumed by the run loop (Task 8) and graph scheduler (Task 14).
#![allow(dead_code)]
// pub(crate) inside a private module — intentional, Task 8+ will use them.
#![allow(clippy::redundant_pub_crate)]

use crate::context::Stoppable;
use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::monitor::{ExecutionMonitor, NoopMonitor};
use crate::observer::{NoopObserver, Observer};
use crate::pool::Pool;
use crate::shutdown;
use crate::task_id::TaskId;
use crate::task_kind::TaskKind;
use crate::thread_attrs::ThreadAttributes;
use crate::trigger::{TriggerDecl, TriggerDeclarer};
use crate::Channel;
use iceoryx2::node::Node;
use iceoryx2::port::listener::Listener as IxListener;
use iceoryx2::prelude::ipc;
use iceoryx2::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
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
    pub fn channel<T: ZeroCopySend + Default + core::fmt::Debug + 'static>(
        &mut self,
        name: &str,
    ) -> Result<Arc<Channel<T>>, ExecutorError> {
        Channel::open_or_create(&self.node, name)
    }

    /// Open or create a request/response service bound to this executor's node.
    pub fn service<Req, Resp>(
        &mut self,
        name: &str,
    ) -> Result<Arc<crate::Service<Req, Resp>>, ExecutorError>
    where
        Req: ZeroCopySend + Default + core::fmt::Debug + 'static,
        Resp: ZeroCopySend + Default + core::fmt::Debug + 'static,
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
            kind: TaskKind::Single(Box::new(item)),
            decls,
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

        self.tasks.push(TaskEntry {
            id: id.clone(),
            kind: TaskKind::Chain(items),
            decls,
        });
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
    /// Whether to install a process-wide Ctrl-C → [`Stoppable::stop`] bridge.
    /// Defaults to `true` when the `ctrlc-default` feature is enabled.
    install_ctrlc: bool,
}

impl Default for ExecutorBuilder {
    fn default() -> Self {
        Self {
            worker_threads: None,
            observer: None,
            monitor: None,
            worker_attrs: ThreadAttributes::new(),
            install_ctrlc: cfg!(feature = "ctrlc-default"),
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

    /// Override whether to install a process-wide Ctrl-C handler that calls
    /// [`Stoppable::stop`] on SIGINT. Has no effect if the `ctrlc-default`
    /// feature is disabled (the handler is never installed regardless). Pass
    /// `false` if you want to handle SIGINT yourself.
    #[must_use]
    pub const fn install_ctrlc(mut self, yes: bool) -> Self {
        self.install_ctrlc = yes;
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
        };

        if self.install_ctrlc {
            shutdown::install_ctrlc(exec.stoppable.clone())?;
        }

        Ok(exec)
    }
}

// ── Run loop ──────────────────────────────────────────────────────────────────

impl Executor {
    /// Run the executor until [`Stoppable::stop`] is called or a task signals
    /// stop via [`crate::Context::stop_executor`].
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
            let iter_err: Arc<std::sync::Mutex<Option<ExecutorError>>> =
                Arc::new(std::sync::Mutex::new(None));

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
            let stoppable_inner = self.stoppable.clone();
            let observer_inner = Arc::clone(&self.observer);
            let monitor_inner = Arc::clone(&self.monitor);
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
                        let id = task.id.clone();
                        let stop = stoppable_inner.clone();
                        let err_slot = Arc::clone(&iter_err);
                        let obs = Arc::clone(&observer_inner);
                        let mon = Arc::clone(&monitor_inner);

                        match &mut task.kind {
                            TaskKind::Single(item_box) => {
                                // Collect app metadata before moving the pointer.
                                let app_id = item_box.app_id();
                                let app_inst = item_box.app_instance_id();
                                // SAFETY: SendItemPtr safety doc above. barrier()
                                // guarantees exclusive access within each iteration.
                                let item_ptr = SendItemPtr::new(item_box.as_mut() as *mut _);
                                pool.submit(move || {
                                    let mut ctx =
                                        crate::context::Context::new(&id, &stop, obs.as_ref());
                                    if let Some(aid) = app_id {
                                        obs.on_app_start(id.clone(), aid, app_inst);
                                    }
                                    // item_ptr.get() is a method call — Rust 2021
                                    // per-field capture analysis captures the whole
                                    // SendItemPtr (Send) rather than the inner raw
                                    // pointer field (not Send by default).
                                    let raw = item_ptr.get();
                                    // SAFETY: pool.barrier() (below) is called
                                    // before we leave the callback scope, so
                                    // the borrow of item_box is bounded to this
                                    // iteration.
                                    let started = std::time::Instant::now();
                                    mon.pre_execute(id.clone(), started);
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
                                });
                            }
                            TaskKind::Chain(items) => {
                                // Collect app metadata for each chain item before moving pointers.
                                let item_meta: Vec<(Option<u32>, Option<u32>)> = items
                                    .iter()
                                    .map(|b| (b.app_id(), b.app_instance_id()))
                                    .collect();
                                let item_ptrs: Vec<SendItemPtr> = items
                                    .iter_mut()
                                    .map(|b| SendItemPtr::new(b.as_mut() as *mut _))
                                    .collect();
                                pool.submit(move || {
                                    let mut ctx =
                                        crate::context::Context::new(&id, &stop, obs.as_ref());
                                    for (ptr, (app_id, app_inst)) in
                                        item_ptrs.into_iter().zip(item_meta)
                                    {
                                        if let Some(aid) = app_id {
                                            obs.on_app_start(id.clone(), aid, app_inst);
                                        }
                                        // SAFETY: pool.barrier() (below) is called
                                        // before we leave the callback scope, so
                                        // the borrows are bounded to this iteration.
                                        let raw = ptr.get();
                                        let started = std::time::Instant::now();
                                        mon.pre_execute(id.clone(), started);
                                        let res =
                                            run_item_catch_unwind(unsafe { &mut *raw }, &mut ctx);
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
                                });
                            }
                            TaskKind::Graph(graph) => {
                                // Outer driver runs on the WaitSet thread; vertices run on the
                                // pool. This avoids deadlock when worker_threads == 1 because
                                // the WaitSet thread is NOT a pool worker.
                                let pool_arc = Arc::clone(pool);
                                let outcome = graph.run_once(&pool_arc, &id, &stop, &obs, &mon);
                                if let Some(source) = outcome.error {
                                    let mut g = err_slot.lock().unwrap();
                                    if g.is_none() {
                                        *g = Some(ExecutorError::Item {
                                            task_id: id.clone(),
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
    pub fn root(&mut self, v: crate::graph::Vertex) -> &mut Self {
        self.builder.root(v);
        self
    }

    /// Override the auto-generated id with a custom one.
    pub fn id(&mut self, id: impl Into<TaskId>) -> &mut Self {
        self.custom_id = Some(id.into());
        self
    }

    /// Validate and register the graph. Returns the task id.
    pub fn build(self) -> Result<TaskId, ExecutorError> {
        let g = self.builder.finish()?;
        let id = self.custom_id.unwrap_or_else(|| {
            TaskId::new(format!(
                "graph-{}",
                self.executor.next_id.fetch_add(1, Ordering::SeqCst)
            ))
        });
        let decls = g.decls.clone();
        self.executor.tasks.push(TaskEntry {
            id: id.clone(),
            kind: TaskKind::Graph(g),
            decls,
        });
        Ok(id)
    }
}

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
