Bounded global allocator
========================

Workspace infrastructure providing a static, pre-allocated, fixed-block
global allocator for sonic binaries that must give compile-time
guarantees on memory usage. The crate (``sonic-bounded-alloc``) is
independent of ``sonic-executor`` but composes naturally with it —
:need:`REQ_0060` (zero-alloc steady-state dispatch) is *tested* by a
counting allocator and *enforced* in deployed binaries by registering
this crate's allocator as ``#[global_allocator]``.

.. contents:: Sections
   :local:
   :depth: 1

----

Umbrella feature
----------------

.. feat:: Bounded global allocator
   :id: FEAT_0040
   :status: open

   A reusable ``#[global_allocator]`` implementation that draws every
   allocation from a statically-sized, pre-allocated arena with hard
   caps on both per-allocation size and total live block count.
   Returns null when either cap is reached, so allocation failure is
   bounded and observable rather than producing OOM-driven address
   space growth. An optional "lock-after-init" mode causes any
   allocation attempt after a designated point in the program to
   panic immediately, making the steady-state zero-allocation
   invariant required by :need:`REQ_0060` enforceable at runtime in
   deployed binaries.

Requirements
------------

.. req:: Pre-allocated fixed-block arena
   :id: REQ_0300
   :status: open
   :satisfies: FEAT_0040

   The allocator shall serve every allocation from a single
   statically-sized arena whose total capacity is
   ``MAX_BLOCKS * BLOCK_SIZE`` bytes. ``MAX_BLOCKS`` and
   ``BLOCK_SIZE`` shall be compile-time const generics on the
   allocator type; the arena and the allocator's own bookkeeping
   (free bitmap, counters, lock flag) shall live in ``static``
   storage and shall not themselves invoke any heap allocation.
   The allocator shall not grow the arena at runtime.

.. req:: Fail-closed on cap overrun
   :id: REQ_0301
   :status: open
   :satisfies: FEAT_0040

   When an allocation request cannot be satisfied — because
   ``layout.size() > BLOCK_SIZE``, ``layout.align() > BLOCK_SIZE``
   (the per-block alignment derives from ``BLOCK_SIZE``), or the
   bitmap has no free block — the allocator's ``alloc`` method
   shall return a null pointer. Rust's default
   ``alloc_error_handler`` shall thereby abort the process,
   producing a fail-closed outcome rather than undefined behaviour
   or silent fallback to the system allocator.

.. req:: Lock-after-init panic mode
   :id: REQ_0302
   :status: open
   :satisfies: FEAT_0040

   The allocator shall expose a ``lock(&self)`` method that flips an
   internal ``AtomicBool`` (Release ordering). After ``lock`` returns,
   every subsequent call to ``alloc`` (including ``alloc_zeroed`` and
   ``realloc``) shall panic immediately rather than returning a
   pointer. Binaries using this mode shall configure ``panic =
   "abort"`` in their Cargo profile so the panic itself does not
   attempt to allocate the unwinder's payload string. The lock is
   one-way; there is no ``unlock`` method.

.. req:: Allocation accounting API
   :id: REQ_0303
   :status: open
   :satisfies: FEAT_0040

   The allocator shall expose public methods returning live counts:
   total successful ``alloc`` calls since process start, total
   successful ``dealloc`` calls since process start, and the
   high-water-mark of simultaneously-live blocks. These counts shall
   be maintained even when the allocator is registered as
   ``#[global_allocator]``, so test harnesses and observability tools
   can read steady-state allocation activity without requiring a
   parallel instrumentation layer.

.. req:: Thread-safe allocation
   :id: REQ_0304
   :status: open
   :satisfies: FEAT_0040

   Concurrent ``alloc`` and ``dealloc`` calls from multiple threads
   shall be safe (no torn state, no double-allocation of a block,
   no double-free) under the standard Rust ``GlobalAlloc`` contract.
   The bitmap shall use ``AtomicU64`` words with
   compare-exchange-based allocation; the counters shall use
   appropriate atomic ordering; the lock flag shall use Acquire on
   read and Release on write.

Safety refinements
------------------

The bounded allocator implements safety obligations :need:`TSR_0001`
and :need:`TSR_0002` derived from the SEooC safety concept (see
:doc:`../safety/tsc`).

* :need:`TSR_0001` (hard caps on per-allocation size and total live
  blocks) is **implemented** today by :need:`FEAT_0040`.

* :need:`TSR_0002` (partitioned per-integrity-level quota pools) is
  **draft** — requires extending the public API to take an
  integrity-level argument at the allocator-init macro. See
  :need:`ADR_0051` for the architectural rationale.
