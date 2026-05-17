.. _safety-decomposition:

ASIL Decomposition
==================

Per ISO 26262-9 §5, an ASIL D safety goal may be decomposed into
two independent safety requirements at lower ASILs, provided the two
elements satisfying them are sufficiently independent in implementation,
hardware allocation, and failure modes.

Both ASG_0001 and ASG_0002 are decomposed as:

``ASIL D = ASIL B(D) + ASIL B(D)``

The ``(D)`` annotation means each element is developed at ASIL B but
under ASIL D process constraints (so the decomposition is reversible
should an integrator later choose to recombine).

Element A — Sonic execution path (ASIL B(D))
--------------------------------------------

Sonic and the hosted application item perform the safety function:
acquire inputs, compute the control output, drive the actuator via
the connector layer. Implemented in safe Rust, hosted in a process
isolated from any QM-grade co-hosted code (see :doc:`ffi`).

Element B — Integrator-supplied diverse monitor (ASIL B(D))
-----------------------------------------------------------

An independent path verifies the plausibility of sonic's safety-critical
output and forces safe state on detected omission or value failure.
Typically a separate hardware safety MCU, a watchdog with deadline
injection capability, or a diverse software channel on a partitioned
core. Sonic does not supply Element B — see AOU_0001.

Independence argument (ISO 26262-9 §5.4.4)
------------------------------------------

The two elements MUST be sufficiently independent. Specifically:

* Different software toolchains or formally diverse implementations.
* Different CPU cores (preferred: separate SoCs); the monitor MUST NOT
  depend on sonic-hosted resources to perform its check.
* Independent power and clock domains where feasible.
* The heartbeat protocol carries enough state for the monitor to detect
  both omission (Element A halted) and value (Element A producing wrong
  output) failures.

This argument is **claimed but not closed by sonic**. Closure is the
integrator's responsibility, covered by AOU_0001 (diverse monitor),
AOU_0002 (independence), and AOU_0003 (heartbeat receiver).
