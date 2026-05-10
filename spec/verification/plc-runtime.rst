PLC runtime — verification
==========================

Test cases verifying the PLC runtime heart family (:need:`FEAT_0010`).
Currently scoped to the bounded-time dispatch sub-feature
(:need:`FEAT_0017`) and its zero-allocation requirement
(:need:`REQ_0060`).

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
