.. _safety-item:

Assumed Item
============

SEooC item-level scope. The integrator's real item must be confirmed
against this description (AOU_0006).

The assumed item is a sonic-hosted **safety-critical periodic control
application** running on a single multi-core SoC under a POSIX-compliant
operating system. The item performs cyclic input acquisition, control
computation, and actuation via field-bus (EtherCAT) and/or pub/sub
(Zenoh) connectors at a cycle rate in the range 1–100 ms. Sonic provides
the execution framework (``sonic-executor``) and the I/O substrate
(``sonic-connector-*``).

In scope (sonic's responsibility)
---------------------------------

* Deterministic execution of registered items at declared triggers
  (intervals, channel arrivals, request/response).
* Bounded memory allocation (``sonic-bounded-alloc``).
* Spatial Freedom From Interference between safety-critical items and
  QM-grade items co-hosted in the same workspace.
* Detection and propagation of internal framework faults — allocator
  exhaustion, missed deadlines, connector disconnect, item panic,
  channel corruption — via the ``ConnectorHealth`` channel.

Out of scope (→ becomes Assumption of Use on the integrator)
------------------------------------------------------------

* Correctness of the safety-critical control algorithm.
* Functional safety of the host OS, libc, hardware (CPU, RAM, clock,
  power).
* CPU / RAM / clock / power fault containment.
* Temporal Freedom From Interference enforcement (scheduling class,
  CPU pinning).
* The diverse monitoring path required by the ASIL D = B(D) + B(D)
  decomposition (see :doc:`decomposition`).
* Reaction to safety-goal violations escalated by sonic — sonic raises
  ``HealthEvent::Faulted``; it does not define what safe state means
  for any particular application.
