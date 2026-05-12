Overview
========

``sonic-executor`` is a Rust execution framework on `iceoryx2`_ that turns IPC
events, intervals, and request/response activity into deterministic,
observable schedules of executable items.

This specification frames ``sonic-executor`` as the **runtime heart of a
soft-real-time PLC**: a foundation for non-safety industrial automation,
robotics control loops, machine-monitoring runtimes, and R&D testbeds where
occasional jitter is acceptable. The framing follows directly from a gap
analysis (recorded in :doc:`requirements/plc-runtime`) that distinguishes
the capabilities ``sonic-executor`` already provides from those that must be
added before it can credibly call itself a soft-RT PLC.

What is **out of scope** for the runtime heart:

* Hard-real-time guarantees (cycle deadlines bounded to microsecond jitter
  under any load).
* IEC 61508 / 26262 functional safety certification or SIL / ASIL claims.
* IEC 61131-3 frontends (ladder logic, function block diagram, structured text).
* Hot-standby, redundancy, online change.
* Specific fieldbus protocols (EtherCAT, Modbus, Profinet, CIP); only the
  *integration interface* is in scope.

This document is structured as engineering artefacts. Top-level capabilities
are ``feat`` directives; individual obligations are ``req`` directives that
``:satisfies:`` their parent feature. Every artefact carries a stable ID
(``FEAT_NNNN``, ``REQ_NNNN``) and a lifecycle ``:status:``.

.. _iceoryx2: https://github.com/eclipse-iceoryx/iceoryx2

Reading order
-------------

Start with :doc:`requirements/plc-runtime` for the full feature/requirement
decomposition. Work-in-progress sections — :doc:`architecture/index` and
:doc:`verification/index` — will land alongside the implementation work that
closes the gap requirements.

Status legend
-------------

* ``open`` — drafted; not yet reviewed.
* ``draft`` — under active authoring (used during edits).
* ``reviewed`` — passed at least one structured review.
* ``approved`` — accepted into the baseline.

All artefacts in this initial drop are authored at ``open`` until they are
reviewed; the ``status`` lifecycle is honest about provenance rather than
optimistic.
