Connector framework — architecture (arc42)
==========================================

Architecture documentation for the connector framework, structured per
the arc42 template (12 sections) and encoded with sphinx-needs using
the useblocks "x-as-code" conventions
(https://x-as-code.useblocks.com/how-to-guides/arc42/index.html).

Each architectural element ``:refines:`` or ``:implements:`` a parent
requirement from :doc:`../requirements/connector` so the trace is
preserved end-to-end.

.. contents:: Sections
   :local:
   :depth: 1

----

1. Introduction and goals
-------------------------

The connector framework's reason-to-exist is fault isolation: keep messy
network protocol code (MQTT, OPC UA, gRPC, fieldbus) outside the
sonic-executor application's deterministic core, while preserving
zero-copy data flow. Quality goals capture the qualities that the
architecture is optimised for.

.. quality-goal:: Fault isolation between protocol stack and app
   :id: QG_0001
   :status: open
   :refines: FEAT_0030

   A panic, hang, or crash in a protocol stack (rumqttc, opcua, tonic,
   ADS) shall not be able to crash, deadlock, or stall the
   sonic-executor application that uses the framework. This goal is
   what motivates the gateway-as-separate-process deployment shape and
   the single-direction control plane.

.. quality-goal:: Compile-time type safety end-to-end
   :id: QG_0002
   :status: open
   :refines: FEAT_0030

   Plugin code that targets a specific protocol shall be checked at
   compile time for routing correctness, codec compatibility, and
   payload-size compliance. Runtime "config-as-strings" indirection
   shall be avoided; type errors are caught by ``cargo check``.

.. quality-goal:: Zero-copy data flow on the publish path
   :id: QG_0003
   :status: open
   :refines: FEAT_0031

   Outbound messages from the application to the broker shall not be
   copied into any intermediate buffer between the codec's encode call
   and the iceoryx2 publish. The iceoryx2 ``Publisher::loan`` mechanism
   carries the codec's output directly to shared memory.

.. quality-goal:: Uniform observable health across connectors
   :id: QG_0004
   :status: open
   :refines: FEAT_0034

   Every connector — regardless of which protocol stack owns its
   reconnect mechanism — shall report the same four health states
   (Up / Connecting / Degraded / Down) on a single observable channel,
   so monitoring and alerting code is connector-agnostic.

----

2. Constraints
--------------

Constraints come from the surrounding workspace and the iceoryx2
ecosystem; they are non-negotiable inputs to the architecture.

.. constraint:: Built on sonic-executor's WaitSet
   :id: CON_0001
   :status: open
   :refines: FEAT_0030

   The plugin and gateway shall be sonic-executor consumers
   (``ExecutableItem``-based, WaitSet-driven). The framework shall not
   introduce a second reactor model running alongside sonic-executor.

.. constraint:: iceoryx2 0.8.x as the IPC layer
   :id: CON_0002
   :status: open
   :refines: FEAT_0030

   The framework shall use the workspace's pinned iceoryx2 version
   (``0.8`` per ``Cargo.toml`` workspace dependencies). Migration to
   a later iceoryx2 series is a follow-on effort outside this spec.

.. constraint:: Rust 2024 edition / MSRV 1.85
   :id: CON_0003
   :status: open
   :refines: FEAT_0030

   All new crates shall target edition 2024 with MSRV 1.85, matching
   the workspace's ``rust-toolchain.toml`` and ``[workspace.package]``.

.. constraint:: Single-threaded test discipline
   :id: CON_0004
   :status: open
   :refines: FEAT_0030

   Workspace tests run with ``--test-threads=1`` because each iceoryx2
   service must own a unique name in shared memory. New crates'
   integration tests shall be safe under this discipline (per-test
   ``Node`` names + per-test tokio runtimes).

.. constraint:: Tokio sidecar contained per connector crate
   :id: CON_0005
   :status: open
   :refines: FEAT_0030

   Where async protocol stacks (``rumqttc``, ``tonic``) require tokio,
   each connector crate shall host its own tokio runtime sidecar; tokio
   shall not appear as a dependency of ``sonic-connector-core``,
   ``sonic-connector-transport-iox``, or ``sonic-connector-codec``.

----

3. Context and scope
--------------------

.. architecture:: System context
   :id: ARCH_0001
   :status: open
   :refines: FEAT_0030

   The connector framework sits between a sonic-executor application
   and one or more external systems (brokers, servers, PLCs).
   Internally, the boundary is split between a **plugin** (in-app side)
   and a **gateway** (out-of-app side); externally, the gateway is the
   only component that touches network I/O.

   .. mermaid::

      flowchart LR
        APP["sonic-executor application<br/>(plugin uses Connector trait)"]
        SHM[("iceoryx2 shared memory<br/>+ event service")]
        GW["sonic-connector gateway<br/>(tokio + protocol stack)"]
        EXT[("external system<br/>e.g. MQTT broker")]
        APP -- ConnectorEnvelope --> SHM
        SHM -- ConnectorEnvelope --> APP
        SHM -- ConnectorEnvelope --> GW
        GW -- ConnectorEnvelope --> SHM
        GW -- protocol native --> EXT
        EXT -- protocol native --> GW

   In-process deployment collapses the SHM hop to a single-process
   shared-memory transport but preserves the same envelope contract;
   see :need:`ARCH_0020` and :need:`ARCH_0021`.

----

.. _connector-arc42-solution-strategy:

4. Solution strategy
--------------------

The framework's shape is the consequence of ten architectural decisions
made during brainstorming. Each decision is captured here as an ADR
that ``:refines:`` the requirement or feature it answers.

.. arch-decision:: Spec scope — framework core + MQTT reference
   :id: ADR_0001
   :status: open
   :refines: FEAT_0030

   **Context.** Four protocol connectors (MQTT, OPC UA, gRPC, ADS) and
   three codecs (JSON, Protobuf, MessagePack) were on the table. Each
   protocol introduces its own design quirks; specifying all four in
   one round risks the spec drifting into protocol-specific minutiae.

   **Decision.** This spec covers the framework core plus **MQTT** as
   the reference connector. OPC UA / gRPC / ADS get follow-on specs
   reusing the same five contracts.

   **Consequences.** ✅ Spec stays focused on the framework's contracts.
   ✅ MQTT exercises every contract (codec, routing, health, reconnect)
   end-to-end. ❌ Other connector specs are blocked on this one
   landing.

.. arch-decision:: Umbrella feature is a peer of FEAT_0010
   :id: ADR_0002
   :status: open
   :refines: FEAT_0030

   **Context.** :need:`FEAT_0010` "PLC runtime heart" is the existing
   top-level umbrella, with :need:`FEAT_0023` "Fieldbus integration
   interface" as a sub-feature. The connector framework is broader than
   fieldbus (MQTT and gRPC are application-protocol level).

   **Decision.** Add :need:`FEAT_0030` "Connector framework" as a peer
   top-level feature, not under :need:`FEAT_0010`. :need:`FEAT_0023`
   later ``:refines:`` :need:`FEAT_0030` when an ADS connector spec
   lands.

   **Consequences.** ✅ Honest semantics — the framework is general
   purpose, not PLC-bound. ❌ The spec now has two top-level umbrellas,
   which the overview page should explicitly explain.

.. arch-decision:: Both deployment shapes supported
   :id: ADR_0003
   :status: open
   :refines: FEAT_0035

   **Context.** Gateway-as-separate-process gives fault isolation
   (:need:`QG_0001`); gateway-as-tokio-task is operationally simpler
   (one binary, one signal handler). Different consumers want different
   trade-offs.

   **Decision.** Define the framework so the same envelope/iceoryx2
   contract works in either deployment. The host wires the gateway as
   a tokio task or a separate binary using identical code; only
   process-startup differs.

   **Consequences.** ✅ Fault-isolation-conscious deployments and
   single-binary deployments share one framework. ❌ Both paths must be
   tested; shutdown coordination is specified twice (in-process,
   out-of-process), but the SHM mechanics are unchanged.

.. arch-decision:: Per-channel envelope size, declared in descriptor
   :id: ADR_0004
   :status: open
   :refines: REQ_0201

   **Context.** A universal 64 KB envelope (the C# Apex.Ida pattern)
   wastes shared memory for small messages and refuses large ones.
   iceoryx2's typed services support per-service payload sizes.

   **Decision.** ``ChannelDescriptor`` carries a per-channel max
   payload size (via const generic ``N``); each iceoryx2 service is
   typed on ``ConnectorEnvelope<N>`` for its compile-time-chosen ``N``.

   **Consequences.** ✅ Memory sized to the workload. ✅ Type system
   prevents publishers and subscribers from disagreeing on size. ❌
   Different channels are different types; const-generic
   monomorphisation could grow code size if many channel sizes are
   used (see :need:`RISK_0003`).

.. arch-decision:: Codec is a generic parameter on the connector
   :id: ADR_0005
   :status: open
   :refines: REQ_0211

   **Context.** Two clean alternatives existed: type-erased
   ``Box<dyn PayloadCodec>`` (runtime-swappable, ``erased_serde``
   indirection) or generic-on-connector (``MqttConnector<C>``,
   compile-time-monomorphised).

   **Decision.** Generic-on-connector. Concrete connector types are
   ``MqttConnector<JsonCodec>``, ``MqttConnector<MsgPackCodec>``, etc.

   **Consequences.** ✅ Zero dynamic dispatch on the hot path. ✅ Codec
   errors carry a static ``format_name``. ❌ Cannot swap codec at
   runtime; code must rebuild to change codec for a connector.

.. arch-decision:: Explicit-builder plugin discovery
   :id: ADR_0006
   :status: open
   :refines: REQ_0270

   **Context.** Two alternatives: ``inventory``-crate compile-time
   registration (link-time globals collect ``ConnectorRegistration``
   entries) versus an explicit builder (``ConnectorHost::builder()
   .with(MqttConnector::<JsonCodec>::new(...)).build()``).

   **Decision.** Explicit builder. Matches sonic-executor's existing
   ``Executor::builder()`` idiom.

   **Consequences.** ✅ One file you can grep for the wiring; no
   link-time global state alongside the compile-time generics. ❌
   Adding a connector requires rebuilding the host (already true given
   :need:`ADR_0005`).

.. arch-decision:: Plugin and gateway are both sonic-executor consumers
   :id: ADR_0007
   :status: open
   :refines: CON_0001

   **Context.** Three options: tokio-only gateway (separate world from
   plugin), sonic-executor on both sides with tokio bridged in, or
   raw-iceoryx2 gateway emitting unified observability.

   **Decision.** Both halves are ``ExecutableItem``-based. Tokio runs
   as a sidecar inside connector crates; sonic-executor's
   ``Channel<T>`` bridges the two. One programming model, one
   observability surface, one shutdown story.

   **Consequences.** ✅ ``Observer`` and ``ExecutionMonitor`` cover the
   gateway for free. ✅ SIGINT-clean-exit story propagates without
   extra plumbing. ❌ The bridge is the place latency can be
   introduced; bridge-channel sizing matters.

.. arch-decision:: Routing carried as a typed struct
   :id: ADR_0008
   :status: open
   :refines: REQ_0221

   **Context.** Three positions: opaque channel name + side-channel
   YAML config; channel name + typed routing struct; channel name +
   key-value attribute bag.

   **Decision.** Typed routing struct (``MqttRouting``, ``OpcUaRouting``,
   ...) implementing the ``Routing`` marker trait, embedded in
   ``ChannelDescriptor``.

   **Consequences.** ✅ Routing is part of the public, type-checked API.
   ✅ Catches misspelled / missing fields at compile time. ❌ Plugin
   code is connector-aware (no protocol-portable channels — see
   :need:`REQ_0294`).

.. arch-decision:: Lifecycle = ReconnectPolicy + ConnectorHealth
   :id: ADR_0009
   :status: open
   :refines: FEAT_0034

   **Context.** Different protocol stacks own reconnect differently —
   ``rumqttc`` exposes raw connect events (fits a policy trait);
   ``tonic`` manages reconnect inside the channel (no hooks); OPC UA
   sessions sit in between.

   **Decision.** Provide both a ``ReconnectPolicy`` trait + default
   ``ExponentialBackoff`` (used by stacks that surface raw events) AND
   a ``ConnectorHealth`` state machine emitted via ``HealthEvent``
   (uniform observability regardless of who owns reconnect).

   **Consequences.** ✅ Stacks that fit a uniform policy aren't
   reinventing backoff; stacks that handle reconnect internally aren't
   forced into a foreign mechanism. ❌ Two ways to get reconnect
   means new connector authors must pick the right one for their
   protocol.

.. arch-decision:: MQTT scope — realistic but bounded
   :id: ADR_0010
   :status: open
   :refines: FEAT_0036

   **Context.** "Reference connector" must exercise enough of the
   framework's contracts to validate them, without ballooning into
   MQTT-protocol-minutiae territory.

   **Decision.** Pub+sub, QoS 0+1, retained messages, wildcard
   subscriptions, username/password auth, optional TLS, MQTT 3.1.1.
   Defer: QoS 2, MQTT 5, LWT, persistent sessions, client-cert TLS.

   **Consequences.** ✅ Each deferred feature exercises framework
   contracts — adding them later doesn't reshape the framework.
   ❌ MQTT 5 user-properties / shared-subscriptions adoption is
   blocked on a follow-on spec.

.. arch-decision:: ethercrab as the EtherCAT MainDevice library
   :id: ADR_0020
   :status: open
   :refines: FEAT_0041

   **Context.** EtherCAT MainDevice options in Rust are ``ethercrab``
   (pure Rust, ``std`` + ``no_std``, actively maintained), ``soem-rs``
   (FFI wrapper around the C SOEM stack), or hand-rolled. SOEM is the
   industry-standard C implementation, but pulling C dependencies and
   their build complexity into the workspace conflicts with the
   no-C-deps posture the rest of sonic adopts.

   **Decision.** Use ``ethercrab`` from the workspace. It is pure
   Rust, supports both ``std`` (tokio TX/RX task on Linux raw socket)
   and ``no_std`` (deferred), and exposes a typestate bring-up API
   (``init_single_group`` → ``into_op``) that maps cleanly onto the
   four EtherCAT bus states.

   **Consequences.** ✅ No C build dependencies; one ``cargo build``
   gets everything. ✅ ``no_std`` deployment becomes possible without
   a second EtherCAT stack. ❌ ethercrab is pre-1.0, so API churn is
   a tracked risk. ❌ SOEM conformance test coverage is broader;
   ethercrab is validated against EK1100 / EL-series modules but
   uncommon vendor extensions may surface gaps.

.. arch-decision:: Single MainDevice per gateway
   :id: ADR_0021
   :status: open
   :refines: REQ_0312

   **Context.** An EtherCAT network is physically one segment per
   network interface; the MainDevice owns that segment's TX/RX cycle.
   Multi-NIC support would require multiple MainDevices arbitrating
   shared cycle timing and working-counter state.

   **Decision.** Each ``EthercatGateway`` instance owns exactly one
   ``ethercrab::MainDevice`` bound to one network interface. Multi-NIC
   deployments instantiate multiple gateways with disjoint SHM service
   names.

   **Consequences.** ✅ Cycle timing, working-counter ownership, and
   Distributed Clocks bring-up have a single source of truth.
   ✅ Mirrors :need:`REQ_0295` (one broker per MQTT gateway).
   ❌ Operators wanting one process to own two EtherCAT segments must
   instantiate two gateways (acceptable — rare configuration).

.. arch-decision:: Static PDO mapping declared at build time
   :id: ADR_0022
   :status: open
   :refines: REQ_0314, REQ_0315

   **Context.** EtherCAT SubDevice PDO mappings can be sourced two
   ways: (1) parsing an ESI / EEPROM XML descriptor per SubDevice at
   startup, or (2) declaring the mapping in application code at build
   time. ESI parsing is what TwinCAT and similar engineering tools
   do; it handles arbitrary vendor modules. Static declaration trades
   generality for compile-time type safety on the routing struct.

   **Decision.** The application declares each SubDevice's PDO
   mapping as a static description in ``EthercatConnectorOptions``;
   the gateway applies it during the PRE-OP → SAFE-OP transition via
   SDO writes to the sync-manager assignment indices ``0x1C12``
   (RxPDO) and ``0x1C13`` (TxPDO). ESI parsing is out of scope.

   **Consequences.** ✅ ``EthercatRouting`` (:need:`REQ_0311`) becomes
   a compile-time-checked struct — bit offset, bit length, and PDO
   direction match the static map. ✅ No runtime XML parsing.
   ❌ Adding a new SubDevice model requires a code change, not a
   config-file swap. ❌ Out-of-tree SubDevices with unusual PDO
   assignments need manual mapping (acceptable — matches the rest of
   sonic's compile-time-config posture).

.. arch-decision:: Distributed Clocks bring-up is opt-in
   :id: ADR_0023
   :status: open
   :refines: REQ_0318

   **Context.** DC sub-microsecond synchronisation matters for motion
   control and time-stamped sampling; many EtherCAT deployments
   (digital I/O, ramped analog, slow process control) don't need it.
   DC bring-up adds a multi-pass register dance (BWR ``0x0900``,
   per-slave offset write to ``0x0920``, FRMW from ``0x0910``) and
   requires every SubDevice on the segment to declare 64-bit DC
   support.

   **Decision.** The gateway performs DC bring-up only when
   ``EthercatConnectorOptions::distributed_clocks`` is explicitly
   enabled by the application. Default is off.

   **Consequences.** ✅ Buses without DC-capable SubDevices work out
   of the box. ✅ Bring-up latency is lower when DC is unused.
   ❌ Motion-control applications must remember to enable DC.
   ❌ Two bring-up paths to test (with and without DC).

.. arch-decision:: Linux raw socket only in first cut
   :id: ADR_0024
   :status: open
   :refines: REQ_0325

   **Context.** ethercrab supports Linux raw sockets, NPCAP / WinPcap
   on Windows, and ``no_std`` direct-MAC drivers. Each adds porting
   work. EtherCAT in industrial deployments is overwhelmingly Linux;
   the production target is Linux.

   **Decision.** The first cut uses ethercrab's ``std::tx_rx_task``
   helper, which opens an ``AF_PACKET`` raw socket. Linux is the only
   supported host OS; the gateway process requires ``CAP_NET_RAW``.
   Windows and ``no_std`` MCU deployments are deferred.

   **Consequences.** ✅ One bring-up path to test in the first cut.
   ✅ Deployment recipe is "install the binary, grant CAP_NET_RAW".
   ❌ Windows-based engineering desks cannot run the gateway natively
   (they can run plugins; the gateway must live on Linux).
   ❌ Embedded MCU EtherCAT mainboards await a follow-on spec.

.. arch-decision:: ``sonic-connector-ethercat`` module decomposition
   :id: ADR_0025
   :status: open
   :refines: FEAT_0041

   **Context.** :need:`BB_0030` decomposes into plugin (:need:`BB_0031`),
   gateway (:need:`BB_0032`), PDO mapping (:need:`BB_0033`), and the
   tokio bridge (:need:`BB_0034`). An implementing crate can either
   place everything in one ``lib.rs`` (faster initial build, harder to
   navigate) or mirror the BB decomposition in module structure
   (one-to-one mapping to specs, slightly more setup).

   **Decision.** ``sonic-connector-ethercat`` mirrors the BB tree as
   sibling modules: ``plugin``, ``gateway``, ``pdo``, ``bridge``,
   ``options``, and ``health``. The public surface re-exports
   ``EthercatConnector`` from ``plugin``, ``EthercatGateway`` from
   ``gateway``, and ``EthercatConnectorOptions`` /
   ``EthercatRouting`` from ``options``. Internal modules are
   ``pub(crate)``.

   **Consequences.** ✅ Each module maps to one BB, so the
   ``IMPL_`` directive can refine its REQs at module granularity if
   future work needs finer-grained traceability. ✅ Test files
   under ``tests/`` align with module names. ❌ One more layer of
   directory nesting than the smaller framework crates currently
   adopt; acceptable because the connector crate is the largest.

.. arch-decision:: Tokio runtime owned by ``EthercatGateway``, joined on Drop
   :id: ADR_0026
   :status: open
   :refines: REQ_0321

   **Context.** :need:`REQ_0321` requires the ethercrab TX/RX task to
   run on a tokio runtime contained inside the connector crate, with
   no tokio leakage into sonic-executor's ``WaitSet`` thread. Three
   shapes are possible: (1) a global ``OnceCell<Runtime>`` shared
   across gateway instances, (2) a runtime owned per-``EthercatGateway``
   instance, joined on ``Drop``, (3) a runtime spawned externally and
   handed to the gateway via a builder.

   **Decision.** Each ``EthercatGateway`` instance owns its own
   ``tokio::runtime::Runtime`` (multi-threaded, defaulting to one
   worker thread, configurable via
   ``EthercatConnectorOptions::tokio_worker_threads``). The runtime
   is constructed in ``EthercatGateway::new`` and shut down via
   ``Runtime::shutdown_timeout`` in ``Drop`` with a 5-second budget
   (mirroring REQ_0244's SIGINT clean-exit budget).

   **Consequences.** ✅ Lifecycle is one-to-one with the gateway —
   no global state, multiple gateways on one host are independent.
   ✅ Mirrors :need:`ADR_0021` (one MainDevice per gateway).
   ❌ Spawning two gateways doubles the tokio worker-thread count;
   operators wanting a shared pool must consolidate gateways or wait
   for a follow-on spec.

.. arch-decision:: ``EthercatConnectorOptions`` is a typed builder; PDO map declared as ``&'static [SubDeviceMap]``
   :id: ADR_0027
   :status: open
   :refines: REQ_0314, REQ_0315

   **Context.** :need:`REQ_0314` requires the PDO mapping be declared
   by the application at build time via ``EthercatConnectorOptions``.
   Two builder shapes are common in Rust: (1) ``Default`` + public
   mutable fields, (2) a fluent typed builder with ``with_*``
   methods returning ``Self``. The PDO map itself can be a heap
   ``Vec<SubDeviceMap>`` or a ``&'static [SubDeviceMap]`` declared
   in application code.

   **Decision.** ``EthercatConnectorOptions`` is a typed builder
   (``EthercatConnectorOptions::builder()...with_subdevice(...).build()``)
   matching :need:`REQ_0270`'s ``ConnectorHost::builder()`` idiom.
   The PDO map is declared as ``&'static [SubDeviceMap]`` — held by
   reference so the application can place it in ``.rodata`` and the
   gateway needs no per-instance heap allocation for it. Individual
   ``SubDeviceMap`` entries reference ``&'static [PdoEntry]`` for the
   same reason.

   **Consequences.** ✅ No heap allocation for the PDO map after
   gateway construction (consistent with sonic-executor's REQ_0060
   posture for the steady-state hot path). ✅ Builder API parallel to
   the framework's other connector options. ❌ Applications that need
   runtime-discovered PDO maps (e.g. EEPROM-parsed) must roll their
   own ``&'static`` storage or wait for a runtime-PDO follow-on spec.

.. arch-decision:: Verification harness — pure-logic unit tests + env-gated bus tests
   :id: ADR_0028
   :status: open
   :refines: FEAT_0041

   **Context.** :need:`FEAT_0041` ships 16 TEST artefacts
   (TEST_0200..TEST_0215) verifying REQ_0310..REQ_0325. Six of those
   tests (TEST_0203, TEST_0205, TEST_0208, TEST_0209, TEST_0210,
   TEST_0215) exercise real bus state transitions, PDO mapping
   application, working-counter accounting, DC bring-up, or raw
   socket access — operations that need either an ``ethercrab``
   ``MainDevice`` driving a real NIC or a mock that simulates
   SubDevice responses. An earlier draft of this ADR assumed
   ``ethercrab`` shipped a ``MockMainDevice``; it does not (as of
   ``ethercrab`` 0.7), so the verification strategy below is the
   actual approach taken.

   **Decision.** The connector's testable logic is factored into
   pure-Rust modules — :need:`IMPL_0050`'s ``sdo`` (SDO write
   sequence generation), ``scheduler`` (cycle-time pacing with
   skip-not-catch-up semantics), ``wkc`` (working-counter health
   policy), ``bridge`` (bounded outbound / inbound bridges),
   ``health`` (health monitor + broadcast), ``options`` (typed
   builder with default-clamp), and ``routing`` — and unit-tested
   deterministically without ``ethercrab`` on the wire (TEST_0201,
   TEST_0204, TEST_0205-partial, TEST_0206, TEST_0207, TEST_0209,
   TEST_0210, TEST_0211-partial, TEST_0212, TEST_0213, TEST_0214 all
   land via this path). The remaining bus-driven tests
   (TEST_0202, TEST_0203, TEST_0205-full, TEST_0208 wire-side,
   TEST_0211-full, TEST_0215) live in
   ``crates/sonic-connector-ethercat/tests`` and are gated on the
   ``ETHERCAT_TEST_NIC`` environment variable; absent the variable
   they ``skip!`` rather than failing. CI runs the pure-logic tests
   on every push; the bus suite runs only on the gateway host
   (Linux + CAP_NET_RAW) as a manual workflow.

   **Consequences.** ✅ Every PR build is green on every developer
   machine and CI runner — no flaky "missing NIC" failures.
   ✅ The factored pure-logic modules (``sdo`` / ``scheduler`` /
   ``wkc``) carry the gateway's load-bearing decision logic and are
   exhaustively tested. ✅ The bus suite still exists in-tree and is
   one ``ETHERCAT_TEST_NIC=eth0`` away from running. ❌ The bus
   tests are not on the CI gate; a regression that only surfaces on
   real hardware will only be caught when the gateway host runs the
   suite — documented as an accepted risk. ❌ Without a mock, the
   bridge between ``ethercrab``'s ``MainDevice`` API and the
   pure-logic helpers is itself untested at unit level; a follow-on
   may introduce a trait abstraction (``BusDriver`` with a
   ``MockBusDriver`` impl in ``dev-dependencies``) once the
   integration surface is stable enough for the abstraction not to
   churn.

.. arch-decision:: Zenoh queries live on a concrete handle type, not the Connector trait
   :id: ADR_0040
   :status: open
   :refines: FEAT_0044

   **Context.** The framework explicitly rejected protocol-portable
   channels (:need:`REQ_0294`) and framework-level request/response
   matching (:need:`REQ_0290`). Three options for surfacing Zenoh
   queries existed: (a) concrete methods on ``ZenohConnector`` only;
   (b) extend the ``Connector`` trait with default-noop query methods;
   (c) re-use pub/sub plus app-level correlation.

   **Decision.** Option (a). ``ZenohConnector::create_querier`` and
   ``ZenohConnector::create_queryable`` are concrete methods that
   return Zenoh-specific handle types (``ZenohQuerier``,
   ``ZenohQueryable``). The shared ``Connector`` trait remains
   unchanged.

   **Consequences.** ✅ Honors :need:`REQ_0290` / :need:`REQ_0294`. ✅
   MQTT and EtherCAT connectors are not forced to invent
   no-op query plumbing. ❌ Plugin code wanting queries depends on
   the concrete ``ZenohConnector`` type, not the abstract trait —
   but that is exactly the framework's existing posture for
   protocol-specific affordances (:need:`REQ_0224`).

.. arch-decision:: Stack-internal reconnect for Zenoh — no ReconnectPolicy
   :id: ADR_0041
   :status: open
   :refines: FEAT_0045

   **Context.** Zenoh's own session machinery handles scout and
   reconnect (peer mode) and reconnect-to-router (client mode). The
   framework provides :need:`REQ_0232` ``ReconnectPolicy`` and a
   default ``ExponentialBackoff``, but also explicitly allows
   stack-internal-reconnect connectors to skip it
   (:need:`REQ_0235`).

   **Decision.** The Zenoh connector follows the
   stack-internal-reconnect path. ``ReconnectPolicy`` is not used;
   the gateway observes the Zenoh session's alive/closed state and
   emits ``HealthEvent`` on every transition. An anti-req
   :need:`REQ_0441` records the decision in the requirements page.

   **Consequences.** ✅ No duplicate retry policy contending with
   Zenoh's own. ✅ Health emission stays uniform across all
   connectors (:need:`REQ_0234`). ❌ If a future user wants
   ``zenoh::open`` itself retried with backoff on initial config
   failure, that becomes a follow-on req — current behavior is to
   return ``Down`` and rely on application-level restart.

.. arch-decision:: One ZenohRouting struct carries pub/sub QoS; query knobs on options
   :id: ADR_0042
   :status: open
   :refines: FEAT_0043

   **Context.** :need:`REQ_0224` already declares that each
   connector ships a single routing struct (``MqttRouting``,
   ``EthercatRouting``, ``ZenohRouting``) implementing the
   ``Routing`` marker. Zenoh has both pub/sub QoS knobs
   (congestion control, priority, reliability, express) and
   query-specific knobs (target, consolidation, timeout). Two
   options: (a) one routing struct carrying pub/sub QoS, with
   query knobs on ``ZenohConnectorOptions``; (b) two distinct
   routing structs.

   **Decision.** Option (a). ``ZenohRouting`` carries
   ``{ key_expr, congestion_control, priority, reliability,
   express }``. Query-specific knobs (target, consolidation,
   timeout) live on ``ZenohConnectorOptions`` as session-wide
   defaults; ``ZenohQuerier`` exposes a builder to override the
   timeout per-call.

   **Consequences.** ✅ Preserves :need:`REQ_0224`'s single-routing-
   struct rule. ✅ Mirrors :need:`REQ_0251` (MQTT carries QoS in
   routing). ❌ Per-channel query target / consolidation overrides
   require a builder method instead of a routing field — accepted
   tradeoff for type-system simplicity.

.. arch-decision:: Reply framing uses a Zenoh-private 1-byte payload prefix
   :id: ADR_0043
   :status: open
   :refines: FEAT_0044

   **Context.** Multi-reply Zenoh queries need an end-of-stream
   signal in addition to data chunks. Two options: (a) allocate
   one bit of ``ConnectorEnvelope``'s reserved word
   (:need:`REQ_0200`) — but that turns the reserved word into
   Zenoh-specific framework metadata; (b) carry a one-byte frame
   discriminator inside ``envelope.payload[0]`` — Zenoh-private,
   the framework remains agnostic.

   **Decision.** Option (b). Every envelope on the two reply-side
   iceoryx2 services (``{name}.reply.in`` / ``{name}.reply.out``)
   begins ``payload`` with a 1-byte discriminator: ``0x01`` = data
   chunk (followed by codec-encoded ``R``), ``0x02`` = end of
   stream (empty body), ``0x03`` = gateway-synthetic timeout
   (empty body). The framework's reserved word stays untouched.

   **Consequences.** ✅ Framework anti-goal (no inspection of
   envelope payload, no protocol-portable semantics in the
   reserved word) preserved. ✅ Future connectors can re-use the
   pattern without coordinating with the framework. ❌ Plugin-side
   ``ZenohQuerier::try_recv`` and ``ZenohQueryable::reply`` add a
   single-byte skip / write step relative to pub/sub channels.

----

5. Building block view
----------------------

The framework decomposes into five workspace crates plus reuse of two
existing sonic-executor crates. The decomposition is hierarchical: a
level-1 view shows crate-level building blocks; level-2 zooms into the
two crates that carry the most logic.

.. building-block:: sonic-connector-core
   :id: BB_0001
   :status: open
   :implements: REQ_0220, REQ_0221, REQ_0222

   Pure trait definitions and shared types. No IPC, no protocol code.
   Public surface: ``Connector`` trait, ``PayloadCodec`` trait,
   ``Routing`` marker, ``ChannelDescriptor<R, const N: usize>``,
   ``ConnectorHealth``, ``HealthEvent``, ``ReconnectPolicy``,
   ``ExponentialBackoff``, ``ConnectorError``.

.. building-block:: sonic-connector-transport-iox
   :id: BB_0002
   :status: open
   :implements: REQ_0200, REQ_0205, REQ_0206

   Concrete envelope (``ConnectorEnvelope<const N: usize>``) and
   iceoryx2-backed channel handles
   (``ChannelWriter<T, C, N>``, ``ChannelReader<T, C, N>``,
   ``ServiceFactory``). Depends on
   ``sonic-connector-core``, ``iceoryx2``, ``sonic-executor``.

.. building-block:: sonic-connector-codec
   :id: BB_0003
   :status: open
   :implements: REQ_0210, REQ_0212

   Concrete ``PayloadCodec`` implementations. ``JsonCodec`` ships
   default-on; ``MsgPackCodec`` and ``ProtoCodec`` are deferred behind
   cargo features.

.. building-block:: sonic-connector-mqtt
   :id: BB_0004
   :status: open
   :implements: REQ_0250, REQ_0251, REQ_0258

   MQTT plugin (``MqttConnector<C>`` implementing ``Connector``) and
   gateway (``MqttGateway`` exposing executable items). Hosts the
   tokio sidecar driving ``rumqttc::EventLoop`` and the bridge
   between sonic-executor and tokio.

.. building-block:: sonic-connector-host
   :id: BB_0005
   :status: open
   :implements: REQ_0270, REQ_0271, REQ_0272

   Composition layer. Provides ``ConnectorHost::builder()`` and
   ``ConnectorGateway::builder()`` wrapping a
   ``sonic_executor::Executor``. Optional ``Observer`` adapter to
   ``sonic-executor-tracing`` lives behind a ``tracing`` cargo feature.

.. architecture:: Level-1 building block decomposition
   :id: ARCH_0002
   :status: open
   :refines: BB_0001, BB_0002, BB_0003, BB_0004, BB_0005, BB_0030, BB_0040

   Crate-level building blocks and their dependency graph. All edges
   point from depender to dependee. The graph is acyclic; the host is
   the only consumer of every other new crate. The
   ``sonic-connector-ethercat`` crate (BB_0030) is a peer of
   ``sonic-connector-mqtt`` (BB_0004) — both depend on the same
   core / transport / codec triad and feed the host.

   .. mermaid::

      flowchart TB
        subgraph existing_crates[existing crates]
          EX[sonic-executor]
          TR[sonic-executor-tracing]
        end
        subgraph new_crates["new crates (this spec)"]
          CO[sonic-connector-core<br/>BB_0001]
          TX[sonic-connector-transport-iox<br/>BB_0002]
          CD[sonic-connector-codec<br/>BB_0003]
          MQ[sonic-connector-mqtt<br/>BB_0004]
          EC[sonic-connector-ethercat<br/>BB_0030]
          ZE[sonic-connector-zenoh<br/>BB_0040]
          HO[sonic-connector-host<br/>BB_0005]
        end
        CO --> TX
        CO --> CD
        CO --> MQ
        CO --> EC
        TX --> MQ
        TX --> EC
        CD --> MQ
        CD --> EC
        EX --> TX
        EX --> MQ
        EX --> EC
        CO --> HO
        TX --> HO
        CD --> HO
        MQ --> HO
        EC --> HO
        CO --> ZE
        TX --> ZE
        CD --> ZE
        EX --> ZE
        ZE --> HO
        TR -.optional adapter.-> HO

.. building-block:: ConnectorEnvelope (sub-block of BB_0002)
   :id: BB_0010
   :status: open
   :implements: REQ_0200, REQ_0201, REQ_0202, REQ_0203, REQ_0204

   The on-wire form. ``#[repr(C)]`` POD type with a fixed header
   (sequence number, timestamp, length, correlation id) and a
   const-generic-sized payload buffer.

   .. code-block:: rust

      #[repr(C)]
      #[derive(Debug, Copy, Clone, ZeroCopySend)]
      pub struct ConnectorEnvelope<const N: usize> {
          pub sequence_number: u64,
          pub timestamp_ns:    u64,
          pub payload_length:  u32,
          pub _reserved:       u32,
          pub correlation_id:  [u8; 32],
          pub payload:         [u8; N],
      }

   At plan stage, the implementation may substitute a small set of
   size-tier types (4 KB / 64 KB / 1 MB) for the const-generic
   variant. The external contract — fixed at service-creation time —
   is identical either way.

.. building-block:: ServiceFactory (sub-block of BB_0002)
   :id: BB_0011
   :status: open
   :implements: REQ_0206

   Derives iceoryx2 service names deterministically from a
   ``ChannelDescriptor`` and creates the publisher / subscriber /
   event-service pairs for each direction.

   .. code-block:: text

      out service:    sonic.connector.<connector>.<channel>.out
      in  service:    sonic.connector.<connector>.<channel>.in
      out event:      sonic.connector.<connector>.<channel>.out.evt
      in  event:      sonic.connector.<connector>.<channel>.in.evt

.. building-block:: MqttConnector (sub-block of BB_0004, plugin side)
   :id: BB_0020
   :status: open
   :implements: REQ_0250, REQ_0251

   ``MqttConnector<C: PayloadCodec>``. Implements ``Connector`` with
   ``type Routing = MqttRouting``. ``create_writer`` /
   ``create_reader`` build ``ServiceFactory``-backed channel handles;
   ``health()`` reads the gateway's status snapshot.

.. building-block:: MqttGateway (sub-block of BB_0004, gateway side)
   :id: BB_0021
   :status: open
   :implements: REQ_0258, REQ_0259, REQ_0260, REQ_0261

   Hosts ``rumqttc::AsyncClient`` + ``EventLoop`` on a tokio runtime,
   plus the bridge channels and the executable items
   (``OutboundGatewayItem``, ``InboundGatewayItem``) registered with
   sonic-executor.

.. building-block:: Tokio bridge (sub-block of BB_0021)
   :id: BB_0022
   :status: open
   :implements: REQ_0259, REQ_0260, REQ_0261

   Two bounded channel pairs that translate between sonic-executor's
   thread (WaitSet driver) and the tokio runtime owning rumqttc.
   Outbound = ``tokio::sync::mpsc``; inbound = ``crossbeam_channel``
   wired as a sonic-executor signal source.

.. building-block:: sonic-connector-ethercat
   :id: BB_0030
   :status: open
   :implements: REQ_0310, REQ_0311, REQ_0312, REQ_0321

   EtherCAT plugin (``EthercatConnector<C>`` implementing
   ``Connector``) and gateway (``EthercatGateway`` exposing executable
   items). Hosts the tokio sidecar driving ethercrab's ``tx_rx_task``
   and the bridge between sonic-executor and tokio. Depends on
   ``sonic-connector-core``, ``sonic-connector-transport-iox``,
   ``ethercrab``, ``sonic-executor``.

.. building-block:: EthercatConnector (sub-block of BB_0030, plugin side)
   :id: BB_0031
   :status: open
   :implements: REQ_0310, REQ_0311

   Plugin-side ``EthercatConnector<C: PayloadCodec>``. Owns no I/O —
   produces ``ChannelWriter`` / ``ChannelReader`` handles whose
   ``EthercatRouting`` (SubDevice configured address, PDO direction,
   bit offset within the SubDevice's process data, bit length of the
   mapped object) identifies one process-data slice. Acts as a
   compile-time-checked façade over the gateway's SHM services.

.. building-block:: EthercatGateway (sub-block of BB_0030, gateway side)
   :id: BB_0032
   :status: open
   :implements: REQ_0312, REQ_0313, REQ_0325

   Gateway-side executable item that owns the ethercrab ``MainDevice``
   and ``PduStorage`` on one Linux network interface. Brings the bus
   from INIT through PRE-OP and SAFE-OP to OP via the typestate
   ``init_single_group`` / ``into_op`` API before serving plugin
   traffic. Opens the NIC via ``ethercrab::std::tx_rx_task``;
   requires ``CAP_NET_RAW``.

.. building-block:: PDO mapping (sub-block of BB_0030)
   :id: BB_0033
   :status: open
   :implements: REQ_0314, REQ_0315

   Module that accepts a static PDO-mapping description per SubDevice
   from ``EthercatConnectorOptions`` and applies it via SDO writes to
   the sync-manager assignment indices ``0x1C12`` (RxPDO) and
   ``0x1C13`` (TxPDO) during the PRE-OP → SAFE-OP transition. No ESI
   or EEPROM parsing.

.. building-block:: Tokio bridge for ethercrab (sub-block of BB_0030)
   :id: BB_0034
   :status: open
   :implements: REQ_0322, REQ_0323, REQ_0324

   Two bounded channel pairs that translate between sonic-executor's
   WaitSet thread and the tokio runtime owning ethercrab's
   ``tx_rx_task``. Outbound saturation surfaces as
   ``ConnectorError::BackPressure`` plus ``ConnectorHealth::Degraded``;
   inbound saturation emits ``HealthEvent::DroppedInbound { count }``
   and drops the inbound process image for the affected cycle.

.. building-block:: sonic-connector-zenoh
   :id: BB_0040
   :status: open
   :implements: REQ_0400, REQ_0420, REQ_0440, REQ_0444

   Zenoh plugin (``ZenohConnector<C>`` implementing ``Connector``)
   and gateway (``ZenohGateway`` exposing executable items). Hosts
   the tokio sidecar driving ``zenoh::Session`` and the bridge
   between sonic-executor and tokio. Depends on
   ``sonic-connector-core``, ``sonic-connector-transport-iox``,
   ``sonic-connector-codec``, ``sonic-executor``, and (behind the
   ``zenoh-integration`` feature) ``zenoh``.

.. building-block:: ZenohConnector (sub-block of BB_0040, plugin side)
   :id: BB_0041
   :status: open
   :implements: REQ_0400, REQ_0401, REQ_0420

   Plugin-side ``ZenohConnector<C: PayloadCodec>``. Implements
   ``Connector`` with ``type Routing = ZenohRouting`` and adds
   concrete non-trait methods ``create_querier`` /
   ``create_queryable``. Owns no I/O — produces ``ChannelWriter`` /
   ``ChannelReader`` / ``ZenohQuerier`` / ``ZenohQueryable`` handles
   whose ``ZenohRouting`` identifies a Zenoh key expression and the
   pub/sub QoS knobs. Acts as a compile-time-checked façade over
   the gateway's SHM services.

.. building-block:: ZenohGateway (sub-block of BB_0040, gateway side)
   :id: BB_0042
   :status: open
   :implements: REQ_0403, REQ_0426, REQ_0440, REQ_0442

   Gateway-side executable item that owns one ``zenoh::Session``
   created via ``zenoh::open(config)`` (or a ``MockZenohSession``
   when ``zenoh-integration`` is off — both implement the
   ``ZenohSessionLike`` trait). Maintains a per-channel routing
   registry mapping each open ``ChannelDescriptor`` to its
   declared Zenoh primitive (publisher / subscriber / queryable),
   and a ``correlation_id → zenoh::Query`` map for in-flight
   queryable reply streams. Translates session-alive ↔
   session-closed transitions into ``HealthEvent``s without
   using ``ReconnectPolicy``.

.. building-block:: Zenoh query handles (sub-block of BB_0041)
   :id: BB_0043
   :status: open
   :implements: REQ_0420, REQ_0421, REQ_0422, REQ_0423, REQ_0424

   ``ZenohQuerier<Q, R, C, N>`` and ``ZenohQueryable<Q, R, C, N>``.
   The non-trait query handle types. ``ZenohQuerier::send`` mints
   a ``QueryId``, encodes ``Q`` via the connector's codec, and
   publishes on the channel's ``{name}.query.out`` iceoryx2
   service; ``try_recv`` drains ``{name}.reply.in`` and decodes
   the 1-byte frame discriminator (0x01=data, 0x02=EoS,
   0x03=timeout) plus the codec-encoded ``R`` chunk.
   ``ZenohQueryable::try_recv`` surfaces ``(QueryId, Q)``; ``reply``
   stamps the ``QueryId`` back onto a reply envelope and publishes
   on ``{name}.reply.out``; ``terminate(id)`` publishes a 0x02
   envelope finalising the upstream ``zenoh::Query``.

.. building-block:: Tokio bridge for zenoh (sub-block of BB_0042)
   :id: BB_0044
   :status: open
   :implements: REQ_0403, REQ_0404, REQ_0405, REQ_0406

   Two bounded channel pairs that translate between sonic-executor's
   WaitSet thread and the tokio runtime owning ``zenoh::Session``.
   Outbound saturation surfaces as ``ConnectorError::BackPressure``
   plus ``ConnectorHealth::Degraded``; inbound saturation emits
   ``HealthEvent::DroppedInbound { count }`` and drops the
   offending sample or reply chunk. Same shape as :need:`BB_0034`
   (EtherCAT) and :need:`BB_0022` (MQTT).

.. building-block:: sonic-connector-can crate
   :id: BB_0070
   :status: open
   :implements: REQ_0600, REQ_0602, REQ_0603, REQ_0604, REQ_0605

   CAN plugin (``CanConnector<C>`` implementing ``Connector``) and
   gateway (``CanGateway`` exposing executable items). Hosts the
   tokio sidecar driving N SocketCAN sockets and the bridges
   between sonic-executor and tokio. Depends on
   ``sonic-connector-core``, ``sonic-connector-transport-iox``,
   ``sonic-connector-codec``, ``sonic-executor``, and (behind the
   ``socketcan-integration`` feature) ``socketcan`` with its
   ``tokio`` feature enabled. Ships ``MockCanInterface``
   unfeature-gated for layer-1 tests on any host OS.

.. building-block:: CanConnector (sub-block of BB_0070, plugin side)
   :id: BB_0071
   :status: open
   :implements: REQ_0600, REQ_0601, REQ_0612, REQ_0615, REQ_0621

   Plugin-side ``CanConnector<C: PayloadCodec>``. Implements
   ``Connector`` with ``type Routing = CanRouting``. Owns no I/O —
   produces ``ChannelWriter<T, C, N>`` / ``ChannelReader<T, C, N>``
   handles whose ``CanRouting`` declares the target interface,
   CAN ID, mask, frame kind, and FD flags. Validates that
   ``CanRouting::iface`` belongs to the configured gateway's
   interface set and that ``ChannelDescriptor::max_payload_size``
   matches ``CanRouting::kind`` (8 for Classical, 64 for FD)
   before any iceoryx2 service is created. Acts as a
   compile-time-checked façade over the gateway's SHM services.

.. building-block:: CanGateway (sub-block of BB_0070, gateway side)
   :id: BB_0072
   :status: open
   :implements: REQ_0613, REQ_0614, REQ_0620, REQ_0624, REQ_0625, REQ_0630, REQ_0631

   Gateway-side executable item that owns one ``CanInterfaceLike``
   per configured interface (real ``socketcan::CanSocket`` /
   ``CanFdSocket`` when ``socketcan-integration`` is on,
   ``MockCanInterface`` otherwise — both implement
   ``CanInterfaceLike``). For each interface, runs an RX task
   draining the socket and a TX drain consuming the outbound
   bridge. Maintains a per-interface routing registry mapping
   each open ``ChannelDescriptor`` to its ``CanRouting`` and
   direction. Aggregates per-interface sub-states into the
   externally-visible ``ConnectorHealth`` via worst-of
   (:need:`REQ_0630`), enables ``CAN_ERR_FLAG`` on every owned
   socket, classifies error frames internally (:need:`REQ_0631`),
   and never forwards error frames to plugin channels
   (:need:`REQ_0636`, :need:`REQ_0643`).

.. building-block:: Tokio bridge for CAN (sub-block of BB_0072)
   :id: BB_0073
   :status: open
   :implements: REQ_0605, REQ_0606, REQ_0607, REQ_0608

   Two bounded channel pairs per owned interface that translate
   between sonic-executor's WaitSet thread and the tokio runtime
   owning the SocketCAN sockets. Outbound saturation surfaces as
   ``ConnectorError::BackPressure`` plus
   ``ConnectorHealth::Degraded``; inbound saturation emits
   ``HealthEvent::DroppedInbound { count }`` and drops the
   offending CAN frame. Same shape as :need:`BB_0044` (Zenoh),
   :need:`BB_0034` (EtherCAT), and :need:`BB_0022` (MQTT).

.. building-block:: Per-iface filter compiler (sub-block of BB_0072)
   :id: BB_0074
   :status: open
   :implements: REQ_0622, REQ_0623, REQ_0624

   Pure-logic helper that maps the per-interface registry of
   inbound ``CanRouting`` entries to a single
   ``Vec<libc::can_filter>`` (or the ``socketcan`` crate's
   equivalent newtype) and applies it via
   ``setsockopt(SOL_CAN_RAW, CAN_RAW_FILTER, …)``. Recomputed
   whenever a reader is created or dropped on the affected
   interface; the recompute does not require the socket to be
   re-opened or the bus to leave its current state. Symmetric
   counterpart for the inbound demux side: given a received
   frame, returns the list of registered readers whose
   ``(can_id, mask, extended)`` matches under kernel
   ``CAN_RAW_FILTER`` semantics so that every matching reader
   gets its own envelope copy (:need:`REQ_0624`).

.. building-block:: MockCanInterface (sub-block of BB_0070)
   :id: BB_0075
   :status: open
   :implements: REQ_0604

   In-process loopback implementation of ``CanInterfaceLike``,
   shipping in the default build (not gated by
   ``socketcan-integration``). Sends queued for transmission on a
   mock interface are immediately delivered to any reader whose
   filter matches; programmable error-frame injection drives the
   :need:`BB_0072` gateway's health classifier under test.
   Exists so the Layer-1 test pyramid can exercise the full
   envelope ↔ interface ↔ envelope hop on Linux, macOS, and
   Windows without depending on the real ``socketcan`` crate or
   a Linux kernel CAN module. Mirrors :need:`BB_0040`'s
   ``MockZenohSession`` posture under :need:`REQ_0445`.

----

6. Runtime view
---------------

Four scenarios cover the connector framework's externally-observable
behaviour. Each ``:refines:`` the requirements that govern its
behaviour and the building blocks that implement it.

.. architecture:: Send path (app → broker)
   :id: ARCH_0010
   :status: open
   :refines: REQ_0205, BB_0021, BB_0022

   The send path is fully zero-copy on the sender's side: the codec
   writes directly into shared memory via ``Publisher::loan``.

   .. mermaid::

      sequenceDiagram
        autonumber
        participant U as user code
        participant W as ChannelWriter
        participant P as Publisher (sonic-executor)
        participant SHM as iceoryx2 SHM
        participant S as Subscriber (gateway)
        participant GI as OutboundGatewayItem
        participant BR as Tokio bridge
        participant MQ as rumqttc::AsyncClient
        participant B as Broker

        U->>W: writer.send(&value)
        W->>P: publisher.loan(|slot| codec.encode(value, slot.payload))
        P->>SHM: zero-copy publish + notify
        SHM-->>S: WaitSet wakes
        S->>GI: ExecutableItem::execute()
        GI->>BR: bridge_tx.try_send(payload, routing)
        BR-->>MQ: tokio task drains bridge
        MQ->>B: client.publish(topic, qos, retained, payload)
        B-->>MQ: PUBACK (QoS 1)

.. architecture:: Receive path (broker → app)
   :id: ARCH_0011
   :status: open
   :refines: REQ_0205, REQ_0254, BB_0021, BB_0022

   The receive path mirrors the send path. The gateway's tokio task
   pushes incoming protocol-stack messages into an inbound bridge
   channel; the inbound gateway item resolves the channel by topic
   match (with wildcard demultiplexing) and re-publishes the envelope
   into the inbound iceoryx2 service.

   .. mermaid::

      sequenceDiagram
        autonumber
        participant B as Broker
        participant MQ as rumqttc::EventLoop
        participant BR as Tokio bridge
        participant GI as InboundGatewayItem
        participant P as Publisher (gateway, in svc)
        participant SHM as iceoryx2 SHM
        participant S as Subscriber (app)
        participant R as ChannelReader
        participant U as user code

        B->>MQ: PUBLISH (topic, payload)
        MQ-->>BR: tokio task pushes (topic, payload) into bridge_in
        BR->>GI: sonic-executor Channel wakes item
        GI->>GI: resolve channel by topic match (wildcard demux)
        GI->>P: publisher.loan(|slot| copy payload, set header)
        P->>SHM: zero-copy publish + notify
        SHM-->>S: WaitSet wakes
        S->>R: reader.try_recv() → Received{ value, header }
        R->>U: user code consumes value

.. architecture:: Health and reconnect lifecycle
   :id: ARCH_0012
   :status: open
   :refines: REQ_0230, REQ_0234, BB_0021

   Every connector implements the same state machine. Concrete
   connectors may add private sub-states, but the externally-visible
   variants are exactly four.

   .. mermaid::

      stateDiagram-v2
        [*] --> Connecting: gateway started
        Connecting --> Up: protocol stack reports connected
        Up --> Degraded: transient error (e.g. PUBACK timeout)
        Degraded --> Up: recovery
        Up --> Down: stack-level disconnect
        Degraded --> Down: error threshold exceeded
        Down --> Connecting: ReconnectPolicy backoff elapses
        Connecting --> Down: connect attempt fails
        Up --> [*]: shutdown
        Down --> [*]: shutdown

   Every transition emits a ``HealthEvent`` on the connector's health
   channel; the host can forward these into ``sonic_executor::Observer``
   via the optional ``tracing``-feature adapter.

.. architecture:: Shutdown coordination
   :id: ARCH_0013
   :status: open
   :refines: REQ_0243, BB_0005, BB_0021

   Shutdown is downstream of sonic-executor: when ``Executor::run()``
   returns (signal or programmatic stop), the host triggers the tokio
   runtime's ``shutdown_timeout(5s)`` (configurable). Out-of-process
   gateway binaries follow the same pattern; the app side detects loss
   via ``HealthEvent::Down``.

   .. mermaid::

      sequenceDiagram
        autonumber
        participant SIG as SIGINT/SIGTERM
        participant EX as sonic-executor WaitSet
        participant HO as ConnectorHost / Gateway
        participant GI as Gateway items
        participant TT as Tokio runtime
        participant B as Broker

        SIG->>EX: signal delivered
        EX->>EX: WaitSet returns Interrupt
        EX->>HO: Executor::run() returns
        HO->>GI: drop items
        HO->>TT: shutdown_handle.send(())
        TT->>B: client.disconnect() (best-effort)
        B-->>TT: DISCONNECT ack (or timeout)
        TT->>TT: tokio runtime drained
        HO-->>SIG: process exits

.. architecture:: EtherCAT bus bring-up sequence
   :id: ARCH_0040
   :status: open
   :refines: REQ_0313, REQ_0314, REQ_0315, BB_0032, BB_0033

   Bring-up walks the four EtherCAT bus states. PDO mapping is applied
   during the PRE-OP → SAFE-OP transition — the only window where SDO
   writes to the sync-manager assignment indices land on a stable
   mailbox but the cyclic process image is not yet live. Plugin
   traffic is accepted only after the bus reaches OP.

   .. mermaid::

      sequenceDiagram
        autonumber
        participant HO as ConnectorHost
        participant GW as EthercatGateway
        participant EC as ethercrab MainDevice
        participant SD as SubDevices
        participant PL as Plugin (EthercatConnector)

        HO->>GW: start gateway
        GW->>EC: PduStorage.try_split + MainDevice::new
        GW->>EC: spawn tx_rx_task on tokio sidecar
        GW->>EC: init_single_group (INIT to PRE-OP)
        EC->>SD: discover and address SubDevices
        GW->>SD: SDO writes 0x1C12 / 0x1C13 (PDO mapping)
        GW->>EC: group.into_safe_op (PRE-OP to SAFE-OP)
        GW->>EC: group.into_op (SAFE-OP to OP)
        GW-->>HO: ConnectorHealth::Up
        PL->>GW: writer.send / reader.try_recv accepted

.. architecture:: Cyclic process-data exchange and working-counter health
   :id: ARCH_0041
   :status: open
   :refines: REQ_0316, REQ_0317, REQ_0319, REQ_0320, BB_0032, BB_0034

   The gateway runs one ``group.tx_rx`` cycle per tick on
   ``tokio::time::interval`` with ``MissedTickBehavior::Skip``.
   Working-counter inspection on each completed cycle drives the
   externally observable ``ConnectorHealth`` transitions; the resulting
   state machine matches :need:`ARCH_0012` (uniform health surface)
   with EtherCAT-specific entry conditions.

   .. mermaid::

      stateDiagram-v2
        [*] --> Connecting: gateway started
        Connecting --> Up: bus reaches OP and WKC matches
        Up --> Degraded: WKC below expected on N consecutive cycles
        Degraded --> Up: WKC restored
        Up --> Down: bus drops below OP
        Degraded --> Down: WKC remains below expected past timeout
        Down --> Connecting: ReconnectPolicy backoff elapses
        Connecting --> Down: bring-up attempt fails
        Up --> [*]: shutdown
        Down --> [*]: shutdown

.. architecture:: Optional Distributed Clocks bring-up
   :id: ARCH_0042
   :status: open
   :refines: REQ_0318, BB_0032

   When ``EthercatConnectorOptions::distributed_clocks`` is enabled,
   the gateway inserts a register-level bring-up step between the
   PRE-OP and SAFE-OP transitions of :need:`ARCH_0040`. Each step
   uses standard EtherCAT broadcast or configured-write commands; the
   sequence runs once per fresh bring-up and once per ReconnectPolicy-
   driven re-bringup. When DC is disabled, the entire sequence is
   skipped.

   .. mermaid::

      sequenceDiagram
        autonumber
        participant GW as EthercatGateway
        participant EC as ethercrab MainDevice
        participant SD as SubDevices

        GW->>EC: read SupportFlags (0x0008) per SubDevice
        EC->>SD: BRD 0x0008
        SD-->>EC: 64-bit DC support flags
        GW->>EC: latch port-0 receive times
        EC->>SD: BWR 0x0900 (system time)
        loop per DC-capable SubDevice
            EC->>SD: read 0x0918 (receive time processing unit)
            SD-->>EC: t_recv
            EC->>SD: write 0x0920 (system time offset = t_sys - t_recv)
        end
        GW->>EC: propagate reference clock
        EC->>SD: FRMW 0x0910 (first SubDevice clock)

.. architecture:: CAN frame send path (app → bus)
   :id: ARCH_0060
   :status: open
   :refines: REQ_0613, REQ_0621, BB_0072, BB_0073

   The CAN send path is the framework's zero-copy publish path
   (:need:`ARCH_0010`) terminated by a SocketCAN write. The codec
   writes envelope payload bytes directly into shared memory via
   ``Publisher::loan``; the gateway pulls them off the outbound
   bridge and constructs a ``CanFrame`` / ``CanFdFrame`` per
   :need:`REQ_0613`. No re-encoding occurs on the gateway side.

   .. mermaid::

      sequenceDiagram
        autonumber
        participant U as user code
        participant W as ChannelWriter
        participant P as Publisher (sonic-executor)
        participant SHM as iceoryx2 SHM
        participant S as Subscriber (gateway)
        participant GI as OutboundGatewayItem
        participant BR as Tokio bridge (per-iface outbound)
        participant SK as socketcan socket (iface)
        participant BUS as CAN bus

        U->>W: writer.send(&value)
        W->>P: publisher.loan(|slot| codec.encode(value, slot.payload))
        P->>SHM: zero-copy publish + notify
        SHM-->>S: WaitSet wakes
        S->>GI: ExecutableItem::execute()
        GI->>BR: bridge_tx.try_send(payload, routing)
        BR-->>SK: tokio task drains bridge
        SK->>SK: build CanFrame{id, ext, data} or CanFdFrame{id, brs, esi, data}
        SK->>BUS: write_frame()

.. architecture:: CAN receive path with multi-iface demux
   :id: ARCH_0061
   :status: open
   :refines: REQ_0614, REQ_0620, REQ_0622, REQ_0624, BB_0072, BB_0074

   The CAN receive path applies the per-interface kernel filter
   (compiled by :need:`BB_0074` from the union of registered
   readers' ``(can_id, mask, extended)``) before user space sees
   the frame. The gateway then demultiplexes each received frame
   to every channel whose filter matches, re-publishing the data
   bytes onto each matching reader's inbound iceoryx2 service.
   Error frames are siphoned off the same read into the health
   classifier and never reach a plugin channel (:need:`REQ_0631`,
   :need:`REQ_0636`).

   .. mermaid::

      sequenceDiagram
        autonumber
        participant BUS as CAN bus
        participant SK as socketcan socket (iface)
        participant RX as RX task (tokio)
        participant FC as filter / demux (BB_0074)
        participant HC as error classifier (BB_0072)
        participant BR as Tokio bridge (per-iface inbound)
        participant GI as InboundGatewayItem
        participant P as Publisher (gateway, in svc)
        participant SHM as iceoryx2 SHM
        participant S as Subscriber (app)
        participant R as ChannelReader
        participant U as user code

        BUS->>SK: arbitration + ACK, data frame matches kernel filter
        SK->>RX: read_frame() → CanFrame | CanFdFrame | error frame
        alt error frame
            RX->>HC: classify (passive / warning / bus-off)
            HC->>HC: update per-iface sub-state and aggregate via worst-of
        else data frame
            RX->>FC: lookup matching readers by (can_id, mask, ext)
            loop for each matching reader
                FC->>BR: enqueue (descriptor, data)
            end
            BR->>GI: sonic-executor Channel wakes item
            GI->>P: publisher.loan(|slot| copy payload bytes, set header)
            P->>SHM: zero-copy publish + notify
            SHM-->>S: WaitSet wakes
            S->>R: reader.try_recv() → Received{ value, header }
            R->>U: user code consumes value
        end

.. architecture:: CAN bus health and bus-off recovery
   :id: ARCH_0062
   :status: open
   :refines: REQ_0630, REQ_0632, REQ_0633, REQ_0634, REQ_0635, BB_0072

   Per-interface sub-state machine driven by error-frame
   classification; aggregated into the connector's single visible
   ``ConnectorHealth`` via worst-of (:need:`REQ_0630`). Bus-off
   closes the offending socket and arms
   ``ReconnectPolicy::next_delay()``; the reopen sequence
   re-applies the per-interface filter (:need:`BB_0074`) before
   the sub-state can transition back through ``Connecting``.

   .. mermaid::

      stateDiagram-v2
        [*] --> Connecting: iface socket bound, filter applied
        Connecting --> Up: first successful read or successful send
        Up --> Degraded: error-passive / error-warning frame received
        Degraded --> Up: no further error frames for recovery window
        Up --> Down: bus-off error frame received
        Degraded --> Down: bus-off escalation
        Down --> Connecting: ReconnectPolicy backoff elapses, socket reopened, filter re-applied
        Connecting --> Down: socket reopen / filter apply fails
        Up --> [*]: shutdown
        Down --> [*]: shutdown

   Aggregation rule (:need:`REQ_0630`): any iface ``Down`` while
   another remains ``Up`` surfaces as connector-level ``Degraded``;
   all ifaces ``Down`` surfaces as connector-level ``Down``. Every
   transition emits one ``HealthEvent`` (:need:`REQ_0635`) with
   the offending interface name in the payload.

----

7. Deployment view
------------------

The framework supports two deployment shapes from the same envelope
contract. Operators choose based on fault-isolation requirements; the
plugin's code is unchanged across both.

.. architecture:: In-process gateway deployment
   :id: ARCH_0020
   :status: open
   :refines: REQ_0240, REQ_0241

   One OS process; the host launches both the plugin's executor and a
   tokio task hosting ``MqttGateway``. SHM transport is in-process
   shared memory between two threads of the same process.

   .. mermaid::

      flowchart LR
        subgraph one_process[Single OS process]
          direction LR
          PLUGIN[Plugin executor<br/>sonic-executor]
          GATEWAY[Gateway tokio task<br/>rumqttc + bridge]
          SHM[(iceoryx2 SHM)]
          PLUGIN <--> SHM <--> GATEWAY
        end
        BROKER[(MQTT broker)]
        GATEWAY <--> BROKER

   **Pros.** Simpler ops (one binary, one signal handler, one log
   stream). No SHM pool sizing for a peer process. **Cons.** A panic
   in the tokio task aborts the application — loses :need:`QG_0001`.
   Recommended for development, examples, and small deployments where
   protocol-stack stability is trusted.

.. architecture:: Separate-process gateway deployment
   :id: ARCH_0021
   :status: open
   :refines: REQ_0240, REQ_0242

   Two OS processes; each runs its own sonic-executor. The plugin's
   process embeds only ``ConnectorHost``; the gateway's process
   embeds ``ConnectorGateway`` + the protocol stack. SHM transport is
   inter-process shared memory.

   .. mermaid::

      flowchart LR
        subgraph plugin_proc[Plugin process]
          PLUGIN[Plugin executor<br/>sonic-executor]
        end
        subgraph shm[iceoryx2 SHM]
          POOL[(shared memory pool)]
        end
        subgraph gw_proc[Gateway process]
          GATEWAY[Gateway executor + tokio<br/>rumqttc + bridge]
        end
        PLUGIN <--> POOL <--> GATEWAY
        BROKER[(MQTT broker)]
        GATEWAY <--> BROKER

   **Pros.** Full fault isolation — a panic in the gateway crashes the
   gateway only; the plugin observes ``HealthEvent::Down`` and the app
   stays alive. Independent restart policies. **Cons.** Two binaries
   to deploy and supervise; SHM pool sizing must be planned for the
   peer process; clean shutdown requires SIGINT to both halves.
   Recommended for production deployments where :need:`QG_0001`
   matters.

----

8. Crosscutting concepts
------------------------

These concepts cut across building blocks and runtime scenarios.

.. architecture:: Codec — compile-time generic
   :id: ARCH_0030
   :status: open
   :refines: ADR_0005, BB_0003

   Every connector instance is parameterised on its ``PayloadCodec``.
   Concrete connector types are
   ``MqttConnector<JsonCodec>``,
   ``MqttConnector<MsgPackCodec>`` (when feature-enabled), etc. The
   codec is invoked inside ``Publisher::loan`` so encoded bytes land
   directly in shared memory; on the receive side, ``decode`` runs
   over the borrowed payload slice. There is no intermediate
   serialised buffer.

.. architecture:: Error handling — single error type, explicit origins
   :id: ARCH_0031
   :status: open
   :refines: REQ_0213, REQ_0214

   ``ConnectorError`` is the framework's single error type. Each
   variant has exactly one origin point in the framework; routing of
   variants to user-visible vs. observable surfaces is explicit:

   .. list-table::
      :header-rows: 1
      :widths: 18 27 27 28

      * - Class
        - Originates in
        - Propagates as
        - Surfaces to user as
      * - ``Transport``
        - ``sonic-connector-transport-iox``
        - ``Result`` from ``send`` / ``try_recv``
        - ``Err`` from the call
      * - ``Codec``
        - ``sonic-connector-codec``
        - ``Result`` from ``encode`` / ``decode``
        - ``Err`` from ``send`` (encode) or ``try_recv`` (decode)
      * - ``Routing``
        - gateway, on inbound topic miss
        - ``HealthEvent::RoutingError``
        - observable; gateway never aborts
      * - ``PayloadOverflow``
        - ``ChannelWriter::send`` pre-loan check
        - ``Err`` from ``send``
        - typed; user resizes channel or splits payload
      * - ``Stack``
        - tokio task in gateway
        - ``HealthEvent::StackError`` + ``Down``; triggers reconnect
        - observable; recovers via ``ReconnectPolicy``
      * - ``BackPressure``
        - bridge ``try_send`` failure
        - ``Err`` from ``send`` + ``Degraded``
        - typed; caller retries or drops
      * - ``Down``
        - ``ChannelWriter::send`` pre-check
        - ``Err`` from ``send``
        - typed; caller decides drop vs. retry
      * - ``Shutdown``
        - host shutdown signal
        - ``Err`` from any in-flight op
        - unique variant — caller treats as graceful end

   No silent failures: every error class is either returned to the
   user or emitted as a ``HealthEvent``.

.. architecture:: Observability — Observer + ExecutionMonitor adapter
   :id: ARCH_0032
   :status: open
   :refines: REQ_0273, BB_0005

   The gateway is a sonic-executor consumer (:need:`ADR_0007`), so
   ``Observer::on_app_*`` and ``ExecutionMonitor::pre_execute`` /
   ``post_execute`` hooks already cover the gateway items.
   ``HealthEvent`` arrives on a dedicated sonic-executor
   ``Channel<HealthEvent>`` exposed by ``Connector::subscribe_health``.
   Behind a ``tracing`` cargo feature, ``sonic-connector-host``
   provides an adapter that maps both into ``tracing`` span events
   so a single ``RUST_LOG=...`` config controls the full stack.

.. architecture:: Back-pressure — explicit at every bounded buffer
   :id: ARCH_0033
   :status: open
   :refines: REQ_0260, REQ_0261

   Four bounded buffers participate; saturation surfaces explicitly at
   each. The framework never silently drops outbound user messages;
   inbound is protocol-bounded — drops are reported via
   ``HealthEvent::DroppedInbound`` rather than pretended-prevented.

   .. mermaid::

      flowchart LR
        U[user code] -->|send| W[ChannelWriter]
        W -->|loan/publish| SHM["iceoryx2 SHM<br/>(bounded queue)"]
        SHM -->|wakes| GI[GatewayItem]
        GI -->|try_send| BR1["Tokio bridge OUT<br/>(bounded mpsc)"]
        BR1 --> TT[Tokio task]
        TT -->|publish| B[Broker]
        B -->|publish| TT
        TT -->|send| BR2["Tokio bridge IN<br/>(bounded crossbeam)"]
        BR2 -->|wakes| GI2[InboundGatewayItem]
        GI2 -->|loan/publish| SHM2["iceoryx2 SHM<br/>(bounded queue)"]
        SHM2 --> R[ChannelReader]

----

9. Architecture decisions
-------------------------

The decisions ``ADR_0001`` through ``ADR_0010`` recorded in
:ref:`section 4 <connector-arc42-solution-strategy>` are the canonical
architecture decision log for this framework. This section is a
needtable view for quick browsing.

.. needtable::
   :types: arch-decision
   :columns: id, title, status, refines
   :show_filters:

----

10. Quality requirements
------------------------

The four quality goals (:need:`QG_0001`–:need:`QG_0004`) form the root
of the quality tree. Concrete quality scenarios that test them are
authored as ``test`` directives in :doc:`../verification/connector` —
the verification artefacts are the operational form of the quality
tree. A future spec round may add an explicit quality-tree
``architecture`` element if measurement targets (latency budgets,
throughput) become first-class.

----

11. Risks and technical debt
----------------------------

.. risk:: rumqttc API stability before 1.0
   :id: RISK_0001
   :status: open
   :links: BB_0021, ADR_0001

   ``rumqttc`` is the chosen MQTT crate but is pre-1.0; minor releases
   may break API. **Mitigation:** pin to a specific 0.x.y in
   ``Cargo.toml``; document the version in ``MqttConnectorOptions``
   docs; gate upgrades behind running the MQTT integration suite.

.. risk:: iceoryx2 0.8 pre-1.0 churn
   :id: RISK_0002
   :status: open
   :links: BB_0002, CON_0002

   iceoryx2 0.8.x is itself pre-1.0 and changes shape between minor
   versions. **Mitigation:** workspace pins ``iceoryx2 = "0.8"``;
   upgrades are an explicit follow-on effort across the entire
   workspace.

.. risk:: Const-generic monomorphisation cost
   :id: RISK_0003
   :status: open
   :links: BB_0010, ADR_0004

   ``ConnectorEnvelope<const N: usize>`` produces a distinct type per
   ``N``; an application with many channel sizes can grow code size.
   **Mitigation:** if profiling shows monomorphisation overhead, the
   plan-stage may substitute a small set of size-tier types (4 KB /
   64 KB / 1 MB) without breaking the external contract.

.. risk:: Tokio bridge latency
   :id: RISK_0004
   :status: open
   :links: BB_0022, ADR_0007

   The sonic-executor↔tokio bridge adds a channel hop on every
   message in both directions. **Mitigation:** the bridge stays in the
   gateway process (not crossing SHM); benchmarks at plan stage
   characterise added latency; if intolerable, a follow-on can move
   the rumqttc EventLoop directly onto a sonic-executor item triggered
   from a raw socket.

.. risk:: Wildcard demux pathological topic patterns
   :id: RISK_0005
   :status: open
   :links: REQ_0254, BB_0021

   MQTT wildcard subscriptions (``+``, ``#``) can produce many channel
   matches per inbound message. **Mitigation:** the gateway's demux
   structure (trie, flat-vec, hash-of-prefixes — chosen at plan stage)
   is proptest'd for equivalence; integration tests cover overlapping
   wildcard scenarios.

----

12. Glossary
------------

.. term:: Connector
   :id: GLOSS_0001
   :status: open

   A pair of (plugin, gateway) that bridges a sonic-executor
   application to one external protocol family (MQTT, OPC UA, gRPC,
   ADS, ...). One concrete crate per protocol; all connectors share
   the framework's five contracts.

.. term:: Plugin
   :id: GLOSS_0002
   :status: open

   The in-app side of a connector. A type implementing
   ``Connector`` that user code obtains channel handles from. Lives
   in the application's own process; speaks no network.

.. term:: Gateway
   :id: GLOSS_0003
   :status: open

   The out-of-app side of a connector. Hosts the actual protocol
   stack (e.g. ``rumqttc::EventLoop``) on a tokio runtime sidecar and
   exposes itself to sonic-executor as a set of ``ExecutableItem``
   instances. Deployed in-process or as a separate binary.

.. term:: ConnectorEnvelope
   :id: GLOSS_0004
   :status: open

   The on-wire form of every message crossing the plugin↔gateway
   boundary. ``#[repr(C)]`` POD with header + const-generic-sized
   payload. See :need:`BB_0010`.

.. term:: Codec
   :id: GLOSS_0005
   :status: open

   A type implementing ``PayloadCodec`` that converts user values to
   payload bytes and back. Selected at compile time as a generic
   parameter on the connector type. See :need:`BB_0003`,
   :need:`ARCH_0030`.

.. term:: Routing
   :id: GLOSS_0006
   :status: open

   A protocol-typed struct (``MqttRouting``, ``OpcUaRouting``, ...)
   embedded in ``ChannelDescriptor`` that tells the gateway how to
   address external endpoints (MQTT topic, OPC UA NodeId, gRPC
   method, ...). See :need:`ADR_0008`.

.. term:: Bridge
   :id: GLOSS_0007
   :status: open

   The bounded-channel pair connecting sonic-executor's WaitSet
   thread to the tokio runtime inside a connector crate. Outbound
   bridge is ``tokio::sync::mpsc``; inbound bridge is
   ``crossbeam_channel`` wired as a sonic-executor signal source.
   See :need:`BB_0022`.

.. term:: Health
   :id: GLOSS_0008
   :status: open

   The four-state observable lifecycle of a connector
   (``Up`` / ``Connecting`` / ``Degraded`` / ``Down``) emitted as
   ``HealthEvent`` on the connector's health channel. Uniform across
   protocols; see :need:`ARCH_0012`.

.. term:: Reconnect policy
   :id: GLOSS_0009
   :status: open

   A ``ReconnectPolicy`` implementation (default
   ``ExponentialBackoff``) used by connectors whose protocol stack
   exposes raw connect events. Stacks that manage reconnect
   internally do not use ``ReconnectPolicy`` but still emit
   ``HealthEvent`` (:need:`REQ_0235`).

.. term:: Channel
   :id: GLOSS_0010
   :status: open

   A logical bidirectional or unidirectional flow named by
   ``ChannelDescriptor::name``. Each channel direction maps to one
   iceoryx2 publish-subscribe service plus an event service for
   wakeups. Per-channel max payload size is fixed at
   service-creation time (:need:`ADR_0004`).

.. term:: ASIL
   :id: GLOSS_0011
   :status: open

   Automotive Safety Integrity Level (ISO 26262). Cited only for
   context in :need:`QG_0001` — the connector framework is a useful
   shape for keeping non-deterministic protocol code OUT of an
   ASIL-rated control loop, but the framework itself makes no safety
   integrity claims.

----

13. Implementations
-------------------

The framework's building blocks (:need:`BB_0001`, :need:`BB_0002`,
:need:`BB_0003`, :need:`BB_0005`) and the EtherCAT reference connector
(:need:`BB_0030` and its sub-blocks) ship as five workspace crates.
Each crate has its own ``impl::`` directive recording which BB it
realises, which requirements it refines, and any deviations from the
spec text that needed amendment during implementation.

.. impl:: sonic-connector-core crate
   :id: IMPL_0010
   :status: open
   :implements: BB_0001
   :refines: REQ_0201, REQ_0210, REQ_0213, REQ_0214, REQ_0221, REQ_0222, REQ_0230, REQ_0232, REQ_0233, REQ_0234

   **Crate.** ``crates/sonic-connector-core``. No iceoryx2 or
   tokio dependency; the crate is the framework's small-types layer.
   Depends on ``thiserror``, ``serde``, ``rand`` (jitter for
   ``ExponentialBackoff``); ``proptest`` dev-only.

   **Surface.**

   * ``Routing`` marker trait (``REQ_0222``).
   * ``ChannelDescriptor<R: Routing, const N: usize>`` with
     empty-name validation (``REQ_0201``, ``REQ_0221``).
   * ``PayloadCodec`` trait — encode / decode + ``format_name``
     (``REQ_0210``). Used as a generic-parameter constraint by
     concrete connectors.
   * ``ConnectorError`` — ``Codec`` / ``BackPressure`` /
     ``PayloadOverflow`` / ``InvalidDescriptor`` / ``Down`` /
     ``Stack`` (``REQ_0213``, ``REQ_0214``).
   * ``ConnectorHealth`` + ``ConnectorHealthKind`` + ``HealthEvent``
     + ``HealthMonitor`` + ``IllegalTransition`` — enforces the
     ARCH_0012 transition matrix; legal transitions emit one
     event, illegal pairs return ``IllegalTransition`` or panic in
     the panic-on-illegal helper (``REQ_0230``, ``REQ_0234``).
   * ``ReconnectPolicy`` trait + ``ExponentialBackoff`` /
     ``ExponentialBackoffBuilder`` — seedable RNG for
     deterministic tests; jitter / growth / max delay clamps at
     ``build()`` time (``REQ_0232``, ``REQ_0233``).

   **Tests.** TEST_0100 (``ExponentialBackoff`` invariants,
   proptest); TEST_0101 (state-machine transitions + illegal-pair
   rejection); TEST_0103 (``ChannelDescriptor`` validation).

.. impl:: sonic-connector-transport-iox crate
   :id: IMPL_0020
   :status: open
   :implements: BB_0002
   :refines: REQ_0200, REQ_0202, REQ_0203, REQ_0204, REQ_0205, REQ_0206, REQ_0214

   **Crate.** ``crates/sonic-connector-transport-iox``. Depends on
   ``sonic-connector-core``, ``iceoryx2``, ``serde``.

   **Surface.**

   * ``ConnectorEnvelope<const N: usize>`` — ``#[repr(C)]`` POD
     with ``ZeroCopySend`` (``REQ_0200``): sequence_number,
     timestamp_ns, 32-byte correlation_id, payload_len, reserved
     word, and inline ``[u8; N]`` payload buffer.
   * ``ChannelWriter<T, C, const N: usize>`` — typed publisher.
     ``send`` / ``send_with_correlation`` use
     ``Publisher::loan_uninit`` and a raw-pointer view of the
     inline payload array so codec writes hit shared memory
     directly with no intermediate user-side buffer (``REQ_0205``).
     Sequence numbers are claimed via ``fetch_add`` *only after*
     a successful codec encode, so failed sends do not advance
     the counter (``REQ_0202``, exercised by TEST_0125).
   * ``ChannelReader<T, C, const N: usize>`` — typed subscriber;
     ``try_recv`` surfaces codec failures as
     ``ConnectorError::Codec`` rather than silently dropping the
     envelope (``REQ_0214``).
   * ``ServiceFactory`` — opens / creates the iceoryx2 pub/sub
     service for a ``ChannelDescriptor`` (``REQ_0206``). The
     two-direction split mandated by ``REQ_0206`` (outbound vs
     inbound) is intentionally realised at the host layer
     (:need:`BB_0005`): each side constructs descriptors with
     a direction suffix, and ``ServiceFactory`` opens one
     service per descriptor.

   **Tests.** Integration tests against real iceoryx2 services:
   TEST_0120 (round-trip), TEST_0121 (sequence monotonicity),
   TEST_0122 (timestamp at send), TEST_0123 (correlation id
   verbatim + default zero), TEST_0125 (payload overflow rejection
   + no sequence advance).

.. impl:: sonic-connector-codec crate
   :id: IMPL_0030
   :status: open
   :implements: BB_0003
   :refines: REQ_0210, REQ_0212, REQ_0213, REQ_0214

   **Crate.** ``crates/sonic-connector-codec``. Re-exports
   ``PayloadCodec`` from ``sonic-connector-core``; ships
   ``JsonCodec`` behind a default-on ``json`` cargo feature
   (``REQ_0212``).

   **Surface.** ``JsonCodec`` writes directly into the
   caller-provided buffer via a tiny ``CountingWriter`` adapter
   wrapping ``serde_json::to_writer``. Buffer-too-small surfaces
   as ``ConnectorError::PayloadOverflow`` (with ``actual``
   computed on the error path via a fallback ``to_vec`` —
   the success path stays allocation-free); other serializer
   faults (non-string map keys, etc.) surface as
   ``ConnectorError::Codec`` carrying ``format = "json"`` and the
   underlying ``serde_json::Error`` (``REQ_0213``).

   Decode delegates to ``serde_json::from_slice``; truncated /
   wrong-shape / wrong-type / empty input all surface as
   ``ConnectorError::Codec`` rather than being silently dropped
   (``REQ_0214``).

   **Tests.** TEST_0110 (round-trip proptest), TEST_0111 (encode
   error paths — see the amended :need:`TEST_0111` text routing
   buffer-too-small to ``PayloadOverflow``), TEST_0112 (decode
   error paths).

.. impl:: sonic-connector-host crate
   :id: IMPL_0040
   :status: open
   :implements: BB_0005
   :refines: REQ_0220, REQ_0223, REQ_0231, REQ_0270, REQ_0271, REQ_0272, REQ_0273

   **Crate.** ``crates/sonic-connector-host``. Depends on
   ``sonic-connector-core``, ``sonic-connector-transport-iox``,
   ``sonic-executor``, ``crossbeam-channel``. Optional
   ``sonic-executor-tracing`` dep behind a default-off ``tracing``
   feature (``REQ_0273``).

   **Surface.**

   * ``Connector`` trait — associated ``Routing`` / ``Codec``
     types; methods ``name`` / ``health`` / ``subscribe_health``
     / ``register_with`` / ``create_writer<T, N>`` /
     ``create_reader<T, N>`` (``REQ_0220``, ``REQ_0223``). Not
     dyn-compatible — concrete connectors plug into the host one
     at a time via ``ConnectorHost::register<C: Connector>``.
   * ``HealthSubscription`` — receive-only handle wrapping a
     ``crossbeam_channel::Receiver<HealthEvent>``. Per the
     amended :need:`REQ_0231`, this is the in-process
     implementation of the spec's "observable handle" contract;
     the alternative cross-process form using
     ``sonic_executor::Channel<HealthEventWire>`` is deferred
     until a real connector exercises out-of-process health
     observation.
   * ``ConnectorHost::builder()`` + ``register`` +
     ``run`` / ``run_for`` / ``run_n`` (``REQ_0270``,
     ``REQ_0272``). Owns the underlying
     ``sonic_executor::Executor`` and exposes a ``Stoppable``
     handle for external shutdown.
   * ``ConnectorGateway::builder()`` — parallel construction for
     the gateway side (``REQ_0271``).

   **Deviation from :need:`REQ_0273`.** The default-off
   ``tracing`` cargo feature is wired (deps pull
   ``sonic-executor-tracing`` when the feature is on); the
   ``Observer`` adapter implementation that forwards
   ``HealthEvent`` and ``ExecutionMonitor`` callbacks through the
   global ``tracing`` subscriber is deferred until a real
   connector emits HealthEvents on a tracing subscriber under
   load. Tracked for a follow-on implementation commit.

   **Tests.** Integration test using a minimal in-tree
   ``EchoConnector`` exercises the full host
   register → run → executable-item-driven loop and confirms
   ``HealthSubscription`` delivers events published on the
   connector's internal health channel.

.. impl:: sonic-connector-ethercat crate (C5a–C5e + C7a)
   :id: IMPL_0050
   :status: open
   :implements: BB_0030
   :refines: REQ_0310, REQ_0311, REQ_0312, REQ_0313, REQ_0314, REQ_0315, REQ_0316, REQ_0317, REQ_0318, REQ_0319, REQ_0320, REQ_0321, REQ_0322, REQ_0323, REQ_0324, REQ_0325, REQ_0326, REQ_0327, REQ_0328

   **Crate.** ``crates/sonic-connector-ethercat``. Default deps:
   ``sonic-connector-core``, ``sonic-connector-transport-iox``,
   ``sonic-connector-host``, ``sonic-executor``,
   ``crossbeam-channel``, ``tokio`` (``rt`` +
   ``rt-multi-thread`` + ``macros`` + ``sync``). Optional
   ``ethercrab`` dep behind the default-off ``bus-integration``
   cargo feature.

   **Status.** C5a + C5b land the protocol-agnostic core:
   routing, options builder, bridges, health monitor, tokio
   runtime lifecycle, ``Connector`` trait impl, and the
   pure-logic helpers (``sdo`` / ``scheduler`` / ``wkc``) that
   carry the gateway's load-bearing decision logic. C5c pulls
   ``ethercrab`` 0.7 as an optional dep and ships the
   forward-compatible declarations (``bus::EthercatPduStorage``
   type alias + ``declare_pdu_storage!`` macro) every
   application that wants real-bus deployment needs to declare
   anyway. The cycle-loop wiring against
   ``ethercrab::MainDevice`` was scoped to C5c but pulled back
   when ``ethercrab`` 0.7's actual API surface diverged from the
   examples reachable via documentation search; writing 1000+
   lines of speculative integration code against an API the
   author can't iterate against would have produced code that
   compiles but whose runtime behaviour is unverified — exactly
   the trust-me-but-untested posture the framework otherwise
   avoids.

   C5d takes the second path: defines the ``BusDriver`` trait
   that abstracts over "the operations the cycle loop needs from a
   real EtherCAT bus", ships an in-tree ``MockBusDriver`` that
   makes the cycle loop exhaustively testable without hardware,
   and a ``CycleRunner`` that composes ``CycleScheduler``,
   ``BusDriver``, ``evaluate_wkc``, and ``EthercatHealthMonitor``
   into one cycle-driving unit.

   C5e lands ``EthercrabBusDriver`` — a concrete ``BusDriver``
   wrapping ``ethercrab::MainDevice`` against ethercrab 0.7's
   API. The integration is **compile-checked only**: no EtherCAT
   hardware is available at the time of authoring, so runtime
   behaviour is unverified. The hardware-gated integration test
   under ``tests/ethercrab_driver.rs`` (``#[ignore]``-marked,
   gated on ``ETHERCAT_TEST_NIC``) documents the bring-up + cycle
   pattern and is one ``--ignored`` flag away from running on a
   Linux gateway host with ``CAP_NET_RAW``. End-to-end
   verification waits on hardware arrival and a follow-on commit
   to capture any API mismatches surfaced by the first real-bus
   run.

   **Surface.**

   * ``EthercatRouting`` — typed routing identifying one
     process-data slice by SubDevice address, PDO direction, bit
     offset, bit length. Implements ``Routing`` (``REQ_0311``).
   * ``EthercatConnectorOptions`` typed builder —
     ``cycle_time`` (default 2 ms, min 1 ms clamp; ``REQ_0316``),
     ``distributed_clocks`` opt-in (``REQ_0318``), bounded
     bridge capacities (``REQ_0322``), network interface name,
     ``&'static [SubDeviceMap]`` PDO descriptor (``REQ_0314``,
     :need:`ADR_0027`), tokio worker-thread count
     (:need:`ADR_0026`).
   * ``OutboundBridge<T>`` — bounded; saturation surfaces as
     ``OutboundError::BackPressure(T)`` (``REQ_0323``).
   * ``InboundBridge<T>`` — bounded; saturation drops the
     message and bumps a running count so the gateway can emit
     ``HealthEvent::DroppedInbound { count }`` (``REQ_0324``).
   * ``EthercatHealthMonitor`` — thread-safe wrapper around
     ``HealthMonitor`` that broadcasts every legal transition
     over a ``crossbeam_channel``.
   * ``EthercatGateway`` — owns its tokio runtime
     (multi-thread, default 1 worker per :need:`ADR_0026`) and
     joins it on ``Drop`` with a 5-second budget mirroring
     :need:`ARCH_0013` (``REQ_0321``).
   * ``EthercatConnector<D: BusDriver, C: PayloadCodec>`` —
     implements the framework ``Connector`` trait (``REQ_0310``).
     ``create_writer`` / ``create_reader`` open the plugin-side
     iceoryx2 service named ``"{descriptor.name()}.out"`` /
     ``".in"``, open the paired gateway-side raw port on the same
     service, and register the channel in the shared
     ``ChannelRegistry`` (``REQ_0223`` + ``REQ_0328``).
     ``register_with`` (C7b) takes the configured driver out of the
     connector and spawns ``dispatcher_loop`` on the gateway's
     tokio runtime (``REQ_0321``); the framework still receives a
     heartbeat ``ExecutableItem`` for ``REQ_0272``.
   * ``sdo::pdo_sdo_writes`` — pure function producing the
     ordered SDO write sequence (clear → entries → set-count
     on indices ``0x1C12`` and ``0x1C13``) that the gateway
     applies during the PRE-OP → SAFE-OP transition
     (``REQ_0315``).
   * ``scheduler::CycleScheduler`` — pure-clock pacing decision
     with skip-not-catch-up semantics; 10-cycle clock jump
     produces exactly one fire (``REQ_0317``).
   * ``wkc::evaluate_wkc`` + ``WkcVerdict::degraded_reason`` —
     working-counter health policy (``REQ_0319``, ``REQ_0320``).
   * ``driver::BusDriver`` — async trait abstracting the
     bring-up + per-cycle operations the runner needs from a
     concrete back-end (``REQ_0312`` / ``REQ_0313`` / ``REQ_0315``
     are encoded in the contract; concrete impls cover them).
     C7a extends the trait with callback-shaped
     ``with_subdevice_outputs_mut`` / ``with_subdevice_inputs``
     methods that expose one SubDevice's PDI region; the
     callback shape keeps ethercrab's internal ``PdiWriteGuard``
     lifetime scoped to the impl (``REQ_0326``, ``REQ_0327``).
   * ``mock::MockBusDriver`` — programmable test fixture: WKC
     sequences, configurable bring-up response, bring-up
     failure injection. C7a extends with per-SubDevice
     PDI buffers (``with_subdevice_outputs`` /
     ``with_subdevice_inputs`` builders + ``Mutex``-backed
     interior storage for the callback methods).
   * ``runner::CycleRunner<D: BusDriver>`` — composes
     ``CycleScheduler`` + ``BusDriver`` + ``evaluate_wkc`` +
     ``EthercatHealthMonitor``. End-to-end tested via
     ``MockBusDriver``.
   * ``pdi::write_routing`` / ``pdi::read_routing`` — pure-logic
     bit-slice translation between a per-SubDevice PDI buffer
     and a codec-encoded byte payload, honouring REQ_0311's
     ``bit_offset`` / ``bit_length``. Read-modify-write on
     partial leading / trailing bytes preserves adjacent
     slices (``REQ_0326``, ``REQ_0327``).
   * ``registry::ChannelRegistry`` — Vec-backed registry of
     ``RegisteredChannel { descriptor_name, routing,
     direction, binding }``. C7b extends ``ChannelBinding`` with
     ``Outbound(Box<dyn OutboundDrain>)`` and
     ``Inbound(Box<dyn InboundPublish>)`` variants carrying the
     gateway-side iceoryx2 ports (trait objects erase the
     channel's user-type ``T`` and codec ``C``).
     Insertion-order iteration verified by TEST_0219; per-cycle
     ``iter()`` is allocation-free (verified via
     ``CountingAllocator`` across 1 000 cycles × 16 channels —
     ``REQ_0328``).
   * ``dispatcher::dispatch_one_cycle`` /
     ``dispatcher::dispatcher_loop`` (C7b) — gateway-side
     byte-shovel composing ``pdi::write_routing`` /
     ``pdi::read_routing`` + ``ChannelRegistry`` + the iceoryx2
     raw pub/sub ports. ``dispatch_one_cycle`` is the
     single-iteration synchronous form used by the
     ``TEST_0220`` / ``TEST_0221`` / ``TEST_0222`` integration
     tests; ``dispatcher_loop`` is the long-running ``async fn``
     spawned by ``register_with``. The trait-object wrappers
     ``IoxOutboundDrain<N>`` / ``IoxInboundPublish<N>`` adapt the
     raw iceoryx2 reader / writer to ``OutboundDrain`` /
     ``InboundPublish`` (``REQ_0326``, ``REQ_0327``, ``REQ_0328``).
   * ``raw::RawChannelWriter<N>`` / ``raw::RawChannelReader<N>``
     in ``sonic-connector-transport-iox`` — byte-only iceoryx2
     ports used by the dispatcher. ``send_raw_bytes`` /
     ``try_recv_into`` bypass the codec entirely, keeping the
     dispatcher hot path codec-free (``REQ_0327`` amended in
     C7b).

   **Verification posture.** Every REQ covered by IMPL_0050 has
   a passing unit / integration test on every CI push.
   ``EthercrabBusDriver`` provides the real-bus path for
   REQ_0312 (single MainDevice — one ``PduStorage::try_split``
   per driver), REQ_0313 (bus reaches OP — ``group.into_op``
   fast path), REQ_0314 + REQ_0315 (PDO mapping applied via
   ``pdo_sdo_writes`` + ``sdo_write`` during PRE-OP), and
   REQ_0325 (Linux raw socket — ``tx_rx_task``); those tests
   await physical hardware under ``ETHERCAT_TEST_NIC``.
   ``REQ_0326`` / ``REQ_0327`` / ``REQ_0328``'s end-to-end byte
   hops are exercised against ``MockBusDriver`` via the C7b
   integration tests ``TEST_0220`` (outbound),
   ``TEST_0221`` (inbound), and ``TEST_0222`` (loopback
   round-trip), so the iceoryx2 ↔ PDI ↔ iceoryx2 pipeline is
   green in every CI run without hardware.

   **Tests.** Cases pass: TEST_0201 (routing round-trip),
   TEST_0204 + TEST_0206 (options builder), TEST_0205-partial
   (SDO write sequence shape), TEST_0207 (cycle scheduler
   skip-not-catch-up), TEST_0208 (DC opt-in flag), TEST_0209 +
   TEST_0210 (WKC policy), TEST_0211-partial (gateway tokio
   runtime ownership and clean drop), TEST_0212-0214 (bridge
   bounded capacity, BackPressure, DroppedInbound), TEST_0216-
   0218 (PDI bit-slice byte-aligned / unaligned round-trips,
   adjacent-slice preservation), TEST_0219 (registry
   alloc-free iter), TEST_0220 (outbound end-to-end), TEST_0221
   (inbound end-to-end), TEST_0222 (loopback round-trip via
   mock), plus surface-shape checks for TEST_0200 (Connector
   trait surface, ``create_writer`` / ``create_reader``
   registration semantics).

.. impl:: sonic-connector-zenoh crate (planned)
   :id: IMPL_0060
   :status: draft
   :implements: BB_0040
   :refines: REQ_0400, REQ_0401, REQ_0402, REQ_0403, REQ_0404, REQ_0405, REQ_0406, REQ_0407, REQ_0408, REQ_0420, REQ_0421, REQ_0422, REQ_0423, REQ_0424, REQ_0425, REQ_0426, REQ_0427, REQ_0428, REQ_0440, REQ_0442, REQ_0443, REQ_0444, REQ_0445, REQ_0446

   **Crate.** ``crates/sonic-connector-zenoh`` (planned; not yet
   scaffolded). Default deps: ``sonic-connector-core``,
   ``sonic-connector-transport-iox``, ``sonic-connector-host``,
   ``sonic-executor``, ``crossbeam-channel``, ``tokio`` (``rt`` +
   ``rt-multi-thread`` + ``macros`` + ``sync``). Optional
   ``zenoh`` dep behind the default-off ``zenoh-integration``
   cargo feature (:need:`REQ_0444`).

   **Status.** Planned surface only — the crate has not been
   scaffolded. This directive locks in the public API that the
   forthcoming implementation will be measured against. Once the
   crate exists, its surface, dependencies, and test coverage are
   reconciled against this directive; divergences are recorded as
   amendments. Status flips from ``draft`` to ``open`` after the
   first complete in-tree implementation lands and the public
   surface matches the bulleted list below.

   **Surface.**

   * ``ZenohRouting`` — typed routing carrying the Zenoh key
     expression and pub/sub QoS knobs (``key_expr``,
     ``congestion_control``, ``priority``, ``reliability``,
     ``express``). Implements ``Routing``. ``key_expr`` is
     validated on the plugin side at ``create_writer`` /
     ``create_reader`` / ``create_querier`` /
     ``create_queryable`` entry; invalid expressions surface as
     ``ConnectorError::Configuration`` (``REQ_0401``,
     :need:`ADR_0042`).
   * ``ZenohConnectorOptions`` typed builder — ``mode``
     (``SessionMode::{Peer, Client, Router}``, default ``Peer``;
     ``REQ_0440``), ``connect`` / ``listen`` locator lists
     surfaced verbatim to ``zenoh::Config`` (``REQ_0443``),
     ``query_target`` / ``query_consolidation`` / ``query_timeout``
     defaults for queriers (``REQ_0425``), bounded bridge
     capacities (``REQ_0404``), optional ``min_peers`` knob
     governing ``Degraded`` health transitions.
   * ``OutboundBridge<T>`` — bounded; saturation surfaces as
     ``ConnectorError::BackPressure`` and flips health to
     ``Degraded`` (``REQ_0405``).
   * ``InboundBridge<T>`` — bounded; saturation drops the offending
     Zenoh sample or reply chunk and bumps a running count so the
     gateway emits ``HealthEvent::DroppedInbound { count }``
     (``REQ_0406``, ``REQ_0428``).
   * ``ZenohSessionLike`` — trait abstracting over ``zenoh::Session``
     vs ``MockZenohSession`` so the gateway is compile-time
     monomorphised against either back-end. Methods:
     ``declare_publisher`` / ``declare_subscriber`` / ``get`` /
     ``declare_queryable`` / ``session_state``.
   * ``MockZenohSession`` — in-process pub/sub + query loopback
     implementing ``ZenohSessionLike``; ships in the default
     build, not gated by ``zenoh-integration`` (``REQ_0445``).
     Programmable session-state sequences for health-state tests.
   * ``RealZenohSession`` — thin wrapper over ``zenoh::Session``
     created via ``zenoh::open(config)``; lives behind the
     ``zenoh-integration`` cargo feature.
   * ``ZenohGateway<S: ZenohSessionLike, C: PayloadCodec>`` —
     owns one session, the per-channel routing registry, the
     bounded bridges, the tokio runtime hosting Zenoh callbacks
     (``REQ_0403``), and the ``correlation_id → zenoh::Query``
     map for in-flight queryable reply streams (``REQ_0426``).
     Observes session-alive ↔ session-closed transitions from
     ``S`` and emits one ``HealthEvent`` per transition
     (``REQ_0442``). No ``ReconnectPolicy``
     (:need:`ADR_0041` / :need:`REQ_0441`).
   * ``ZenohConnector<C: PayloadCodec>`` — implements the
     framework ``Connector`` trait with ``type Routing =
     ZenohRouting`` and ``type Codec = C`` (``REQ_0400``).
     ``create_writer`` / ``create_reader`` open the plugin-side
     iceoryx2 services named ``"{descriptor.name()}.out"`` /
     ``".in"`` (``REQ_0407``, ``REQ_0408``). Beyond the trait,
     ``ZenohConnector`` exposes two **concrete** non-trait
     methods — ``create_querier<Q, R, …>`` and
     ``create_queryable<Q, R, …>`` — returning Zenoh-specific
     handle types (``REQ_0420``, :need:`ADR_0040`).
   * ``ZenohQuerier<Q, R, C, N>`` — non-trait query-initiation
     handle. ``send(q)`` mints a fresh ``QueryId``, encodes ``Q``
     via the connector's codec, stamps the ``QueryId`` into the
     envelope's ``correlation_id`` (``REQ_0421``), and publishes
     on ``"{name}.query.out"``; returns the ``QueryId`` for
     reply demultiplexing. ``try_recv`` drains
     ``"{name}.reply.in"``, decoding the 1-byte frame
     discriminator (``0x01`` data, ``0x02`` end-of-stream,
     ``0x03`` gateway-synthetic timeout per :need:`ADR_0043` /
     ``REQ_0424``). Per-call ``send_with_timeout`` overrides
     the session-wide default (``REQ_0425``).
   * ``ZenohQueryable<Q, R, C, N>`` — non-trait query-handling
     handle. ``try_recv`` surfaces ``(QueryId, Q)`` decoded from
     ``"{name}.query.in"``. ``reply(id, r)`` encodes ``R``
     into ``envelope.payload[1..]`` with byte ``[0] = 0x01``
     and publishes on ``"{name}.reply.out}"``; callable zero
     or more times per ``QueryId`` (``REQ_0423``, ``REQ_0427``).
     ``terminate(id)`` publishes a ``0x02`` envelope; the
     gateway drops the corresponding ``zenoh::Query`` handle,
     finalising the upstream reply stream (``REQ_0426``).
     Matching of ``QueryId`` to ``zenoh::Query`` lives inside
     this type and the gateway — never the framework
     (preserves :need:`REQ_0290`, ``REQ_0422``).
   * Reply framing — every envelope on ``"{name}.reply.out"`` /
     ``"{name}.reply.in"`` carries a 1-byte Zenoh-private
     discriminator at ``payload[0]``: ``0x01`` data chunk
     (followed by codec-encoded ``R``), ``0x02`` end of stream,
     ``0x03`` gateway-synthetic timeout terminator. The
     framework's ``ConnectorEnvelope`` reserved word stays
     untouched (``REQ_0424``, :need:`ADR_0043`).
   * Codec is a generic parameter, compile-time-monomorphised
     (re-affirms :need:`REQ_0211`); ``JsonCodec`` is the default
     wiring in examples (``REQ_0402``, re-affirms
     :need:`REQ_0212`). Gateway-side dispatching forwards raw
     bytes only — codecs run plugin-side on both pub/sub and
     query paths (``REQ_0408``, ``REQ_0427``).
   * Cross-platform — Linux, macOS, and Windows are supported
     host operating systems for both plugin and gateway
     (``REQ_0446``); no Linux-specific socket capability is
     required (contrast :need:`REQ_0325` for EtherCAT).

   **Tests.** The corpus authored alongside this directive in
   :doc:`../verification/connector` includes TEST_0300
   (``ZenohRouting`` validation), TEST_0301 (``Connector`` trait
   surface), TEST_0302 (pub/sub end-to-end via
   ``MockZenohSession``), TEST_0303 (query round-trip via
   ``MockZenohSession``), TEST_0304 (codec failure paths),
   TEST_0305 / TEST_0306 (bridge saturation), TEST_0307
   (query-timeout terminator), TEST_0308 (health state machine),
   TEST_0309 (anti-req — no ``ReconnectPolicy``), TEST_0310
   (cargo-feature dep gating), TEST_0311 (cross-platform build
   matrix), TEST_0314 (tokio sidecar containment). Layer-2 /
   Layer-3 cases ``TEST_0312`` (two-peer real session) and
   ``TEST_0313`` (client-mode router smoke) remain
   ``:status: draft`` until the ``zenoh-integration`` and
   ``ZENOH_TEST_ROUTER`` CI jobs land.

.. impl:: sonic-connector-can crate (planned)
   :id: IMPL_0080
   :status: draft
   :implements: BB_0070
   :refines: REQ_0600, REQ_0601, REQ_0602, REQ_0603, REQ_0604, REQ_0605, REQ_0606, REQ_0607, REQ_0608, REQ_0610, REQ_0611, REQ_0612, REQ_0613, REQ_0614, REQ_0615, REQ_0620, REQ_0621, REQ_0622, REQ_0623, REQ_0624, REQ_0625, REQ_0630, REQ_0631, REQ_0632, REQ_0633, REQ_0634, REQ_0635, REQ_0636

   **Crate.** ``crates/sonic-connector-can`` (planned; not yet
   scaffolded). Default deps: ``sonic-connector-core``,
   ``sonic-connector-transport-iox``, ``sonic-connector-host``,
   ``sonic-executor``, ``crossbeam-channel``, ``tokio`` (``rt`` +
   ``rt-multi-thread`` + ``macros`` + ``sync``). Optional
   ``socketcan`` dep (with its ``tokio`` feature) behind the
   default-off ``socketcan-integration`` cargo feature
   (:need:`REQ_0603`).

   **Status.** Planned surface only — the crate has not been
   scaffolded. This directive locks in the public API that the
   forthcoming implementation will be measured against. Once the
   crate exists, its surface, dependencies, and test coverage are
   reconciled against this directive; divergences are recorded as
   amendments. Status flips from ``draft`` to ``open`` after the
   first complete in-tree implementation lands and the public
   surface matches the bulleted list below.

   **Surface.**

   * ``CanIface`` — bounded ASCII string newtype of
     ``IFNAMSIZ`` − 1 = 15 bytes, validated on construction;
     wraps the Linux network interface name (``can0``, ``vcan0``,
     etc.). Implements ``Copy``, ``Eq``, ``Hash``.
   * ``CanId { value: u32, extended: bool }`` — typed identifier
     newtype carrying both 11-bit (standard) and 29-bit
     (extended) CAN identifiers; the ``extended`` flag is
     preserved end-to-end (:need:`REQ_0615`). Constructors
     ``CanId::standard(u16)`` / ``CanId::extended(u32)`` enforce
     the per-form bit-width invariant.
   * ``CanFrameKind`` — enum ``{ Classical, Fd }``. Determines
     ``ChannelDescriptor::max_payload_size`` deterministically
     (8 / 64 bytes per :need:`REQ_0612`).
   * ``CanFdFlags`` — bitflags ``{ BRS, ESI }`` carried in
     ``CanRouting`` for FD channels; ignored when
     ``kind == Classical``.
   * ``CanRouting`` — typed routing carrying ``iface``, ``can_id``,
     ``mask: u32``, ``kind``, ``fd_flags``. Implements ``Routing``
     (:need:`REQ_0601`). Plugin-side validation runs inside
     ``create_writer`` / ``create_reader`` before any iceoryx2
     service is created — invalid iface or payload-kind mismatch
     yields ``ConnectorError::Configuration``.
   * ``CanConnectorOptions`` typed builder — ``ifaces: Vec<CanIface>``
     listing the gateway-owned interfaces (:need:`REQ_0620`),
     ``outbound_bridge_capacity`` / ``inbound_bridge_capacity``
     (:need:`REQ_0606`), ``reconnect_policy: Box<dyn ReconnectPolicy>``
     with ``ExponentialBackoff::default()`` (:need:`REQ_0634`),
     and a ``recovery_window: Duration`` controlling the
     error-passive → Up debounce on :need:`ARCH_0062`.
   * ``OutboundBridge<T>`` — bounded per-iface; saturation
     surfaces as ``ConnectorError::BackPressure`` and flips
     health to ``Degraded`` (:need:`REQ_0607`).
   * ``InboundBridge<T>`` — bounded per-iface; saturation drops
     the offending CAN frame and bumps a running count so the
     gateway emits ``HealthEvent::DroppedInbound { count }``
     (:need:`REQ_0608`).
   * ``CanInterfaceLike`` — trait abstracting over real
     SocketCAN sockets vs ``MockCanInterface`` so the gateway is
     compile-time monomorphised against either back-end.
     Methods: ``send_classical`` / ``send_fd`` / ``recv`` /
     ``apply_filter`` / ``state``. ``recv`` returns an enum
     mirroring the upstream ``socketcan::CanFrame`` discriminant
     (``Data | Remote | Error``) so error frames are surfaced on
     the same call as data frames and routed to the gateway's
     classifier without a separate code path.
   * ``MockCanInterface`` — in-process loopback implementing
     ``CanInterfaceLike``; ships in the default build, not gated
     by ``socketcan-integration`` (:need:`REQ_0604`). Programmable
     error-frame injection for testing
     ``Connecting → Up → Degraded → Down → Connecting`` walks
     against :need:`ARCH_0062`.
   * ``RealCanInterface`` — thin wrapper over the upstream
     ``socketcan`` crate's async sockets
     (``socketcan::tokio::CanSocket`` for classical,
     ``socketcan::tokio::CanFdSocket`` for FD); lives behind the
     ``socketcan-integration`` cargo feature, which also enables
     the upstream crate's ``tokio`` feature. Owns the
     ``CAP_NET_RAW`` socket bind and the ``CAN_RAW_ERR_FILTER``
     ``setsockopt`` for error-frame reporting (:need:`REQ_0631`).
     The Linux raw-socket smoke test (:need:`TEST_0512`) follows
     the upstream crate's ``vcan_tests`` posture: gate the CI job
     on the ``vcan`` kernel module being loaded
     (``./scripts/vcan.sh``) so the test only runs where ``vcan0``
     actually exists.
   * ``PerIfaceFilter`` (pure-logic helper, :need:`BB_0074`) —
     compiles the union of ``(can_id, mask, extended)`` tuples
     from registered readers into a single
     ``Vec<libc::can_filter>`` (or the ``socketcan`` crate's
     equivalent) suitable for one ``setsockopt(SOL_CAN_RAW,
     CAN_RAW_FILTER, …)`` call. Symmetric counterpart for the
     demux side: ``matching_readers(&frame) → impl Iterator<Reader>``
     so every matching reader gets its own envelope copy
     (:need:`REQ_0624`).
   * ``CanGateway<I: CanInterfaceLike, C: PayloadCodec>`` —
     owns one ``I`` per configured iface, per-iface routing
     registries, the bounded bridges, the tokio runtime hosting
     RX/TX tasks (:need:`REQ_0605`), and the per-iface
     ``HealthSubState`` map aggregated via worst-of
     (:need:`REQ_0630`). Observes error frames and updates
     ``ConnectorHealth`` via :need:`ARCH_0062`; emits one
     ``HealthEvent`` per transition (:need:`REQ_0635`).
     ``ReconnectPolicy`` is owned at this layer
     (:need:`REQ_0634`).
   * ``CanConnector<C: PayloadCodec>`` — implements the framework
     ``Connector`` trait with ``type Routing = CanRouting`` and
     ``type Codec = C`` (:need:`REQ_0600`). ``create_writer`` /
     ``create_reader`` open the plugin-side iceoryx2 services
     named ``"{descriptor.name()}.out"`` / ``".in"`` and, on the
     gateway side, trigger a per-iface filter recompute
     (:need:`REQ_0623`).
   * Linux-only real I/O — the gateway requires Linux ``PF_CAN``
     and the ``CAP_NET_RAW`` capability (:need:`REQ_0602`,
     mirrors :need:`REQ_0325`). The plugin-side ``CanConnector``
     and ``MockCanInterface`` stay portable to macOS and Windows
     for layer-1 development; the ``socketcan-integration``
     feature is the Linux gate.
   * Gateway-side dispatching forwards raw bytes only — codecs
     run plugin-side on both send and receive paths
     (:need:`REQ_0614`, mirrors :need:`REQ_0408` /
     :need:`REQ_0327`).

   **Tests.** The corpus authored alongside this directive in
   :doc:`../verification/connector` includes TEST_0500
   (``CanConnector`` trait surface), TEST_0501 (``CanRouting``
   field round-trip), TEST_0502 / TEST_0503 (classical and FD
   round-trip via ``MockCanInterface``), TEST_0504 (per-iface
   filter union), TEST_0505 (multi-iface inbound demux),
   TEST_0506 (bus-off → Down → ReconnectPolicy reopen),
   TEST_0507 (error-passive → Degraded → recovery), TEST_0508
   (tokio sidecar containment), TEST_0509 / TEST_0510 (bridge
   saturation), TEST_0511 (cargo-feature dep gating), TEST_0513
   (anti-req — error frames not on plugin channel), TEST_0514
   (per-iface registry alloc-free iteration). Layer-2
   ``TEST_0512`` (Linux raw-socket smoke against ``vcan0``)
   remains ``:status: draft`` until the
   ``socketcan-integration`` CI job and the kernel ``vcan``
   module are wired into CI.

----

Cross-cutting traceability
--------------------------

.. needtable::
   :types: building-block
   :columns: id, title, status, implements
   :show_filters:

.. needtable::
   :types: architecture
   :columns: id, title, status, refines
   :show_filters:
