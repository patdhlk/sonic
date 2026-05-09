Soft-RT PLC runtime heart
=========================

This page captures the requirements for using `sonic-executor` as the runtime
heart of a soft-real-time PLC. It follows from the gap analysis between
typical PLC architecture (Beckhoff TwinCAT, Siemens TIA, B&R Automation
Studio, Rockwell Logix) and the abstractions sonic-executor provides today.

The decomposition is two-tier:

* **Top-level feature** — :need:`FEAT_0010` — the umbrella capability.
* **Sub-features** — capability themes, each one ``:satisfies:`` the
  top-level feature.
* **Requirements** — concrete shall-clauses that ``:satisfies:`` a
  sub-feature.

Sub-features are grouped into **foundation capabilities** (already provided
by sonic-executor v0.1) and **gap capabilities** (must be added before the
runtime credibly serves as a soft-RT PLC heart). Foundation reqs reference
the existing API surface; gap reqs describe TBD work.

Top-level feature
-----------------

.. feat:: PLC runtime heart on iceoryx2
   :id: FEAT_0010
   :status: open

   A Rust runtime that schedules, sequences, and observes the cyclic
   execution of PLC-style logic (read inputs → run logic → write outputs)
   under soft-real-time constraints, with iceoryx2 as the inter-process
   data plane.

   The runtime targets non-safety industrial automation, robotics control
   loops, and machine-monitoring scenarios. Hard-real-time bounds, safety
   certification, IEC 61131-3 frontends, hot-standby, and specific
   fieldbus protocol stacks are explicitly out of scope; the runtime
   integrates with such concerns but does not implement them.

----

Foundation capabilities
-----------------------

The following sub-features are **already provided** by sonic-executor v0.1.
Their requirements describe the contracts the runtime exposes today; the
work for them is closing the review/approval lifecycle, not authoring new
implementation.

Cyclic scan execution
~~~~~~~~~~~~~~~~~~~~~

.. feat:: Cyclic scan execution
   :id: FEAT_0011
   :status: open
   :satisfies: FEAT_0010

   Periodic execution of a scheduled item at a configured scan period —
   the PLC equivalent of a scan cycle.

.. req:: Configurable scan period
   :id: REQ_0001
   :status: open
   :satisfies: FEAT_0011

   The runtime shall allow each cyclic item to declare a scan period as a
   ``Duration`` via ``TriggerDeclarer::interval(period)``.

.. req:: One execution per scan period
   :id: REQ_0002
   :status: open
   :satisfies: FEAT_0011

   Under nominal load (no item exceeding its scan period), the runtime
   shall invoke each cyclic item exactly once per declared period.

.. req:: Scan-cycle execution observability
   :id: REQ_0003
   :status: open
   :satisfies: FEAT_0011

   The runtime shall emit pre-execute and post-execute timestamps for
   every scan-cycle invocation through the ``ExecutionMonitor`` trait.

Event-driven I/O dispatch
~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Event-driven I/O dispatch
   :id: FEAT_0012
   :status: open
   :satisfies: FEAT_0010

   Inter-process inputs and outputs flow through iceoryx2 channels so
   producers wake consumers without polling.

.. req:: Subscriber-triggered ingestion
   :id: REQ_0010
   :status: open
   :satisfies: FEAT_0012

   The runtime shall trigger an item's ``execute`` whenever a declared
   ``Subscriber<T>`` receives a new sample.

.. req:: Publisher-driven emission
   :id: REQ_0011
   :status: open
   :satisfies: FEAT_0012

   The runtime shall expose ``Publisher<T>`` send paths (``send_copy``,
   ``loan_send``, ``loan``) for emitting outputs to other processes.

.. req:: Zero-copy IPC transport
   :id: REQ_0012
   :status: open
   :satisfies: FEAT_0012

   Pub/sub data transfer between processes shall be zero-copy across
   shared memory via iceoryx2; receivers shall obtain a borrowed view of
   the producer's payload, not a deserialised copy.

.. req:: Notification-drop visibility
   :id: REQ_0013
   :status: open
   :satisfies: FEAT_0012

   The runtime shall surface dropped event-service notifications to the
   sender as a non-error counter (``NotifyOutcome::listeners_notified``)
   so the sender can detect consumer back-pressure programmatically.

Deterministic logic sequencing
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Deterministic logic sequencing
   :id: FEAT_0013
   :status: open
   :satisfies: FEAT_0010

   Items compose into chains and DAGs with explicit ordering and abort
   semantics — the structural equivalent of a PLC cause-effect network.

.. req:: Sequential chain execution
   :id: REQ_0020
   :status: open
   :satisfies: FEAT_0013

   The runtime shall execute the items of a chain in declared order on a
   single dispatch slot per chain invocation.

.. req:: Parallel DAG execution
   :id: REQ_0021
   :status: open
   :satisfies: FEAT_0013

   The runtime shall execute the vertices of a DAG concurrently when their
   in-edges are all satisfied, and shall block downstream vertices until
   all of their upstream vertices have completed.

.. req:: Abort propagation
   :id: REQ_0022
   :status: open
   :satisfies: FEAT_0013

   An item returning ``Ok(ControlFlow::StopChain)`` or ``Err`` shall
   prevent any downstream items in its enclosing chain or DAG from being
   dispatched within the same triggering cycle.

.. req:: Conditional inclusion
   :id: REQ_0023
   :status: open
   :satisfies: FEAT_0013

   The runtime shall provide a ``wrap_with_condition(item, predicate)``
   helper that gates an item's execution on a runtime-evaluated predicate.

Cycle-time watchdog
~~~~~~~~~~~~~~~~~~~

.. feat:: Cycle-time watchdog
   :id: FEAT_0014
   :status: open
   :satisfies: FEAT_0010

   Visibility into deadline-missed events at the dispatch layer.

.. req:: Subscriber deadline detection
   :id: REQ_0030
   :status: open
   :satisfies: FEAT_0014

   The runtime shall provide a ``TriggerDeclarer::deadline(subscriber,
   deadline)`` declaration that fires the item if no event arrives at the
   subscriber within ``deadline``.

.. req:: Per-execute timing visibility
   :id: REQ_0031
   :status: open
   :satisfies: FEAT_0014

   The runtime shall report each item's actual execute duration through
   ``ExecutionMonitor::post_execute(task, started_at, took, ok)``.

Real-time scheduling
~~~~~~~~~~~~~~~~~~~~

.. feat:: Real-time worker scheduling
   :id: FEAT_0015
   :status: open
   :satisfies: FEAT_0010

   Worker threads can be pinned and prioritized for predictable latency on
   PREEMPT_RT-capable Linux systems.

.. req:: Core-affinity assignment
   :id: REQ_0040
   :status: open
   :satisfies: FEAT_0015

   The runtime shall, behind the ``thread_attrs`` feature, allow worker
   threads to be pinned to a specified set of CPU cores.

.. req:: SCHED_FIFO priority on Linux
   :id: REQ_0041
   :status: open
   :satisfies: FEAT_0015

   The runtime shall, behind the ``thread_attrs`` feature on Linux, allow
   worker threads to run under ``SCHED_FIFO`` at a configured priority,
   subject to the process holding ``CAP_SYS_NICE``.

Cooperative shutdown
~~~~~~~~~~~~~~~~~~~~

.. feat:: Cooperative shutdown
   :id: FEAT_0016
   :status: open
   :satisfies: FEAT_0010

   The runtime exits cleanly on signal or programmatic stop without
   leaking worker threads or shared-memory artefacts.

.. req:: Signal-driven shutdown
   :id: REQ_0050
   :status: open
   :satisfies: FEAT_0016

   The runtime shall return cleanly from ``run()`` when SIGINT or SIGTERM
   is delivered to the process, surfacing iceoryx2's ``WaitSetRunResult``
   ``Interrupt`` and ``TerminationRequest`` variants.

.. req:: Programmatic shutdown wakeup
   :id: REQ_0051
   :status: open
   :satisfies: FEAT_0016

   The runtime shall expose a clonable ``Stoppable`` handle whose
   ``stop()`` method wakes the WaitSet thread within a bounded time even
   when no other trigger is pending.

----

Gap capabilities
----------------

The following sub-features are **not yet provided** by sonic-executor v0.1.
Each is a prerequisite for credibly calling the runtime a soft-real-time
PLC heart. Their requirements are authored at ``status: open`` and
represent work to be planned and executed.

Bounded-time dispatch
~~~~~~~~~~~~~~~~~~~~~

.. feat:: Bounded-time dispatch
   :id: FEAT_0017
   :status: open
   :satisfies: FEAT_0010

   The dispatch hot path shall not allocate, take unbounded locks, or
   block on poll loops, so steady-state cycle latency is bounded by
   factors the runtime declares (not by the system allocator or kernel
   futex implementation).

.. req:: No heap allocation in dispatch
   :id: REQ_0060
   :status: open
   :satisfies: FEAT_0017

   The runtime's dispatch path shall perform zero heap allocations during
   steady-state execution after ``Executor::run`` has been entered. All
   per-iteration data structures (error capture, vertex tracking,
   completion signalling) shall reuse capacity provisioned at
   ``Executor::build`` time.

.. req:: Statically-sized task pool
   :id: REQ_0061
   :status: open
   :satisfies: FEAT_0017

   The runtime's worker pool shall be sized at ``Executor::build`` time
   from a configuration value, and the dispatch path shall not grow or
   shrink the pool during execution.

.. req:: Wait-free completion signalling
   :id: REQ_0063
   :status: open
   :satisfies: FEAT_0017

   The graph DAG scheduler shall not rely on a polling condvar
   ``wait_timeout`` for vertex-completion signalling. Completion shall be
   communicated via a wait-free or bounded-wait primitive whose worst-case
   wakeup latency is documented and dominated by the kernel's wakeup
   delivery latency, not by an internal polling interval.

.. req:: Pre-allocated error slot
   :id: REQ_0062
   :status: open
   :satisfies: FEAT_0017

   The runtime shall capture per-iteration item errors in a pre-allocated
   bounded slot rather than constructing an ``Arc<Mutex<Option<...>>>``
   per dispatch iteration.

Cycle-overrun fault primitive
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Cycle-overrun fault primitive
   :id: FEAT_0018
   :status: open
   :satisfies: FEAT_0010

   Deadline violations transition the runtime — at task or executor scope —
   to a configured fault state, rather than only being reported as
   timestamps via ``ExecutionMonitor``.

.. req:: Per-task overrun fault transition
   :id: REQ_0070
   :status: open
   :satisfies: FEAT_0018

   When a task's ``execute`` exceeds a configured per-task deadline, the
   runtime shall transition that task to a configured fault state and
   shall not invoke its normal ``execute`` again until cleared.

.. req:: Executor-wide overrun fault transition
   :id: REQ_0071
   :status: open
   :satisfies: FEAT_0018

   When any single dispatch iteration exceeds a configured executor-wide
   deadline, the runtime shall transition the executor to a configured
   fault state.

.. req:: Fault-handler item dispatch
   :id: REQ_0072
   :status: open
   :satisfies: FEAT_0018

   When a task or the executor is in a fault state, the runtime shall
   not run the normal item logic and shall instead dispatch an optional
   user-supplied fault-handler item once per triggering cycle.

.. req:: Fault state observability
   :id: REQ_0073
   :status: open
   :satisfies: FEAT_0018

   Fault transitions shall be visible to the configured ``Observer`` via
   a dedicated callback distinct from ``on_app_error`` so users can react
   to overruns separately from item-returned errors.

Mode / state-machine framework
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Mode / state-machine framework
   :id: FEAT_0019
   :status: open
   :satisfies: FEAT_0010

   A first-class lifecycle for the runtime — distinct from item lifecycle
   — that captures the operational modes typical of PLC programs.

.. req:: Mode lifecycle
   :id: REQ_0080
   :status: open
   :satisfies: FEAT_0019

   The runtime shall support an explicit mode lifecycle of at least
   ``{init, ready, running, fault, stopping, stopped}`` and shall expose
   the current mode through a query API.

.. req:: Mode transition triggers
   :id: REQ_0081
   :status: open
   :satisfies: FEAT_0019

   Mode transitions shall be triggered both programmatically (caller-driven)
   and as a consequence of configured events (executor-wide deadline
   overrun, item error, signal-driven stop).

.. req:: Per-mode task gating
   :id: REQ_0082
   :status: open
   :satisfies: FEAT_0019

   Each registered task shall declare which modes it is enabled in; the
   runtime shall not dispatch a task while it is disabled by the current
   mode.

.. req:: Mode change observability
   :id: REQ_0083
   :status: open
   :satisfies: FEAT_0019

   Mode transitions shall be visible to the configured ``Observer`` via
   a dedicated callback that reports the previous mode, the new mode, and
   the reason for the transition.

Retentive state
~~~~~~~~~~~~~~~

.. feat:: Retentive state
   :id: FEAT_0020
   :status: open
   :satisfies: FEAT_0010

   State that survives process restarts — the equivalent of NVRAM-backed
   retentive memory in classical PLCs.

.. req:: Process-restart persistence
   :id: REQ_0090
   :status: open
   :satisfies: FEAT_0020

   The runtime shall provide a retentive memory abstraction whose declared
   contents persist unchanged across cooperative process restarts.

.. req:: Memory-mapped backing
   :id: REQ_0091
   :status: open
   :satisfies: FEAT_0020

   Retentive memory regions shall be backed by a memory-mapped file with
   a checksum verified at load.

.. req:: Crash-atomic checkpoints
   :id: REQ_0092
   :status: open
   :satisfies: FEAT_0020

   A retentive-memory checkpoint shall be atomic with respect to process
   crash — a concurrent crash shall yield either the pre-checkpoint or
   post-checkpoint contents, never a partial state.

.. req:: Recovery status reporting
   :id: REQ_0093
   :status: open
   :satisfies: FEAT_0020

   At startup, the runtime shall report whether retentive state was loaded
   cleanly, recovered from an incomplete checkpoint (and which version was
   selected), or initialised from defaults because no prior state existed.

Scan-cycle observability
~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Scan-cycle observability
   :id: FEAT_0021
   :status: open
   :satisfies: FEAT_0010

   First-class statistics on cycle-time behaviour — percentiles, jitter,
   overrun counts — exposed without requiring users to build their own.

.. req:: Per-task latency percentiles
   :id: REQ_0100
   :status: open
   :satisfies: FEAT_0021

   The runtime shall report p50, p95, and p99 execute-duration percentiles
   per registered task, computed over a sliding window whose size is
   configurable at ``Executor::build`` time.

.. req:: Per-task maximum jitter
   :id: REQ_0101
   :status: open
   :satisfies: FEAT_0021

   The runtime shall report the maximum observed jitter — defined as the
   absolute difference between actual and declared scan period — per
   cyclic task.

.. req:: Per-task overrun counter
   :id: REQ_0102
   :status: open
   :satisfies: FEAT_0021

   The runtime shall expose a monotonic counter per task that increments
   on each scan-cycle execution that exceeds the declared scan period.

.. req:: Statistics query API
   :id: REQ_0103
   :status: open
   :satisfies: FEAT_0021

   Cycle-cycle statistics shall be available both as Observer callbacks
   (push) and as a query API on ``Executor`` (pull) so monitoring agents
   can sample at their own cadence.

PREEMPT_RT validation
~~~~~~~~~~~~~~~~~~~~~

.. feat:: PREEMPT_RT validation harness
   :id: FEAT_0022
   :status: open
   :satisfies: FEAT_0010

   The runtime's worst-case latency on PREEMPT_RT Linux is characterised
   under realistic load — a continuous regression gate, not a one-off
   measurement.

.. req:: Documented worst-case jitter
   :id: REQ_0110
   :status: open
   :satisfies: FEAT_0022

   The runtime shall ship a documented worst-case jitter envelope on at
   least one PREEMPT_RT Linux configuration, including the kernel version,
   isolation flags (``isolcpus``, ``nohz_full``), CPU model, and load
   profile under which the envelope holds.

.. req:: Cyclictest-style benchmark harness
   :id: REQ_0111
   :status: open
   :satisfies: FEAT_0022

   The repository shall include a benchmark harness that exercises the
   dispatch path under a configured load and emits per-cycle latency
   distributions in a machine-readable form.

.. req:: CI regression gate on jitter
   :id: REQ_0112
   :status: open
   :satisfies: FEAT_0022

   A CI job shall run the benchmark harness on a dedicated PREEMPT_RT
   runner and shall fail the build if the observed jitter envelope
   exceeds the declared threshold.

Fieldbus integration interface
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Fieldbus integration interface
   :id: FEAT_0023
   :status: open
   :satisfies: FEAT_0010

   The shape by which fieldbus protocol stacks (EtherCAT, Modbus, Profinet,
   CIP) plug into the runtime — without committing to any specific
   protocol implementation in the core.

.. req:: Adapter-driven I/O
   :id: REQ_0120
   :status: open
   :satisfies: FEAT_0023

   The runtime shall expose an adapter trait by which a fieldbus driver
   produces ``Channel<T>`` / ``Subscriber<T>`` bindings for ingested
   process variables and consumes ``Publisher<T>`` for outputs.

.. req:: Out-of-tree driver crates
   :id: REQ_0121
   :status: open
   :satisfies: FEAT_0023

   Fieldbus driver implementations shall live in separate crates and shall
   not require modifications to the executor core.

.. req:: Protocol-neutral runtime
   :id: REQ_0122
   :status: open
   :satisfies: FEAT_0010

   The executor core shall not embed any specific fieldbus protocol
   implementation; protocol selection is a deployment concern carried in
   adapter crates.

----

Cross-cutting traceability
--------------------------

Every requirement on this page ``:satisfies:`` exactly one parent feature;
every sub-feature ``:satisfies:`` :need:`FEAT_0010`. The needtables on
:doc:`index` and :doc:`../architecture/index` will populate as ``spec``
artefacts are authored.

.. needtable::
   :types: feat
   :columns: id, title, status, satisfies
   :show_filters:

.. needtable::
   :types: req
   :columns: id, title, status, satisfies
   :show_filters:
