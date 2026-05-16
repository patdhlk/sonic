PLC runtime — verification
==========================

Test cases verifying the PLC runtime heart family (:need:`FEAT_0010`).
Coverage today: the bounded-time dispatch sub-feature
(:need:`FEAT_0017`) and its zero-allocation requirement
(:need:`REQ_0060`); the scan-cycle observability sub-feature
(:need:`FEAT_0021`); and the PREEMPT_RT validation harness sub-feature
(:need:`FEAT_0022`).

----

Zero-allocation dispatch
------------------------

.. test:: Zero allocations in steady-state dispatch
   :id: TEST_0170
   :status: open
   :verifies: REQ_0060

   **Goal.** Confirm that **steady-state** iterations of
   ``Executor::run_n`` perform **zero** heap allocations on
   any thread (WaitSet thread + pool worker threads).
   "Steady-state" excludes the one-time setup that
   ``dispatch_loop`` performs each ``run_n`` entry (WaitSet
   construction, trigger attachment, iceoryx2 lazy init); the
   harness isolates per-iteration allocations from setup
   allocations via a differential measurement.

   **Fixture.** Three executor configurations covering the three
   dispatch paths:

   * ``Executor::builder().worker_threads(0).build()`` + ``add_chain([h, m, t])`` —
     ``TaskKind::Chain`` on the inline pool.
   * ``Executor::builder().worker_threads(2).build()`` + ``add_chain([h, m, t])`` —
     ``TaskKind::Chain`` on the threaded pool.
   * ``Executor::builder().worker_threads(0).build()`` + ``add(single_item)`` —
     ``TaskKind::Single`` on the inline pool.
   * ``Executor::builder().worker_threads(2).build()`` + diamond ``add_graph`` —
     ``TaskKind::Graph`` on the threaded pool (vertex
     dispatch via per-vertex pre-built closures + SPSC
     ring).

   Each item / vertex returns ``Ok(Continue)`` without
   allocating.

   **Allocator instrumentation.** A hand-rolled counting
   ``#[global_allocator]`` (``CountingAllocator``) wraps
   ``std::alloc::System``. Two atomics — ``ALLOC_COUNT`` and
   ``TRACKING`` — are flipped on / off around the measurement
   window. Every thread (including pool workers) increments
   ``ALLOC_COUNT`` on alloc / realloc / alloc_zeroed when
   ``TRACKING`` is set. This covers paths that
   thread-local-flag schemes (``assert_no_alloc``) cannot
   reach.

   **Steps.**

   1. Build the executor; register the task / chain / graph.
   2. ``per_iter_allocs(&mut exec)``:

      a. Warm up with ``run_n(10)`` (untracked) to absorb any
         one-shot lazy init (iceoryx2 service handles
         first-touched on the WaitSet thread, etc.).
      b. Bracket ``run_n(10)`` with the counting allocator
         and record ``a_small``.
      c. Bracket ``run_n(100)`` with the counting allocator
         and record ``a_big``.
      d. Return ``ceil((a_big - a_small) / (100 - 10))`` —
         the average steady-state allocations per dispatch
         iteration, with setup-phase allocations subtracted
         out via the differential.

   3. Assert ``per_iter == 0``.

   4. Repeat for each of the four fixture configurations
      above.

   **Expected outcome.** All four assertions hold:
   ``per_iter == 0``. Test passes under ``cargo test
   -p sonic-executor --test no_alloc_dispatch --release``.

   **Negative case.** ``harness_catches_deliberate_allocation``
   registers a task whose ``execute`` body does
   ``vec![1, 2, 3]`` per iteration and asserts that the
   counting allocator records ``≥ 10`` allocations across 10
   iterations — guards against silent harness regressions
   where the ``#[global_allocator]`` is not actually wired up.

   Lives under
   ``crates/sonic-executor/tests/no_alloc_dispatch.rs``.

----

Scan-cycle observability
------------------------

Test cases verifying the scan-cycle observability sub-feature
(:need:`FEAT_0021`).

.. test:: Histogram percentile accuracy
   :id: TEST_0190
   :status: open
   :verifies: REQ_0100

   **Goal.** Confirm the :need:`ADR_0060` histogram returns p50, p95,
   p99 values within the documented relative-error bound when fed a
   known reference distribution.

   **Fixture.** A standalone unit test in
   ``crates/sonic-executor/src/stats/histogram.rs`` that drives the
   ``Histogram`` directly (no full executor).

   **Steps.**

   1. Build a ``Histogram`` with the production bucket table.
   2. Feed it 10 000 samples drawn from a known distribution
      (uniform on ``[100 ns, 100 ms]`` and exponential with mean
      ``1 ms``).
   3. Compute exact percentile values from the input samples and
      compare to ``Histogram::percentile(q)`` for q ∈ {0.5, 0.95,
      0.99}.
   4. Assert relative error ≤ 1% (bucket centroid bound) for each
      percentile in each distribution.

   **Expected outcome.** All twelve assertions hold (3 quantiles × 2
   distributions × 2 runs for stability).

   Lives under
   ``crates/sonic-executor/src/stats/histogram.rs`` ``#[cfg(test)]``.

.. test:: Per-task max jitter under synthetic period violation
   :id: TEST_0191
   :status: open
   :verifies: REQ_0101

   **Goal.** A synthetic period violation produces the correct
   max-jitter readout.

   **Fixture.** Executor with one cyclic task at 10 ms scan period.
   The task body sleeps for a configurable extra delay on selected
   cycles to induce period jitter.

   **Steps.**

   1. Build executor, register cyclic task with 10 ms period.
   2. Run 100 cycles where the task adds a 3 ms delay on every
      10th cycle.
   3. Query ``Executor::stats_snapshot``; read
      ``per_task[0].max_jitter_ns``.
   4. Assert ``max_jitter_ns ≥ 3 ms - timer-resolution-margin`` and
      ``max_jitter_ns ≤ 3 ms + timer-resolution-margin``.

   **Expected outcome.** Max jitter falls within the expected band.

   Lives under
   ``crates/sonic-executor/tests/cycle_stats_max_jitter.rs``.

.. test:: Overrun counter increments exactly per overrun cycle
   :id: TEST_0192
   :status: open
   :verifies: REQ_0102

   **Goal.** ``overrun_count`` increments exactly once per cycle that
   exceeds the declared scan period, and not at all on cycles within
   the period.

   **Fixture.** Executor with one cyclic task at 10 ms period.

   **Steps.**

   1. Run 50 cycles where the task body completes in 1 ms.
      Assert ``overrun_count == 0``.
   2. Run 30 cycles where the task body deliberately takes 15 ms
      (overrun by 5 ms). Assert ``overrun_count == 30``.
   3. Run 20 more cycles at 1 ms each. Assert
      ``overrun_count == 30`` (no further increments).

   **Expected outcome.** All three assertions hold.

   Lives under
   ``crates/sonic-executor/tests/cycle_stats_overruns.rs``.

.. test:: Push and pull stat paths agree
   :id: TEST_0193
   :status: open
   :verifies: REQ_0103

   **Goal.** Each completed scan cycle delivers exactly one
   ``Observer::on_cycle_stats`` callback, and the aggregate visible
   to ``stats_snapshot`` reflects every observation pushed.

   **Fixture.** Executor with two cyclic tasks (5 ms and 7 ms scan
   periods) and a custom ``Observer`` that records every
   ``on_cycle_stats`` invocation into a thread-safe ring.

   **Steps.**

   1. Run for 200 cycles total.
   2. Assert the recorded callback count matches the number of
      completed scan cycles per task.
   3. Compute the percentile from the recorded raw samples directly;
      compare against
      ``Executor::stats_snapshot().per_task[i].p95_ns`` to within
      the histogram-bucket bound.

   **Expected outcome.** Push and pull paths report consistent
   aggregates.

   Lives under
   ``crates/sonic-executor/tests/cycle_stats_push_pull.rs``.

.. test:: Allocation-free telemetry update
   :id: TEST_0194
   :status: open
   :verifies: REQ_0104

   **Goal.** The per-sample telemetry update path performs zero heap
   allocations under steady state.

   **Fixture.** Reuses the ``CountingAllocator`` from
   :need:`TEST_0170`. Executor with one cyclic task whose body is a
   no-op; the only per-cycle work on the runtime side is the
   telemetry update.

   **Steps.**

   1. Build executor; warm up with ``run_n(10)`` untracked.
   2. ``per_iter_allocs`` differential measurement over ``run_n(10)``
      vs ``run_n(100)``.
   3. Assert ``per_iter == 0``.

   **Negative case.** Replace the no-op task body with a
   ``vec![1, 2, 3]`` allocator-poisoning task; assert
   ``per_iter ≥ 1`` so the harness is verified to actually catch
   allocations.

   **Expected outcome.** Steady-state telemetry update performs zero
   heap allocations.

   Lives under
   ``crates/sonic-executor/tests/no_alloc_cycle_stats.rs``.

----

PREEMPT_RT validation harness
-----------------------------

Test cases verifying the PREEMPT_RT validation harness sub-feature
(:need:`FEAT_0022`). These tests do **not** validate the absolute
jitter envelope — that is a manual procedure per :need:`REQ_0112` and
:need:`ADR_0061`. The tests below verify that the harness itself is
well-formed (it builds, emits valid output, and agrees with the
runtime's own telemetry).

.. test:: Harness builds and runs on Linux non-RT
   :id: TEST_0240
   :status: open
   :verifies: REQ_0111

   **Goal.** The harness binary builds and runs to completion on a
   stock (non-PREEMPT_RT) Linux host without requiring elevated
   capabilities, and produces well-formed NDJSON on stdout.

   **Fixture.** GitHub Actions Linux x86_64 runner; the harness is
   built with ``cargo build --release -p xtask-preempt-rt``.

   **Steps.**

   1. Build the harness in release mode.
   2. Run ``cargo run --release -p xtask-preempt-rt --
      --load-profile idle --cycle-count 1000 --task-count 1
      --scan-period-us 1000``.
   3. Capture stdout; assert each line parses as JSON and contains
      the expected keys (``ts_ns``, ``task_id``, ``period_ns``,
      ``actual_period_ns``, ``jitter_ns``, ``took_ns``).
   4. Assert the captured line count equals ``cycle-count``.

   **Expected outcome.** Smoke run succeeds; output is well-formed.

   Lives under ``xtask/preempt-rt/tests/smoke.rs``.

.. test:: NDJSON schema validation
   :id: TEST_0241
   :status: open
   :verifies: REQ_0111

   **Goal.** The harness output conforms exactly to the documented
   NDJSON schema; no extra keys, no missing keys, correct value
   types.

   **Fixture.** An in-tree JSON Schema file
   (``xtask/preempt-rt/schema/cycle-observation.schema.json``)
   describes the record shape from :need:`REQ_0111`.

   **Steps.**

   1. Run a short harness invocation (100 cycles).
   2. Validate every output line against the schema using a
      lightweight in-tree validator (no new workspace dep — match
      keys + value-type assertions manually).
   3. Assert all 100 lines validate.

   **Expected outcome.** Output is schema-conformant.

   Lives under ``xtask/preempt-rt/tests/schema.rs``.

.. test:: Harness telemetry agrees with stats_snapshot
   :id: TEST_0242
   :status: open
   :verifies: REQ_0113

   **Goal.** The NDJSON cycle observations produced by the harness
   agree with ``Executor::stats_snapshot`` aggregates taken at the
   end of the run — i.e. the harness and the pull API see the same
   underlying data.

   **Fixture.** A test variant of the harness that, after writing
   its last NDJSON line, also writes a single ``StatsSnapshot`` JSON
   record to stderr.

   **Steps.**

   1. Run 1000 cycles with one cyclic task.
   2. Compute the percentile from the NDJSON ``took_ns`` column
      directly.
   3. Compare against the matching field in the stderr
      ``StatsSnapshot`` record.
   4. Assert agreement within the histogram-bucket bound (~1%).

   **Expected outcome.** Push and pull paths agree on the same data.

   Lives under ``xtask/preempt-rt/tests/push_pull_agreement.rs``.
