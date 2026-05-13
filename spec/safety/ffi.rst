.. _safety-ffi:

Freedom From Interference Argument
==================================

Per ISO 26262-6 Annex D, the FFI argument justifies that the Element A
(sonic + hosted application) at ASIL B(D) can stand even with QM-grade
code co-hosted in the same item. FFI must hold across three categories:
spatial, temporal, and information-exchange.

Spatial FFI
-----------

**Threats:** stray pointers, buffer overflows, use-after-free,
allocator denial-of-service from QM-grade items corrupting or starving
safety-critical state.

**Mitigations:**

1. **Rust memory safety.** Safe Rust prevents the classic
   memory-corruption families within a process by construction.
   Refines AFSR_0001.

2. **Process boundary as isolation context.** The OS MMU enforces
   address-space isolation between the SC process (hosting safety-critical
   items) and one-or-more QM processes (connectors, codecs,
   observability). This is the userspace analog of NVIDIA's kernel
   page-table-swap "safety context" mechanism, but cheaper — OS
   process isolation is already supplied. Refines AFSR_0001 →
   :need:`TSR_0003`, :need:`TSR_0009`.

3. **iceoryx2 shared-memory capability model.** Each shared-memory
   segment grants W/R capability per-process at open time. The SC
   process owns the writable side of SC→QM channels; a QM process
   holds only a read handle. Reverse-direction QM→SC channels are
   read-only from SC's side. Refines AFSR_0002 → :need:`TSR_0007`.

4. **Partitioned bounded allocator.** Separate quota pools per integrity
   level prevent QM exhaustion from denying SC. Refines AFSR_0003 →
   :need:`TSR_0002`.

**Residual risk** absorbed by AOU_0008 (integrator's own ``unsafe``
discipline) and AOU_0009 (host OS, libc, iceoryx2, toolchain
qualification at ASIL B(D)).

Temporal FFI
------------

**Threats:** QM-grade items consume CPU, hold OS locks, or block
priorities such that SC items miss their deadlines.

**Mitigations within sonic:**

1. **Missed-deadline detection** in ``sonic-executor`` — :need:`TSR_0004`.
2. **Heartbeat emission** to the integrator's Element B monitor —
   :need:`TSR_0010`.

**Sonic does not itself enforce temporal FFI.** Temporal isolation is
delegated to the OS scheduler and the Element B monitor:

* AOU_0005 — integrator configures real-time scheduling class
  (SCHED_FIFO or SCHED_DEADLINE) and CPU pinning.
* AOU_0001 — the diverse Element B monitor catches deadline overrun
  and forces safe state.

Information-exchange FFI
------------------------

**Threats:** channel-level corruption, repetition, insertion, loss,
masquerade, out-of-order delivery between SC and QM processes
communicating through iceoryx2.

**Mitigations:**

1. **One-writer/many-reader iceoryx2 topology** prevents writer
   collision — :need:`TSR_0007`.

2. **Envelope sequence + CRC** detects corruption, repetition,
   omission, and out-of-order. ``ConnectorEnvelope`` carries a sequence
   counter and a CRC over header + payload; CRC mismatch on read raises
   ``HealthEvent::Faulted`` and discards the frame — :need:`TSR_0008`.

3. **Compile-time channel direction** via Rust's type system prevents
   accidental cross-channel writes within a process — :need:`TSR_0005`.

**Residual risk** — loss-on-the-wire (iceoryx2 does not guarantee
delivery, only zero-copy when delivered) is absorbed by AOU_0007: the
SC application item must enter safe state on omission.
