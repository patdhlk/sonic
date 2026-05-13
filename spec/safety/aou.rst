.. _safety-aou:

Assumptions of Use
==================

The SEooC contract with the integrator. Each AoU is a claim sonic
*makes* about the integrator's environment or process. The integrator
MUST validate every AoU before claiming any ASIL for a sonic-hosted
item.

.. aou:: Diverse Element B monitor at ASIL B(D)
   :id: AOU_0001
   :status: open

   The integrator supplies a diverse, independent **Element B monitor**
   of equivalent ASIL B(D) capability that observes sonic's outputs and
   forces safe state on detected omission or value failure.

   :Validates: Decomposition (:doc:`decomposition`)

.. aou:: Independence between Element A and Element B
   :id: AOU_0002
   :status: open

   Element A (sonic) and Element B (monitor) run on independent CPU
   cores or independent SoCs, with independent power and clock
   domains where feasible.

   :Validates: Independence per ISO 26262-9 §5.4.4

.. aou:: Heartbeat receiver and safe-state path
   :id: AOU_0003
   :status: open

   The integrator implements the **receiver side** of sonic's heartbeat
   protocol and the safe-state forcing path with reaction time at most
   ``FTTI − sonic's emission period`` (at most 50 ms given FTTI=100 ms
   and heartbeat period ≤ FTTI/2).

   :Validates: :need:`TSR_0010`

.. aou:: Host OS provides MMU isolation and deterministic scheduling
   :id: AOU_0004
   :status: open

   The host OS provides MMU-enforced address-space isolation between
   processes and a deterministic scheduling discipline (real-time class
   or deadline-based scheduling).

   :Validates: :need:`TSR_0003`, :need:`TSR_0009`

.. aou:: Real-time scheduling and CPU pinning for SC process
   :id: AOU_0005
   :status: open

   The integrator pins the SC process to dedicated CPU core(s) and
   configures it under SCHED_FIFO or SCHED_DEADLINE; QM processes are
   excluded from those cores.

   :Validates: Temporal FFI

.. aou:: Integrator confirms HARA inputs and FTTI
   :id: AOU_0006
   :status: open

   The integrator validates that the assumed hazards (:need:`AHZ_0001`,
   :need:`AHZ_0002`) and assumed safety goals (:need:`ASG_0001`,
   :need:`ASG_0002`) match the result of their own HARA. The FTTI of
   100 ms is confirmed or replaced.

   :Validates: Whole concept

.. aou:: Integrator owns safe-state semantics
   :id: AOU_0007
   :status: open

   The integrator's application logic enters a defined safe state on
   receipt of ``HealthEvent::Faulted`` or on absence of expected channel
   data within deadline. Sonic raises faults; it does not define what
   safe state means for any particular application.

   :Validates: :need:`AFSR_0004`

.. aou:: Integrator unsafe-Rust discipline
   :id: AOU_0008
   :status: open

   The integrator's own ``ExecutableItem`` implementations use
   ``unsafe`` Rust only in ways that do not violate spatial isolation
   invariants — no aliasing of channel handles, no escape of writable
   references across integrity-level boundaries, no shared mutable
   state outside iceoryx2 channels.

   :Validates: Spatial FFI

.. aou:: Lower-stack qualification at ASIL B(D)
   :id: AOU_0009
   :status: open

   The integrator confirms that the host OS kernel, libc, iceoryx2
   runtime, and Rust toolchain are qualified to at least ASIL B(D).
   Sonic does not qualify these — they sit below sonic in the stack.

   :Validates: Whole stack
