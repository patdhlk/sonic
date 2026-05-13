.. _safety-tsc:

Technical Safety Concept — TSRs
===============================

Refinement of the AFSRs (see :doc:`fsc`) onto sonic's concrete crates.
TSRs are sonic's own commitments — not assumed. They take the ASIL of
their parent AFSR (B(D)).

Each TSR carries a **status** field describing today's implementation
state, with the convention:

* ``implemented`` — current code satisfies (references concrete FEAT/REQ/BB IDs).
* ``draft`` — requires extension to existing crate (gap analysis pending).

.. tsr:: Bounded allocator hard caps
   :id: TSR_0001
   :status: implemented
   :asil: B(D)
   :refines: AFSR_0003

   The bounded allocator (``sonic-bounded-alloc``) shall enforce hard
   compile-time caps on per-allocation size and total live blocks;
   allocation requests exceeding the cap shall return null per the
   ``core::alloc::GlobalAlloc`` contract.

   :Allocates to: ``sonic-bounded-alloc``
   :Today: Satisfied by FEAT_0040.

.. tsr:: Per-integrity-level allocation quotas
   :id: TSR_0002
   :status: draft
   :asil: B(D)
   :refines: AFSR_0003

   The bounded allocator shall maintain partitioned quotas per
   integrity level, such that exhaustion of the QM-grade pool cannot
   deny allocation from the safety-critical pool.

   :Allocates to: ``sonic-bounded-alloc``
   :Today: EXT — current allocator has a single global pool. Requires
       API extension to take an integrity-level argument at allocator-init.

.. tsr:: Integrity-level declaration and process isolation
   :id: TSR_0003
   :status: draft
   :asil: B(D)
   :refines: AFSR_0001

   Each ``ExecutableItem`` registration shall declare an integrity
   level (``SafetyCritical`` | ``QualityManaged``); the executor shall
   reject in-process co-hosting of mixed integrity levels and require
   QM-grade items to run in a separate OS process.

   :Allocates to: ``sonic-executor``
   :Today: NEW — neither the trait nor the registration API today
       carries an integrity-level field.

.. tsr:: Missed-deadline detection within one cycle
   :id: TSR_0004
   :status: implemented
   :asil: B(D)
   :refines: AFSR_0004

   Missed-deadline detection shall fire within one cycle of the
   configured interval and propagate via ``ExecutionMonitor``.

   :Allocates to: ``sonic-executor``
   :Today: Satisfied by the executor's existing deadline monitor.

.. tsr:: Compile-time channel directionality
   :id: TSR_0005
   :status: implemented
   :asil: B(D)
   :refines: AFSR_0002

   The ``ChannelWriter`` / ``ChannelReader`` types shall enforce
   direction at compile time via the Rust type system; runtime
   construction shall not be able to forge a writer from a reader
   handle.

   :Allocates to: ``sonic-connector-host``, ``sonic-connector-core``
   :Today: Satisfied by BB_0001, BB_0005.

.. tsr:: Bounded health-event latency
   :id: TSR_0006
   :status: implemented
   :asil: B(D)
   :refines: AFSR_0004

   ``ConnectorHealth`` events shall be emitted within FTTI/2 (at most
   50 ms) of a connector state transition
   (Healthy → Degraded → Faulted).

   :Allocates to: ``sonic-connector-host``, ``sonic-connector-zenoh``
   :Today: Satisfied by REQ_0440..REQ_0444.

.. tsr:: Single-publisher iceoryx2 topology for safety-critical channels
   :id: TSR_0007
   :status: implemented
   :asil: B(D)
   :refines: AFSR_0002

   iceoryx2 services backing safety-critical channels shall be
   configured with single-publisher topology; the publisher process
   holds the only write capability over the underlying shared-memory
   segment.

   :Allocates to: ``sonic-connector-transport-iox``
   :Today: Single-publisher is the iceoryx2 default for PublishSubscribe
       services; the transport-iox factory does not override.

.. tsr:: Envelope sequence + CRC integrity
   :id: TSR_0008
   :status: draft
   :asil: B(D)
   :refines: AFSR_0002, AFSR_0004

   The ``ConnectorEnvelope`` POD wire format shall carry a sequence
   counter and a CRC over header + payload; CRC mismatch on read
   shall raise a ``HealthEvent`` and discard the frame without
   surfacing it to the reader.

   :Allocates to: ``sonic-connector-transport-iox``
   :Today: EXT — current ``ConnectorEnvelope<N>`` carries a
       ``CorrelationId`` but no sequence counter or CRC.

.. tsr:: Cross-process hosting mode
   :id: TSR_0009
   :status: draft
   :asil: B(D)
   :refines: AFSR_0001, AFSR_0002

   Sonic shall provide a hosting mode in which safety-critical items
   and QM-grade items run in distinct OS processes communicating
   exclusively through iceoryx2 shared-memory channels with per-process
   read/write capability.

   :Allocates to: ``sonic-executor``, ``sonic-connector-host``
   :Today: NEW — current executor hosts all items in one process.

.. tsr:: Heartbeat for Element B monitor
   :id: TSR_0010
   :status: draft
   :asil: B(D)
   :refines: AFSR_0004

   The safety-critical executor process shall emit a heartbeat
   ``HealthEvent`` at a period at most FTTI/2 (50 ms) to support the
   integrator's diverse monitor (Element B per :doc:`decomposition`).

   :Allocates to: ``sonic-executor``, ``sonic-connector-host``
   :Today: NEW — no liveness heartbeat exists today.

TSR coverage summary
--------------------

.. needtable::
   :types: tsr
   :columns: id, title, status, refines
   :show_filters:

* 5 ``implemented`` — TSR_0001, TSR_0004, TSR_0005, TSR_0006, TSR_0007.
* 2 ``draft`` (extension to existing crate) — TSR_0002, TSR_0008.
* 3 ``draft`` (new component) — TSR_0003, TSR_0009, TSR_0010.

**AFSR coverage.** This batch refines AFSR_0001..AFSR_0004 onto 10 TSRs.
AFSR_0005 (startup integrity verification) is intentionally deferred to
the follow-on implementation plan that owns TSR_0003 (process
isolation), since startup verification is the natural admission-time
companion to the process-isolation invariant.

The five ``draft`` TSRs are the substance of the context-based isolation
work item and are the subject of a follow-on sonic implementation plan.
