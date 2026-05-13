.. _safety-hara:

Assumed HARA — Hazards and Safety Goals
=======================================

Illustrative Hazard Analysis and Risk Assessment driving the FFI
argument. Integrators MUST run their own HARA per ISO 26262-3 §6;
the entries below are *assumed* inputs (AOU_0006).

Assumed Hazards
---------------

E/S/C ratings are illustrative for a typical electromechanical
actuator. ASIL is determined per ISO 26262-3 Table 4.

.. assumed-hazard:: Loss of cyclic safety-critical command
   :id: AHZ_0001
   :status: assumed
   :asil: D

   Control loop silently halted by QM-grade subsystem corrupting
   executor state in the same address space.

   :Exposure: E4 (high probability — most operating situations)
   :Severity: S3 (life-threatening to fatal injuries)
   :Controllability: C3 (difficult to control or uncontrollable)
   :ASIL: D

.. assumed-hazard:: Erroneous safety-critical command
   :id: AHZ_0002
   :status: assumed
   :asil: D

   Output computed from corrupted shared-memory channel written by a
   QM-grade subsystem (stray pointer, buffer overflow, intentional
   compromise of a non-critical dependency).

   :Exposure: E4
   :Severity: S3
   :Controllability: C3
   :ASIL: D

Assumed Safety Goals
--------------------

Safety goals are the top-level functional intent that addresses the
hazards. Each ASG carries the ASIL of its source hazard.

.. assumed-safety-goal:: Prevent unintended termination of the safety-critical cyclic computation
   :id: ASG_0001
   :status: assumed
   :asil: D
   :links: AHZ_0001

   Prevent unintended termination of the safety-critical cyclic
   computation by lower-integrity software co-hosted in the same item.

   :Safe state: Integrator-defined fail-operational or fail-silent state.
   :FTTI: 100 ms (assumed for typical electromechanical actuators;
       integrator override via AOU_0006).

.. assumed-safety-goal:: Prevent silent corruption of safety-critical input/output data
   :id: ASG_0002
   :status: assumed
   :asil: D
   :links: AHZ_0002

   Prevent silent corruption of safety-critical input/output data by
   lower-integrity software co-hosted in the same item.

   :Safe state: Integrator-defined fail-operational or fail-silent state.
   :FTTI: 100 ms (assumed; override via AOU_0006).
