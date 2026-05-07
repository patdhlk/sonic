//! Parallel-execution graph: a DAG of [`ExecutableItem`]s rooted at a single
//! vertex whose triggers gate the whole graph.

use crate::error::ExecutorError;
use crate::item::ExecutableItem;
use crate::trigger::{TriggerDecl, TriggerDeclarer};

/// Opaque handle to a graph vertex. Returned by [`GraphBuilder::vertex`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Vertex(pub(crate) usize);

/// Internal graph storage.
#[allow(clippy::redundant_pub_crate)]
pub(crate) struct Graph {
    pub(crate) items: Vec<Box<dyn ExecutableItem>>,
    pub(crate) successors: Vec<Vec<usize>>, // adjacency list
    pub(crate) in_degree: Vec<usize>,       // initial in-degree
    pub(crate) root: usize,
    pub(crate) decls: Vec<TriggerDecl>,
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

        Ok(Graph {
            items: self.items,
            successors,
            in_degree,
            root,
            decls,
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

/// Shared graph dispatch state. Owned via Arc by the `WaitSet` thread and
/// every pool job spawned during a single `run_once` invocation.
struct GraphRuntime {
    items: Vec<VertexPtr>,
    succ: Vec<Vec<usize>>,
    counters: Vec<AtomicUsize>,
    pending: AtomicUsize,
    stop_flag: AtomicBool,
    first_err: Mutex<Option<crate::error::ItemError>>,
    stop_chain_seen: AtomicBool,
    done_cv: (Mutex<()>, Condvar),
    observer: Arc<dyn Observer>,
    monitor: Arc<dyn ExecutionMonitor>,
}

impl GraphRuntime {
    /// Account for a vertex that was skipped (stop already set when it
    /// would have run). Same accounting as `finalise_ran` but no successor
    /// dispatch.
    fn finalise_skipped(self: &Arc<Self>, i: usize) {
        if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.notify_done();
            return;
        }
        for &j in &self.succ[i] {
            self.cancel_subtree(j);
        }
    }

    /// Walk a subtree from `root` and mark every vertex as "done" without
    /// running it. Used when a stop happens and we need to drain pending.
    fn cancel_subtree(self: &Arc<Self>, root: usize) {
        let mut stack = vec![root];
        while let Some(u) = stack.pop() {
            // If counter is already MAX, this vertex was already cancelled.
            let prev = self.counters[u].swap(usize::MAX, Ordering::AcqRel);
            if prev != usize::MAX {
                if self.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                    self.notify_done();
                    return;
                }
                for &v in &self.succ[u] {
                    stack.push(v);
                }
            }
        }
    }

    fn notify_done(self: &Arc<Self>) {
        let _g = self.done_cv.0.lock().unwrap();
        self.done_cv.1.notify_all();
    }
}

impl Graph {
    /// Dispatch this graph once and block until completion.
    ///
    /// Runs on the calling thread (the `WaitSet` thread). Vertices are
    /// dispatched into `pool` and run concurrently; ready successors are
    /// piped back to this thread via a crossbeam channel.
    #[allow(unsafe_code, clippy::too_many_lines)]
    pub(crate) fn run_once(
        &mut self,
        pool: &Arc<Pool>,
        task_id: &TaskId,
        stop: &Stoppable,
        observer: &Arc<dyn Observer>,
        monitor: &Arc<dyn ExecutionMonitor>,
    ) -> GraphRunOutcome {
        let n = self.items.len();
        let counters: Vec<AtomicUsize> = self
            .in_degree
            .iter()
            .map(|d| AtomicUsize::new(*d))
            .collect();

        let runtime = Arc::new(GraphRuntime {
            items: self
                .items
                .iter_mut()
                .map(|b| VertexPtr(std::ptr::from_mut(b.as_mut())))
                .collect(),
            succ: self.successors.clone(),
            counters,
            pending: AtomicUsize::new(n),
            stop_flag: AtomicBool::new(false),
            first_err: Mutex::new(None),
            stop_chain_seen: AtomicBool::new(false),
            done_cv: (Mutex::new(()), Condvar::new()),
            observer: Arc::clone(observer),
            monitor: Arc::clone(monitor),
        });

        // Re-dispatch channel: completed vertex closures push successors
        // that became ready; this thread drains the channel and submits
        // them via `dispatch`.
        let (ready_tx, ready_rx) = crossbeam_channel::unbounded::<usize>();

        let dispatch = {
            let runtime = Arc::clone(&runtime);
            let pool = Arc::clone(pool);
            let task_id = task_id.clone();
            let stop = stop.clone();
            move |i: usize| {
                let runtime = Arc::clone(&runtime);
                let ready_tx = ready_tx.clone();
                let task_id = task_id.clone();
                let stop = stop.clone();
                // Extract app metadata before moving the pointer into the closure.
                // SAFETY: we are on the WaitSet thread and in-degree sequencing
                // ensures no other thread is currently executing vertex `i`.
                let app_id = unsafe { (*runtime.items[i].0).app_id() };
                let app_inst = unsafe { (*runtime.items[i].0).app_instance_id() };
                pool.submit(move || {
                    if runtime.stop_flag.load(Ordering::Acquire) {
                        runtime.finalise_skipped(i);
                        return;
                    }
                    let mut ctx =
                        crate::context::Context::new(&task_id, &stop, runtime.observer.as_ref());
                    let ptr = runtime.items[i].0;
                    if let Some(aid) = app_id {
                        runtime
                            .observer
                            .on_app_start(task_id.clone(), aid, app_inst);
                    }
                    let started = std::time::Instant::now();
                    runtime.monitor.pre_execute(task_id.clone(), started);
                    let res = crate::executor::run_item_catch_unwind_external(
                        // SAFETY: VertexPtr is stable for the duration of run_once
                        // (the Box is not moved). In-degree counters guarantee at
                        // most one concurrent execution of any given vertex.
                        unsafe { &mut *ptr },
                        &mut ctx,
                    );
                    let took = started.elapsed();
                    runtime
                        .monitor
                        .post_execute(task_id.clone(), started, took, res.is_ok());
                    if let Err(ref e) = res {
                        runtime.observer.on_app_error(task_id.clone(), e.as_ref());
                    }
                    if app_id.is_some() {
                        runtime.observer.on_app_stop(task_id.clone());
                    }
                    match &res {
                        Ok(crate::ControlFlow::Continue) => {}
                        Ok(crate::ControlFlow::StopChain) => {
                            runtime.stop_chain_seen.store(true, Ordering::Release);
                            runtime.stop_flag.store(true, Ordering::Release);
                        }
                        Err(_) => runtime.stop_flag.store(true, Ordering::Release),
                    }
                    if let Err(e) = res {
                        let mut g = runtime.first_err.lock().unwrap();
                        if g.is_none() {
                            *g = Some(e);
                        }
                    }
                    if runtime.pending.fetch_sub(1, Ordering::AcqRel) == 1 {
                        runtime.notify_done();
                    } else if runtime.stop_flag.load(Ordering::Acquire) {
                        for &j in &runtime.succ[i] {
                            runtime.cancel_subtree(j);
                        }
                    } else {
                        for &j in &runtime.succ[i] {
                            if runtime.counters[j].fetch_sub(1, Ordering::AcqRel) == 1 {
                                let _ = ready_tx.send(j);
                            }
                        }
                    }
                });
            }
        };

        // Seed: dispatch every initially-ready vertex (in-degree 0). By
        // construction the only such vertex is the root.
        for i in 0..n {
            if runtime.counters[i].load(Ordering::Acquire) == 0 {
                dispatch(i);
            }
        }

        // Drain ready_rx until pending hits 0.
        let (lock, condvar) = &runtime.done_cv;
        loop {
            // Drain whatever ready successors are pending.
            while let Ok(i) = ready_rx.try_recv() {
                dispatch(i);
            }
            // Fast-path: check pending without acquiring the lock.
            if runtime.pending.load(Ordering::Acquire) == 0 {
                break;
            }
            // Slow path: acquire the lock, re-check, then wait briefly.
            // condvar::wait_timeout requires we hold the guard across the
            // call, so we intentionally keep it alive that long.
            let guard = lock.lock().unwrap();
            if runtime.pending.load(Ordering::Acquire) == 0 {
                drop(guard);
                break;
            }
            drop(
                condvar
                    .wait_timeout(guard, std::time::Duration::from_millis(5))
                    .unwrap()
                    .0,
            );
        }
        // Final drain.
        while ready_rx.try_recv().is_ok() {}

        let mut first_err = runtime.first_err.lock().unwrap();
        GraphRunOutcome {
            error: first_err.take(),
            stopped_chain: runtime.stop_chain_seen.load(Ordering::Acquire),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{item, ControlFlow};

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
