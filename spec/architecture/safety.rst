.. _architecture-safety:

Safety architecture decisions
=============================

Architecture decisions supporting the SEooC safety concept (see
:doc:`../safety/index`).

.. arch-decision:: Process boundary as spatial isolation context
   :id: ADR_0050
   :status: open
   :refines: AFSR_0001, AFSR_0002
   :satisfies: TSR_0003, TSR_0009

   **Context.** Sonic-hosted items at different integrity levels
   (safety-critical and QM-grade) must coexist in the same workspace
   without QM code being able to corrupt safety-critical state. The
   spatial Freedom From Interference argument has to hold without
   certifying the entire stack (host OS, libc, every transitive
   dependency) to ASIL B(D).

   **Decision.** Adopt OS process boundaries as the unit of spatial
   isolation between safety-critical and QM-grade hosted code. Cross-
   boundary communication is exclusively via iceoryx2 shared-memory
   channels with per-process read/write capability.

   **Alternatives considered.**

   * *Single-process with Rust-level isolation only.* Rejected —
     ``unsafe`` in any QM-grade dependency invalidates the FFI
     argument.
   * *Hardware-enforced page-table-swap contexts à la NVIDIA's ELISA
     proposal.* Rejected — applicable only in kernel space; userspace
     processes give us the same isolation for free via OS MMU
     enforcement.
   * *Hypervisor partitioning.* Rejected — too heavyweight for the
     target SoC class and forces integrators onto specific hypervisors.

   **Consequences.** Every SC↔QM call becomes an iceoryx2 channel
   hop. Classification of which crates live inside the SC process
   becomes load-bearing; per-crate integrity-level tags in
   ``Cargo.toml`` metadata are a natural follow-on.

.. arch-decision:: Bounded allocator as spatial-determinism anchor
   :id: ADR_0051
   :status: open
   :refines: AFSR_0003
   :satisfies: TSR_0001, TSR_0002

   **Context.** Safety-critical hosted items at ASIL B(D) need
   deterministic memory-allocation behavior to meet their FTTI bound
   and must not be denied allocations by allocation pressure from
   QM-grade items. Standard system allocators don't provide either
   guarantee.

   **Decision.** All allocation by safety-critical hosted code goes
   through ``sonic-bounded-alloc`` with compile-time-declared
   per-integrity-level quotas.

   **Alternatives considered.**

   * *Standard heap with OOM-killer.* Rejected — violates FTTI bound
     and admits QM-side pressure as a denial mechanism.
   * *Arena-per-task.* Rejected — adds API complexity without solving
     cross-process partitioning.
   * *``no_std`` + stack-only.* Rejected — too restrictive for
     realistic application code.

   **Consequences.** Caps must be sized at build time; growing past
   the cap requires a rebuild. Partitioned pools (:need:`TSR_0002`)
   require extending ``sonic-bounded-alloc``'s public API to take an
   integrity-level argument at the allocator-init macro.
