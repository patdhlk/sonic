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

----

Scan-cycle observability
------------------------

Detailed design for the **scan-cycle observability** sub-feature
(:need:`FEAT_0021`). Two structural pieces: a fixed-bucket histogram
for percentile estimation (chosen for its allocation-free, bounded-time
per-sample update path), and per-task aggregate slots allocated at
``Executor::build`` time.

.. arch-decision:: Fixed-bucket histogram for percentile estimation
   :id: ADR_0060
   :status: open
   :refines: REQ_0100

   **Context.** :need:`REQ_0100` requires p50 / p95 / p99
   execute-duration percentiles per task over a sliding window, and
   :need:`REQ_0104` requires the update path to be allocation-free with
   bounded per-sample latency. A window-of-raw-samples approach (keep
   the last N samples, sort on query) is allocation-free if N is fixed
   at build time but pays O(N log N) on every query. Streaming sketches
   (t-digest, CKMS) give tight p99 accuracy but their compaction step
   is amortised, not bounded, and they reshape memory as data arrives.

   **Decision.** Use a fixed-bucket log-linear histogram covering the
   value range 100 ns … 10 s with at least three buckets per decade
   (eight decades × three buckets ≈ 24 active buckets, padded to a
   power of two for cheap indexing). The bucket layout is fixed at
   compile time as a ``const`` table; the per-sample update is a
   ``log2``-style index computation plus an atomic increment.
   Percentile queries scan the bucket array in O(B) where B is
   constant (~32). Sliding-window behaviour is implemented as a small
   ring of histogram snapshots (size = window-count divided by
   snapshot period); ageing-out is a snapshot subtraction.

   **Alternatives considered.**

   * *Exact sliding window of raw samples.* Allocation-free if the
     ring is pre-allocated, but percentile query is O(N log N) and
     the ring must be sized for the worst case (~1 MB per task at
     100 k samples vs ~1 kB for the histogram). Rejected for memory
     pressure under many-task configurations.
   * *t-digest / CKMS streaming sketch.* Tighter p99 accuracy but
     compaction is amortised; worst-case per-sample latency is not
     bounded. Rejected because the per-sample update is on the
     dispatch hot path.

   **Consequences.**

   ✅ Per-sample update is O(1) and allocation-free
   (per :need:`REQ_0104`).
   ✅ Per-task memory footprint is bounded and known at build time
   (~1 kB / task for the histogram + snapshots).
   ❌ Percentile values are bucket-quantised — relative accuracy is
   bounded by bucket width (~33% within a single bucket, ≤ 1% at
   the bucket centroid). Acceptable for soft-RT telemetry; the
   :need:`REQ_0111` harness exposes raw samples for finer offline
   analysis when needed.

.. building-block:: Per-task cycle statistics
   :id: BB_0050
   :status: open
   :implements: REQ_0100
   :refines: ADR_0060

   ``CycleStats`` — per-task statistics owned by ``Executor``,
   allocated once at ``Executor::build`` time. Three fields:

   * ``hist: Histogram`` — fixed-bucket histogram of execute durations
     per :need:`ADR_0060`.
   * ``max_jitter_ns: AtomicU64`` — windowed maximum of
     ``|actual_period - declared_period|`` (per :need:`REQ_0101`).
   * ``overrun_count: AtomicU64`` — monotonic counter, incremented when
     a scan-cycle exceeds the declared period (per :need:`REQ_0102`).

   One ``CycleStats`` per registered task; the array is sized at
   ``Executor::build``. Update paths use relaxed atomic stores so
   workers do not synchronise on the stats field.

.. building-block:: Statistics snapshot view
   :id: BB_0051
   :status: open
   :implements: REQ_0103
   :refines: ADR_0060

   ``StatsSnapshot`` — borrowed view returned by the pull API
   (``Executor::stats_snapshot``). Per-task entries carry
   ``{ task_id, p50_ns, p95_ns, p99_ns, max_jitter_ns,
   overrun_count }`` computed from the matching :need:`BB_0050` at
   the moment of the call. The snapshot itself is a thin slice over
   pre-allocated buffers on ``Executor``; the caller may clone it for
   off-stack consumption but the runtime side never allocates.

.. impl:: Stats module — sonic-executor/src/stats/
   :id: IMPL_0070
   :status: open
   :implements: BB_0050, BB_0051
   :refines: REQ_0100

   Concrete Rust changes that realise :need:`BB_0050` and
   :need:`BB_0051`.

   **New module ``crates/sonic-executor/src/stats/``**

   * ``mod.rs`` — public re-exports (``CycleStats``,
     ``CycleObservation``, ``StatsSnapshot``).
   * ``histogram.rs`` — ``Histogram`` with the fixed bucket table
     from :need:`ADR_0060`. Public API: ``record(value_ns)``,
     ``percentile(q: f32) -> u64``. The record path is ``#[inline]``
     and contains no allocation (verified by :need:`TEST_0194`).
   * ``cycle.rs`` — ``CycleStats`` struct plus the
     ``CycleObservation { task_id, period_ns, actual_period_ns,
     jitter_ns, took_ns }`` value type carried by
     ``on_cycle_stats``.

   **In ``crates/sonic-executor/src/observer.rs``**

   * Extend ``Observer`` with a default-method
     ``fn on_cycle_stats(&self, _: &CycleObservation) {}`` — the
     default no-op preserves backward compatibility for existing
     ``Observer`` implementations.

   **In ``crates/sonic-executor/src/executor.rs``**

   * Add a ``Vec<CycleStats>`` field on ``Executor``, sized at
     ``build`` time from the registered-task count. Pre-allocate per
     :need:`REQ_0060`.
   * In the ``dispatch_loop`` post-execute integration: record
     ``took`` into ``CycleStats[task].hist``, compute
     ``period_jitter`` against the task's declared scan period,
     update ``max_jitter_ns`` via ``fetch_max``, increment
     ``overrun_count`` if ``took > period``, then call
     ``observer.on_cycle_stats(&obs)``.
   * Add public ``Executor::stats_snapshot(&self) -> StatsSnapshot``
     that walks ``self.cycle_stats`` and emits a snapshot.

   **Verification**

   * Histogram accuracy — :need:`TEST_0190`.
   * Jitter readout — :need:`TEST_0191`.
   * Overrun counter — :need:`TEST_0192`.
   * Push/pull contract — :need:`TEST_0193`.
   * Allocation-free update — :need:`TEST_0194`.

----

PREEMPT_RT validation harness
-----------------------------

Detailed design for the **PREEMPT_RT validation harness** sub-feature
(:need:`FEAT_0022`). The harness is packaged as an out-of-tree cargo
bin and consumes the :need:`FEAT_0021` telemetry push channel as its
sole measurement path.

.. arch-decision:: Harness as xtask, not CI gate
   :id: ADR_0061
   :status: open
   :refines: REQ_0112

   **Context.** :need:`REQ_0110` requires a documented worst-case
   jitter envelope. The natural ASPICE / industrial pattern is to wire
   a benchmark gate into CI so regressions block merge. Cloud
   GitHub-hosted runners do not run PREEMPT_RT and cannot be made to
   do so without self-hosting. A self-hosted PREEMPT_RT runner for a
   single-maintainer personal project carries ongoing infra cost
   (host availability, kernel updates, runner-agent updates).

   **Decision.** Package the harness as an out-of-tree cargo bin
   under ``xtask/preempt-rt/`` and document a manual reproduction
   procedure (per :need:`REQ_0112`). Do not gate CI on jitter
   measurements. The envelope artifact (:need:`REQ_0110`) is updated
   manually after a measurement run.

   **Alternatives considered.**

   * *Self-hosted PREEMPT_RT runner with auto-gate.* Captures
     regressions automatically but introduces a single-point-of-
     failure infra dependency. Rejected for the current
     single-maintainer setup; revisitable once the project has
     persistent infrastructure.
   * *Scheduled (nightly) run on self-hosted runner.* Same infra
     dependency as the auto-gate, with slower regression detection.
     Rejected for the same reason.
   * *Run ``cyclictest`` only, no harness.* Loses the link between
     measurements and the ``sonic-executor`` dispatch path. Rejected
     because the relevant question is "what jitter does sonic add on
     top of the kernel?", which ``cyclictest`` alone cannot answer.

   **Consequences.**

   ✅ Zero ongoing infra cost; runs are on-demand by the maintainer.
   ✅ The harness path is identical to the production telemetry path
   (per :need:`REQ_0113`), so the manual run is representative of
   production behaviour.
   ❌ Regressions can land between manual runs. Mitigated partly by
   :need:`TEST_0194` (allocation-free telemetry update) and
   :need:`TEST_0192` (overrun counter correctness) staying in regular
   CI; what the harness uniquely validates is the *absolute envelope*,
   not behavioural correctness.

.. building-block:: xtask-preempt-rt harness
   :id: BB_0052
   :status: open
   :implements: REQ_0111
   :refines: ADR_0061

   Workspace member ``xtask-preempt-rt`` — a cargo bin that
   constructs a representative ``Executor``, runs it for a configurable
   number of scan cycles, and writes ``CycleObservation`` records to
   stdout as NDJSON.

   CLI shape:

   .. code-block:: text

      cargo xtask preempt-rt-bench \
          --load-profile {idle,cpu-stress,cyclictest-coexist} \
          --cycle-count <N> \
          --task-count <K> \
          --scan-period-us <P>

   The harness installs a custom ``Observer`` implementation whose
   ``on_cycle_stats`` writes one NDJSON line per call. No timing
   measurements are taken outside the ``Observer`` callback
   (per :need:`REQ_0113`).

.. impl:: xtask-preempt-rt — crate layout and procedure doc
   :id: IMPL_0071
   :status: open
   :implements: BB_0052
   :refines: REQ_0111

   **New workspace member ``xtask/preempt-rt/``**

   * ``Cargo.toml`` — depends on ``sonic-executor`` plus minimal
     transitive crates. Not a default workspace build target.
   * ``src/main.rs`` — argument parsing (``clap``), executor
     construction, ``Observer`` wiring, run loop.
   * ``src/workload.rs`` — load-profile fixtures
     (``idle``, ``cpu-stress``, ``cyclictest-coexist``).
     ``cpu-stress`` spawns ``stress-ng``; ``cyclictest-coexist`` prints
     a copy-paste ``cyclictest`` command and waits for the operator.
   * ``src/ndjson.rs`` — minimal NDJSON writer (no ``serde_json``
     dependency to keep the harness's own jitter low).

   **New document ``docs/preempt-rt-procedure.md``** (deferred to
   the implementation phase — written when the first measurement run
   is staged so the procedure can reflect the actual host).

   Sections planned:

   * Prerequisites — Debian / Ubuntu host with
     ``linux-image-rt-amd64`` or equivalent, ``stress-ng``,
     ``rt-tests``.
   * Kernel configuration — ``CONFIG_PREEMPT_RT=y`` verification,
     boot-line flags (``isolcpus=2,3``, ``nohz_full=2,3``,
     ``rcu_nocbs=2,3``).
   * Capability and pinning — ``CAP_SYS_NICE`` requirement for
     ``SCHED_FIFO`` (per :need:`REQ_0041`).
   * Reproducing the envelope — sample command line for each load
     profile.
   * Updating the envelope artifact — how to incorporate fresh
     measurements into :need:`REQ_0110`'s versioned document.

   **Verification**

   * Build + smoke run — :need:`TEST_0240`.
   * NDJSON schema — :need:`TEST_0241`.
   * Push/pull agreement — :need:`TEST_0242`.
