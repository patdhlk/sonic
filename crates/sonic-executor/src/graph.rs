//! Parallel-execution graph: a DAG of [`ExecutableItem`]s rooted at a single
//! vertex whose triggers gate the whole graph.

use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::trigger::{TriggerDecl, TriggerDeclarer};

/// Opaque handle to a graph vertex. Returned by [`GraphBuilder::vertex`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Vertex(pub(crate) usize);

/// Internal graph storage.
///
/// Stored inside `TaskKind::Graph(Box<Graph>)` to guarantee a stable heap
/// address — the per-vertex dispatch closures capture a `*const Graph`
/// pointing back into this struct, and would dangle if the `Graph` moved.
/// All runtime state below is pre-allocated at `finish()` time and reset
/// in place each `run_once_borrowed` call. Required for `REQ_0060`.
#[allow(clippy::redundant_pub_crate)]
pub(crate) struct Graph {
    pub(crate) items: Vec<Box<dyn ExecutableItem>>,
    pub(crate) successors: Vec<Vec<usize>>, // adjacency list
    pub(crate) in_degree: Vec<usize>,       // initial in-degree
    pub(crate) root: usize,
    pub(crate) decls: Vec<TriggerDecl>,

    // ── Pre-allocated dispatch state (REQ_0060) ────────────────────────
    /// Stable raw pointers into each item's heap-allocated `Box`.
    /// Populated once in `finish`. The `Box` contents do not move when
    /// the outer `Vec` resizes, so these pointers stay valid for the
    /// lifetime of the `Graph`.
    vertex_ptrs: Vec<VertexPtr>,
    /// Per-vertex in-degree counter; reset to `in_degree[i]` at the top
    /// of every `run_once_borrowed`. `usize::MAX` is used as a "cancelled"
    /// sentinel during stop-flag propagation.
    counters: Vec<AtomicUsize>,
    /// Number of vertices still pending in the current run.
    pending: AtomicUsize,
    /// Stop request observed during this run.
    stop_flag: AtomicBool,
    /// `ControlFlow::StopChain` observed during this run.
    stop_chain_seen: AtomicBool,
    /// First per-vertex error observed during this run.
    first_err: Mutex<Option<crate::error::ItemError>>,
    /// Completion condvar; signalled when `pending` reaches zero.
    done_cv: (Mutex<()>, Condvar),
    /// Re-dispatch ring — completed pool workers push ready successors;
    /// the `WaitSet` thread drains and re-dispatches. Sized to
    /// `next_power_of_two(n_vertices)` at `finish`. Required for `REQ_0060`.
    ready_ring: crate::ready_ring::ReadyRing,
    /// Per-vertex pre-built dispatch closures. Empty after `finish`,
    /// populated by `prepare_dispatch` when the graph is registered with
    /// an executor (it needs `task_id`/`stop`/`obs`/`mon`/`err_slot` from
    /// the executor). Used by `run_once_borrowed` via
    /// `Pool::submit_borrowed`, avoiding the per-vertex `Box` allocation
    /// that `Pool::submit` requires.
    vertex_jobs: Vec<Box<dyn FnMut() + Send + 'static>>,
}

impl core::fmt::Debug for Graph {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Graph")
            .field("n_items", &self.items.len())
            .field("successors", &self.successors)
            .field("in_degree", &self.in_degree)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl Graph {
    /// Return the root vertex's `task_id()` override, if any.
    pub(crate) fn root_task_id(&self) -> Option<&str> {
        self.items[self.root].task_id()
    }
}

/// Builder for a graph.
pub struct GraphBuilder {
    items: Vec<Box<dyn ExecutableItem>>,
    edges: Vec<(usize, usize)>,
    root: Option<usize>,
}

impl GraphBuilder {
    pub(crate) fn new() -> Self {
        Self {
            items: Vec::new(),
            edges: Vec::new(),
            root: None,
        }
    }

    /// Add a vertex; returns its handle.
    pub fn vertex<I: ExecutableItem>(&mut self, item: I) -> Vertex {
        let idx = self.items.len();
        self.items.push(Box::new(item));
        Vertex(idx)
    }

    /// Add a directed edge `from -> to`.
    pub fn edge(&mut self, from: Vertex, to: Vertex) -> &mut Self {
        self.edges.push((from.0, to.0));
        self
    }

    /// Designate the root vertex (whose triggers gate the graph).
    pub const fn root(&mut self, v: Vertex) -> &mut Self {
        self.root = Some(v.0);
        self
    }

    /// Build, validating connectedness, acyclicity, and exactly-one root.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn finish(mut self) -> Result<Graph, ExecutorError> {
        let n = self.items.len();
        if n == 0 {
            return Err(ExecutorError::InvalidGraph("graph has no vertices".into()));
        }
        let root = self
            .root
            .ok_or_else(|| ExecutorError::InvalidGraph("no root vertex set".into()))?;
        if root >= n {
            return Err(ExecutorError::InvalidGraph(
                "root index out of bounds".into(),
            ));
        }

        let mut successors = vec![Vec::<usize>::new(); n];
        let mut in_degree = vec![0_usize; n];
        for &(from, to) in &self.edges {
            if from >= n || to >= n {
                return Err(ExecutorError::InvalidGraph(
                    "edge index out of bounds".into(),
                ));
            }
            if from == to {
                return Err(ExecutorError::InvalidGraph(
                    "self-loops are not allowed".into(),
                ));
            }
            successors[from].push(to);
            in_degree[to] += 1;
        }

        // Acyclicity via Kahn's algorithm — clone in_degree because we mutate.
        let mut k_in = in_degree.clone();
        let mut queue: Vec<usize> = k_in
            .iter()
            .enumerate()
            .filter_map(|(i, d)| (*d == 0).then_some(i))
            .collect();
        let mut visited = 0_usize;
        while let Some(u) = queue.pop() {
            visited += 1;
            for &v in &successors[u] {
                k_in[v] -= 1;
                if k_in[v] == 0 {
                    queue.push(v);
                }
            }
        }
        if visited != n {
            return Err(ExecutorError::InvalidGraph("graph contains a cycle".into()));
        }

        // Reachability from root (DFS).
        let mut reach = vec![false; n];
        let mut stack = vec![root];
        while let Some(u) = stack.pop() {
            if reach[u] {
                continue;
            }
            reach[u] = true;
            for &v in &successors[u] {
                stack.push(v);
            }
        }
        if reach.iter().any(|r| !*r) {
            return Err(ExecutorError::InvalidGraph(
                "every vertex must be reachable from the root".into(),
            ));
        }

        // Root's triggers gate the graph.
        let mut decl = TriggerDeclarer::new_internal();
        self.items[root].declare_triggers(&mut decl)?;
        let decls = decl.into_decls();

        // Warn if non-root vertices declared triggers (ignored).
        for (i, body) in self.items.iter_mut().enumerate() {
            if i == root {
                continue;
            }
            let mut spurious = TriggerDeclarer::new_internal();
            let _ = body.declare_triggers(&mut spurious);
            if !spurious.is_empty() {
                #[cfg(feature = "tracing")]
                tracing::warn!(target: "sonic-executor", vertex = i,
                    "non-root graph vertex declared triggers; ignored");
            }
        }

        let n_items = self.items.len();
        let mut items = self.items;
        // SAFETY: each `Box<dyn ExecutableItem>` is heap-allocated; its
        // contents do not move when the outer Vec resizes. Stable.
        #[allow(unsafe_code)]
        let vertex_ptrs: Vec<VertexPtr> = items
            .iter_mut()
            .map(|b| VertexPtr(std::ptr::from_mut(b.as_mut())))
            .collect();
        let counters: Vec<AtomicUsize> = in_degree.iter().map(|d| AtomicUsize::new(*d)).collect();

        Ok(Graph {
            items,
            successors,
            in_degree,
            root,
            decls,
            vertex_ptrs,
            counters,
            pending: AtomicUsize::new(n_items),
            stop_flag: AtomicBool::new(false),
            stop_chain_seen: AtomicBool::new(false),
            first_err: Mutex::new(None),
            done_cv: (Mutex::new(()), Condvar::new()),
            ready_ring: crate::ready_ring::ReadyRing::new(n_items),
            vertex_jobs: Vec::new(),
        })
    }
}

// ── Graph scheduler (Task 14) ─────────────────────────────────────────────────

use crate::context::Stoppable;
use crate::monitor::ExecutionMonitor;
use crate::observer::Observer;
use crate::pool::Pool;
use crate::task_id::TaskId;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// Outcome of running a graph once.
#[allow(clippy::redundant_pub_crate)]
pub(crate) struct GraphRunOutcome {
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) error: Option<crate::error::ItemError>,
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) stopped_chain: bool,
}

/// Wrapper around `*mut dyn ExecutableItem` that asserts Send+Sync.
struct VertexPtr(*mut dyn ExecutableItem);

// SAFETY: the executor guarantees a vertex runs on at most one thread at
// a time (the in-degree counter sequences dispatches), and the pointer is
// stable for the lifetime of `Graph::run_once` (the underlying Box is not
// moved while we hold &mut self in run_once).
#[allow(unsafe_code)]
unsafe impl Send for VertexPtr {}
#[allow(unsafe_code)]
unsafe impl Sync for VertexPtr {}

/// Send-able raw pointer back into a `Box<Graph>`. Used by the per-vertex
/// dispatch closures to reach the graph's atomics and the ready ring
/// without an `Arc`. Sound because the `Graph` is owned by
/// `TaskKind::Graph(Box<Graph>)`, which keeps it at a stable heap
/// address, and `pool.barrier()` (in `dispatch_loop`) serialises the
/// closure's invocation with the executor thread's own access.
#[allow(unsafe_code)]
#[derive(Copy, Clone)]
struct SendGraphPtr(*const Graph);

impl SendGraphPtr {
    /// Return the underlying pointer. Method form so Rust 2021 per-field
    /// capture analysis grabs the whole `SendGraphPtr` (which is `Send +
    /// Sync`) rather than `self.0` (a `*const`, which is not).
    const fn get(&self) -> *const Graph {
        self.0
    }
}

#[allow(unsafe_code)]
unsafe impl Send for SendGraphPtr {}
#[allow(unsafe_code)]
unsafe impl Sync for SendGraphPtr {}

impl Graph {
    fn finalise_skipped(&self, i: usize) {
        if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.notify_done();
            return;
        }
        for &j in &self.successors[i] {
            self.cancel_subtree(j);
        }
    }

    fn cancel_subtree(&self, root: usize) {
        // Iterative DFS using a small stack on the heap. The stack is
        // bounded by the number of vertices and used only on the stop
        // path; the steady-state happy path (REQ_0060) never enters
        // here, so this stack's allocation does not violate the
        // requirement. A pre-allocated scratch stack would be needed if
        // cancellation were ever a hot path; document as future work
        // when that becomes relevant.
        let mut stack = vec![root];
        while let Some(u) = stack.pop() {
            let prev = self.counters[u].swap(usize::MAX, Ordering::AcqRel);
            if prev != usize::MAX {
                if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                    self.notify_done();
                    return;
                }
                for &v in &self.successors[u] {
                    stack.push(v);
                }
            }
        }
    }

    fn notify_done(&self) {
        let _g = self.done_cv.0.lock().unwrap();
        self.done_cv.1.notify_all();
    }
}

impl Graph {
    /// Build per-vertex dispatch closures and stash them on the graph.
    /// Called once, when the graph is registered with an executor via
    /// `ExecutorGraphBuilder::build`. The graph must already live inside
    /// its `Box<Graph>` — closures capture `*const Graph` and rely on
    /// that pointer remaining valid for the graph's lifetime.
    ///
    /// All captures are `Arc::clone`s (refcount-only at build time)
    /// and `Copy` primitives; no per-iteration allocation occurs in
    /// the resulting closures. Required for `REQ_0060`.
    #[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
    pub(crate) fn prepare_dispatch(
        self: &mut Box<Self>,
        task_id: TaskId,
        stop: Stoppable,
        observer: Arc<dyn Observer>,
        monitor: Arc<dyn ExecutionMonitor>,
        err_slot: Arc<Mutex<Option<crate::error::ExecutorError>>>,
    ) {
        let n = self.items.len();
        // SAFETY: we deref through the Box, getting a `*const Graph`
        // that points at the Box's heap allocation. The Box's contents
        // do not move while we hold the Box, so this pointer is stable
        // for the lifetime of `self`. The pointer is shared with every
        // per-vertex closure; the closures access only `&self`-style
        // immutable atomics / Mutex slots on `Graph` (no aliasing
        // mutation through this pointer).
        #[allow(unsafe_code)]
        let graph_ptr = SendGraphPtr(std::ptr::from_ref::<Self>(self.as_ref()));

        let mut jobs: Vec<Box<dyn FnMut() + Send + 'static>> = Vec::with_capacity(n);
        for i in 0..n {
            let task_id = task_id.clone();
            let stop = stop.clone();
            let observer = Arc::clone(&observer);
            let monitor = Arc::clone(&monitor);
            let err_slot = Arc::clone(&err_slot);
            let job: Box<dyn FnMut() + Send + 'static> = Box::new(move || {
                // SAFETY: see SendGraphPtr doc — pointer is stable, no
                // aliasing mutation; pool.barrier() serialises the
                // closure with the executor thread's own graph access.
                #[allow(unsafe_code)]
                let g: &Self = unsafe { &*graph_ptr.get() };

                if g.stop_flag.load(Ordering::Acquire) {
                    g.finalise_skipped(i);
                    return;
                }
                let mut ctx = crate::context::Context::new(&task_id, &stop, observer.as_ref());
                let ptr = g.vertex_ptrs[i].0;
                // SAFETY: vertex_ptrs hold stable raw pointers into the
                // graph's `items` Boxes (see VertexPtr). In-degree
                // counters sequence pool dispatches so at most one
                // thread executes vertex `i` at a time.
                #[allow(unsafe_code)]
                let app_id = unsafe { (*ptr).app_id() };
                #[allow(unsafe_code)]
                let app_inst = unsafe { (*ptr).app_instance_id() };
                if let Some(aid) = app_id {
                    observer.on_app_start(task_id.clone(), aid, app_inst);
                }
                let started = std::time::Instant::now();
                monitor.pre_execute(task_id.clone(), started);
                #[allow(unsafe_code)]
                let res =
                    crate::executor::run_item_catch_unwind_external(unsafe { &mut *ptr }, &mut ctx);
                let took = started.elapsed();
                monitor.post_execute(task_id.clone(), started, took, res.is_ok());
                if let Err(ref e) = res {
                    observer.on_app_error(task_id.clone(), e.as_ref());
                }
                if app_id.is_some() {
                    observer.on_app_stop(task_id.clone());
                }
                match &res {
                    Ok(crate::ControlFlow::Continue) => {}
                    Ok(crate::ControlFlow::StopChain) => {
                        g.stop_chain_seen.store(true, Ordering::Release);
                        g.stop_flag.store(true, Ordering::Release);
                    }
                    Err(_) => g.stop_flag.store(true, Ordering::Release),
                }
                if let Err(e) = res {
                    let mut fe = g.first_err.lock().unwrap();
                    if fe.is_none() {
                        *fe = Some(e);
                    }
                }
                if g.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                    g.notify_done();
                } else if g.stop_flag.load(Ordering::Acquire) {
                    for &j in &g.successors[i] {
                        g.cancel_subtree(j);
                    }
                } else {
                    for &j in &g.successors[i] {
                        if g.counters[j].fetch_sub(1, Ordering::AcqRel) == 1 {
                            // Ring is sized to `next_power_of_two(n)` so
                            // every vertex becoming ready exactly once
                            // fits. If this fires the graph's
                            // accounting is broken.
                            g.ready_ring
                                .push(j)
                                .expect("ready_ring sized to n_vertices");
                        }
                    }
                }
                let _ = &err_slot; // currently unused on the vertex path
                // (errors are bubbled via first_err / GraphRunOutcome)
            });
            jobs.push(job);
        }
        self.vertex_jobs = jobs;
    }

    /// Dispatch this graph once and block until completion. Allocation-free
    /// in the steady state — runtime state was pre-allocated by
    /// `Graph::finish` and per-vertex closures by `prepare_dispatch`.
    /// Required by `REQ_0060`.
    #[allow(unsafe_code)]
    pub(crate) fn run_once_borrowed(&mut self, pool: &Pool) -> GraphRunOutcome {
        let n = self.items.len();

        // Reset per-iteration state in place.
        for (c, d) in self.counters.iter().zip(self.in_degree.iter()) {
            c.store(*d, Ordering::Relaxed);
        }
        self.pending.store(n, Ordering::Relaxed);
        self.stop_flag.store(false, Ordering::Relaxed);
        self.stop_chain_seen.store(false, Ordering::Relaxed);
        *self.first_err.lock().unwrap() = None;
        self.ready_ring.reset();

        // Seed: dispatch every initially-ready vertex (those whose
        // **initial** in-degree is zero). Race-free — `in_degree` is
        // built once at finish() and never mutated, so we can't be
        // tricked by a worker that has already started running root
        // and decremented `counters[succ]` to zero before the seed
        // loop reaches `succ`. Reading `counters[i]` here would race
        // with the worker, redispatching the successor a second time
        // (the worker's own push to `ready_ring` is the legitimate
        // dispatch path).
        for i in 0..n {
            if self.in_degree[i] == 0 {
                self.dispatch_vertex(pool, i);
            }
        }

        // Drain ready_ring until pending hits 0.
        loop {
            while let Some(i) = self.ready_ring.pop() {
                self.dispatch_vertex(pool, i);
            }
            if self.pending.load(Ordering::Acquire) == 0 {
                break;
            }
            let guard = self.done_cv.0.lock().unwrap();
            if self.pending.load(Ordering::Acquire) == 0 {
                drop(guard);
                break;
            }
            drop(
                self.done_cv
                    .1
                    .wait_timeout(guard, std::time::Duration::from_millis(5))
                    .unwrap()
                    .0,
            );
        }
        // Final drain.
        while self.ready_ring.pop().is_some() {}

        let mut first_err = self.first_err.lock().unwrap();
        GraphRunOutcome {
            error: first_err.take(),
            stopped_chain: self.stop_chain_seen.load(Ordering::Acquire),
        }
    }

    /// Submit vertex `i`'s pre-built closure to the pool. Allocation-free
    /// (uses `Pool::submit_borrowed`).
    #[allow(unsafe_code)]
    fn dispatch_vertex(&mut self, pool: &Pool, i: usize) {
        let job_ptr: *mut (dyn FnMut() + Send) =
            std::ptr::from_mut::<dyn FnMut() + Send>(self.vertex_jobs[i].as_mut());
        // SAFETY: closure lives on this Graph, which lives inside
        // `Box<Graph>` inside `TaskEntry`. `pool.barrier()` (called by
        // the WaitSet thread at the end of every callback) ensures the
        // closure has finished executing before the next iteration's
        // callback can touch the graph again. We hold `&mut self`
        // throughout `run_once_borrowed`, so the WaitSet thread is the
        // sole user of the graph state outside of the pool worker's
        // closure invocation.
        unsafe {
            pool.submit_borrowed(crate::pool::BorrowedJob::new(job_ptr));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlFlow, item};

    #[test]
    fn empty_graph_rejected() {
        let b = GraphBuilder::new();
        let err = b.finish().expect_err("empty graph");
        assert!(format!("{err}").contains("no vertices"));
    }

    #[test]
    fn missing_root_rejected() {
        let mut b = GraphBuilder::new();
        b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let err = b.finish().expect_err("missing root");
        assert!(format!("{err}").contains("no root"));
    }

    #[test]
    fn cycle_rejected() {
        let mut b = GraphBuilder::new();
        let a = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let v = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        b.edge(a, v).edge(v, a).root(a);
        let err = b.finish().expect_err("cycle");
        assert!(format!("{err}").contains("cycle"));
    }

    #[test]
    fn unreachable_vertex_rejected() {
        let mut b = GraphBuilder::new();
        let a = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let _orphan = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        b.root(a);
        let err = b.finish().expect_err("unreachable");
        assert!(format!("{err}").contains("reachable"));
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn diamond_graph_builds() {
        let mut b = GraphBuilder::new();
        let r = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let l = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let rt = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        let m = b.vertex(item(|_| Ok(ControlFlow::Continue)));
        b.edge(r, l).edge(r, rt).edge(l, m).edge(rt, m).root(r);
        let g = b.finish().expect("diamond");
        assert_eq!(g.successors[r.0], vec![l.0, rt.0]);
        assert_eq!(g.in_degree[m.0], 2);
    }
}
