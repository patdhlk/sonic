.. _safety-fsc:

Functional Safety Concept — Assumed FSRs
========================================

Derived from ASG_0001 / ASG_0002 per ISO 26262-3 §8. Each AFSR carries
the post-decomposition ASIL of Element A: ASIL B(D).

The AFSRs are *assumed* — they describe the item-level functional
safety obligations that an integrator's real item HARA must confirm
(AOU_0006). Sonic's own technical refinement onto its crates lives
in :doc:`tsc`.

.. assumed-fsr:: Spatial Freedom From Interference between integrity levels
   :id: AFSR_0001
   :status: assumed
   :asil: B(D)
   :refines: ASG_0001, ASG_0002

   Sonic shall enforce spatial Freedom From Interference between
   safety-critical hosted items and QM-grade hosted items co-located
   in the same workspace, such that no QM-grade item can mutate the
   address-space or in-memory state observed by a safety-critical
   item.

.. assumed-fsr:: Directional channel topology
   :id: AFSR_0002
   :status: assumed
   :asil: B(D)
   :refines: ASG_0002

   Sonic shall enforce channel directionality on all inter-item
   shared-memory communication, such that a reader of integrity level
   ``L_r`` can only receive data from a writer of integrity level
   ``L_w >= L_r``.

.. assumed-fsr:: Per-integrity-level allocation isolation
   :id: AFSR_0003
   :status: assumed
   :asil: B(D)
   :refines: ASG_0001

   Sonic shall isolate memory-allocation failures between integrity
   levels, such that allocation pressure from QM-grade items cannot
   cause a safety-critical item to encounter an allocation failure
   it would not otherwise have encountered.

.. assumed-fsr:: Internal fault detection and propagation
   :id: AFSR_0004
   :status: assumed
   :asil: B(D)
   :refines: ASG_0001, ASG_0002

   Sonic shall detect and propagate internal framework faults —
   allocator exhaustion, missed deadlines, connector disconnect,
   item panic, channel corruption — to an integrator-observable
   surface within FTTI/2 (at most 50 ms given the assumed 100 ms
   FTTI).

.. assumed-fsr:: Startup integrity verification
   :id: AFSR_0005
   :status: assumed
   :asil: B(D)
   :refines: ASG_0001, ASG_0002

   Sonic shall verify that the spatial-isolation context is intact
   before admitting a safety-critical item into the executor's
   runnable set on each cold start.
