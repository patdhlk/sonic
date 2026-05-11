Bounded global allocator — verification
=======================================

Test cases verifying :need:`FEAT_0040`. Each ``test`` directive
``:verifies:`` one of the requirements ``REQ_0300..REQ_0304``.

----

.. test:: Cap exhaustion and oversize alloc both fail-closed
   :id: TEST_0180
   :status: open
   :verifies: REQ_0300, REQ_0301

   **Goal.** Confirm that the allocator (a) returns a non-null
   pointer for the first ``MAX_BLOCKS`` allocations whose
   ``layout.size() ≤ BLOCK_SIZE``, (b) returns null for the
   ``(MAX_BLOCKS + 1)``-th in-bounds allocation, and (c) returns
   null for any allocation whose ``layout.size() > BLOCK_SIZE``
   regardless of how many blocks are free.

   **Fixture.** A ``BoundedAllocator<4, 32>`` instantiated as a
   regular ``static`` (not as ``#[global_allocator]``) so the test
   harness can keep using the system allocator for its own
   bookkeeping (``Vec`` for the captured pointers, ``assert!``
   panics for diagnostics).

   **Steps.**

   1. Call ``alloc(Layout::from_size_align(16, 8))`` four times.
      Each call must return ``Some(ptr)``.
   2. Call once more — must return ``null``.
   3. Free one of the four held pointers via ``dealloc``.
   4. Call ``alloc(16, 8)`` again — must return non-null (a freed
      slot is reused).
   5. Call ``alloc(Layout::from_size_align(64, 8))`` — must return
      null (size exceeds ``BLOCK_SIZE``).
   6. Free all remaining pointers.

   **Expected outcome.** All assertions hold.

.. test:: Steady-state cap behaviour under burst
   :id: TEST_0181
   :status: open
   :verifies: REQ_0300

   **Goal.** Confirm that repeated allocate-then-free cycles never
   leak a block: after ``N`` iterations of
   ``alloc(...).expect(..); dealloc(...)``, every block is free.

   **Fixture.** ``BoundedAllocator<8, 64>``.

   **Steps.**

   1. Run ``for _ in 0..10_000 { let p = alloc(...); dealloc(p); }``.
   2. After the loop, allocate ``MAX_BLOCKS`` blocks back-to-back;
      all calls must return non-null.

   **Expected outcome.** Bitmap returns to an all-free state after
   each balanced ``alloc/dealloc`` pair; capacity is fully
   recoverable.

.. test:: lock() then alloc panics
   :id: TEST_0182
   :status: open
   :verifies: REQ_0302

   **Goal.** Confirm ``lock()`` causes a subsequent ``alloc`` call
   to panic (which, under ``panic = "abort"`` in the consumer
   binary, aborts the process).

   **Fixture.** ``BoundedAllocator<4, 32>`` as a regular ``static``.
   Test harness uses ``#[should_panic]`` so cargo-test detects the
   expected panic and reports the case as passing. The test crate
   does **not** set ``panic = "abort"`` (it stays at the workspace
   default unwind so cargo-test can capture the panic; the
   ``panic = "abort"`` requirement applies to deployed binaries,
   not the test harness).

   **Steps.**

   1. ``alloc(16, 8)`` — should succeed.
   2. ``lock()``.
   3. ``alloc(16, 8)`` — should panic.

   **Expected outcome.** Step 3 panics; cargo-test reports the
   ``#[should_panic]`` case as ``ok``.

.. test:: Counter accuracy
   :id: TEST_0183
   :status: open
   :verifies: REQ_0303

   **Goal.** Confirm ``alloc_count``, ``dealloc_count``, and
   ``peak_blocks_used`` reflect the actual allocation history.

   **Fixture.** ``BoundedAllocator<8, 64>``.

   **Steps.**

   1. Allocate 3 blocks. Read counters: ``alloc_count == 3``,
      ``dealloc_count == 0``, ``peak_blocks_used == 3``.
   2. Free 1 block. ``alloc_count == 3``, ``dealloc_count == 1``,
      ``peak_blocks_used == 3``.
   3. Allocate 2 more (now 4 live). ``alloc_count == 5``,
      ``dealloc_count == 1``, ``peak_blocks_used == 4``.
   4. Free everything. ``alloc_count == 5``, ``dealloc_count == 5``,
      ``peak_blocks_used == 4``.

   **Expected outcome.** All counter reads match the expected
   values exactly.

.. test:: Concurrent alloc/dealloc safety smoke
   :id: TEST_0184
   :status: open
   :verifies: REQ_0304

   **Goal.** Confirm concurrent ``alloc``/``dealloc`` from multiple
   threads neither double-allocates a block nor produces a
   double-free.

   **Fixture.** ``BoundedAllocator<256, 64>``. Four threads each
   doing 1000 iterations of ``alloc + (short delay) + dealloc``.

   **Steps.**

   1. Spawn four threads; each holds onto its pointer briefly
      before freeing, so the bitmap reaches roughly half-full
      under load.
   2. Each thread records the pointer values it received. After
      all threads join, sort the combined list and assert no two
      distinct in-flight allocations ever shared a pointer
      (proxy for "no double-allocation").
   3. After join, ``alloc_count`` and ``dealloc_count`` are both
      ``4 * 1000``; ``peak_blocks_used`` is ``≤ 4``.

   **Expected outcome.** No double-allocation observed; counters
   balance; the allocator is left in the all-free state.

   Optionally — gate a ``loom``-based version of the same scenario
   behind ``--features loom-test`` so a model-checked version of
   the bitmap CAS protocol can be run on demand. Out of scope for
   the initial ``FEAT_0040`` landing.
