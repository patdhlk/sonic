PLC runtime — architecture
==========================

Detailed-design notes for the soft-real-time PLC heart family
(:need:`FEAT_0010`). This page currently covers the **bounded-time
dispatch** sub-feature (:need:`FEAT_0017`) and its zero-allocation
guarantee (:need:`REQ_0060`); other sub-features are added as their
designs land.

Per the arc42 conventions used across this spec, design decisions are
captured as ``arch-decision`` directives, structural elements as
``building-block`` directives, and concrete code mappings as ``impl``
directives. Test cases live in :doc:`../verification/plc-runtime`.

.. contents:: Sections
   :local:
   :depth: 1

----

Solution strategy
-----------------

The dispatch hot path's zero-allocation goal is solved by **moving every
per-iteration allocation up to ``Executor::build`` time** and reusing
that capacity. Two design choices follow from that posture: how to
reuse the per-iteration error slot, and how to replace the unbounded
crossbeam re-dispatch channel that ``Graph::run_once`` allocates today.

.. arch-decision:: Pre-allocate dispatch scratch at Executor::build time
   :id: ADR_0011
   :status: open
   :refines: REQ_0060

   **Context.** Today ``Executor::dispatch_loop`` allocates
   ``Arc<Mutex<Option<ExecutorError>>>`` on every iteration
   (``executor.rs:557-558``) and ``Graph::run_once`` allocates a
   fresh ``Vec<AtomicUsize>`` counter table, a fresh
   ``Arc<GraphRuntime>``, and a fresh
   ``crossbeam_channel::unbounded::<usize>()`` on every dispatch
   (``graph.rs:276-302``). None of those shapes change between
   iterations — vertex count, successor map, and error-channel
   width are fixed once ``Executor::build`` returns.

   **Decision.** Provision all per-iteration scratch at
   ``Executor::build`` time and reset (rather than reallocate) it
   on each tick of the dispatch loop. Concretely: hoist the
   error-capture slot onto ``Executor``, hoist the runtime
   counters / pending counter / successor borrow onto ``Graph``,
   and replace the unbounded re-dispatch channel with a
   hand-rolled bounded SPSC ring whose capacity is
   ``next_power_of_two(n_vertices)`` (see :need:`BB_0023`).

   **Alternatives considered.**

   * *Slab/arena per iteration.* Trades unconditional allocation
     for a slab reset, but slabs still allocate on resize and
     hide cost in the slab implementation. Rejected — the shapes
     are statically known, so a typed pre-allocation is sharper.
   * *Switch to ``smallvec`` everywhere.* Inline storage avoids
     small allocations but spills to the heap on overflow, which
     is non-deterministic — incompatible with a soft-real-time
     guarantee.
   * *Keep ``crossbeam_channel`` but call ``bounded(n)`` once.*
     Bounded crossbeam channels still allocate Arc'd shared state
     at construction, which is acceptable at build time but adds
     an external dependency we do not need on the hot path. A
     hand-rolled SPSC ring is a few dozen lines and removes the
     send-side allocation question entirely.

   **Consequences.**

   ✅ Steady-state dispatch performs zero heap allocations
   (per :need:`REQ_0060`).
   ✅ Worst-case re-dispatch latency is bounded by ring capacity,
   not allocator behaviour.
   ❌ Adds one ``unsafe`` block to ``sonic-executor`` (the SPSC
   ring push/pop), justified by a ``// SAFETY:`` comment and
   covered by ``loom`` tests under feature flag.
   ❌ Vertex count is now an explicit ``Executor::build`` input —
   builders that add vertices after build must rebuild
   (already the case in practice; documented explicitly).

----

Building blocks
---------------

.. building-block:: Dispatch scratch (pre-allocated)
   :id: BB_0023
   :status: open
   :implements: REQ_0060
   :refines: ADR_0011

   The collection of fields hoisted from per-iteration locals onto
   ``Executor`` and ``Graph`` so that dispatch reuses them. Three
   sub-components:

   * **iter_err slot** — single ``Mutex<Option<ExecutorError>>``
     stored on ``Executor``, reset to ``None`` at the start of
     each ``dispatch_loop`` iteration.
   * **Graph runtime fields** — ``counters: Vec<AtomicUsize>``,
     ``pending: AtomicUsize``, ``first_err: Mutex<Option<...>>``,
     ``stop_flag: AtomicBool``, ``stop_chain_seen: AtomicBool``,
     ``done_cv: (Mutex, Condvar)`` — all stored on ``Graph``,
     reset at the top of ``Graph::run_once``. ``self.successors``
     is borrowed rather than cloned.
   * **Re-dispatch SPSC ring** — bounded, ``Box<[AtomicUsize]>``
     of length ``next_power_of_two(n_vertices)``, owned by
     ``Graph``. Producer = pool worker; consumer = WaitSet
     thread. Used to communicate "vertex ``j`` became ready"
     from worker to scheduler without per-iteration allocation.

   Lifetime contract: every field is created in ``Executor::build``
   (or ``Graph::build`` when the executor builds its graphs) and
   lives for the lifetime of the ``Executor``. Reset semantics —
   not deallocation — drive per-iteration state hygiene.

----

Implementation
--------------

.. impl:: Zero-alloc dispatch — executor.rs + graph.rs refactor
   :id: IMPL_0001
   :status: open
   :implements: BB_0023
   :refines: REQ_0060

   Concrete Rust changes that realise :need:`BB_0023`.

   **In ``crates/sonic-executor/src/executor.rs``**

   * Add ``iter_err: Arc<Mutex<Option<ExecutorError>>>`` field on
     ``Executor`` (built once in ``Executor::build``). In
     ``dispatch_loop``, reset to ``None`` at the top of each
     iteration via ``*self.iter_err.lock().unwrap() = None``.
   * Add ``job: Option<Box<dyn FnMut() + Send + 'static>>``
     field on ``TaskEntry``. At ``add`` / ``add_chain`` time
     build the dispatch closure once with stable captures
     (``id``, ``stop``, ``Arc::clone`` of ``observer`` /
     ``monitor`` / ``iter_err``, raw ``SendItemPtr`` or
     ``SendChainPtr``) and store it on the task.
   * In ``dispatch_loop`` the ``Single`` and ``Chain`` arms
     dispatch via ``pool.submit_borrowed(BorrowedJob::new(task
     .job.as_deref_mut().unwrap() as *mut _))`` — no per-iter
     ``Box::new`` allocation.

   **In ``crates/sonic-executor/src/pool.rs``**

   * Generalise the worker job type from ``Box<dyn FnOnce>`` to
     an enum ``Job { Owned(Box<dyn FnOnce>), Borrowed(BorrowedJob)
     }`` so workers can run both styles.
   * Add ``unsafe fn submit_borrowed(&self, BorrowedJob)`` — the
     caller-owned closure path that performs no per-call
     allocation.

   **In ``crates/sonic-executor/src/graph.rs``**

   * Move ``counters``, ``pending``, ``stop_flag``,
     ``stop_chain_seen``, ``first_err``, ``done_cv``,
     ``vertex_ptrs``, and the ready ring from the per-call
     ``Arc<GraphRuntime>`` onto ``Graph`` itself. Reset (don't
     re-allocate) at the top of ``Graph::run_once_borrowed``.
   * Use ``&self.successors`` directly inside per-vertex
     closures via a ``SendGraphPtr`` (a ``*const Graph``
     wrapped in an ``unsafe Send + Sync`` marker).
   * Replace the per-call
     ``crossbeam_channel::unbounded::<usize>()`` with the
     ``ReadyRing`` defined in the new ``ready_ring`` module,
     stored as ``Graph::ready_ring`` and sized at ``finish``
     from ``next_power_of_two(n_vertices.max(2))``.
   * Pre-build one ``Box<dyn FnMut() + Send + 'static>`` per
     vertex in ``Graph::prepare_dispatch``, called by
     ``ExecutorGraphBuilder::build`` once the graph has been
     boxed and stable captures (task_id, stop, observer,
     monitor, err_slot) are known. Closures capture
     ``SendGraphPtr`` plus the per-vertex index.
   * In ``dispatch_loop`` the ``Graph`` arm calls
     ``graph.run_once_borrowed(pool)``; the graph dispatches
     each ready vertex via ``pool.submit_borrowed`` of its
     pre-built closure — no per-vertex ``Box`` per iter.
   * **Seed-loop race fix**: the seed dispatch in
     ``run_once_borrowed`` reads ``self.in_degree[i]``, not
     ``self.counters[i]``, when deciding which vertices to
     dispatch initially. Reading the runtime counter would
     race with the just-dispatched root's worker — if root
     starts running fast enough to decrement
     ``counters[successor]`` to zero before the seed loop
     reaches ``successor``, the seed loop would re-dispatch
     ``successor`` a second time. The worker's own
     ``ready_ring.push`` is the legitimate dispatch path
     for non-root vertices. ``in_degree`` is set once at
     ``finish()`` and never mutated — safe to read in any
     ordering. (Caught by the diamond test under the
     ``submit_borrowed`` path, which dispatches faster than
     the old per-vertex ``Box``-allocating path and so
     exposed the race that had previously been hidden by
     ``Box::new`` latency.)

   **In ``crates/sonic-executor/src/task_kind.rs``**

   * ``TaskKind::Graph(Box<Graph>)`` — Graph must live at a
     stable heap address because per-vertex closures capture
     ``*const Graph``.

   **New module ``crates/sonic-executor/src/ready_ring.rs``**

   * ``pub(crate) struct ReadyRing { buf: Box<[AtomicUsize]>,
     mask: usize, head: AtomicUsize, tail: AtomicUsize }``
     where ``usize::MAX`` is the empty sentinel.
   * ``new(min_capacity) -> Self`` rounds up to the next power
     of two (≥ 2) and pre-fills with the sentinel. One-time
     allocation.
   * ``reset(&self)``, ``push(&self, v) -> Result<(), ()>``,
     ``pop(&self) -> Option<usize>``. Producer side uses
     ``compare_exchange`` on ``tail`` (MPSC); consumer side
     spins briefly on the sentinel value when a slot has been
     reserved but the producer's value-store has not yet
     landed. Allocation-free in steady state.

   **Verification harness**

   * ``crates/sonic-executor/tests/no_alloc_dispatch.rs`` ships
     a hand-rolled counting ``#[global_allocator]`` (no new
     workspace dependency — covers pool worker threads, which
     ``assert_no_alloc``'s thread-local model does not).
     Differential measurement: ``per_iter = (run_n(100) -
     run_n(10)) / (100 - 10)`` separates setup-phase
     allocations from steady-state allocations. See
     :need:`TEST_0170`.
