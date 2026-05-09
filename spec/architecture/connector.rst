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
        APP <-- ConnectorEnvelope --> SHM
        SHM <-- ConnectorEnvelope --> GW
        GW <-- protocol native --> EXT

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
   :refines: BB_0001, BB_0002, BB_0003, BB_0004, BB_0005

   Crate-level building blocks and their dependency graph. All edges
   point from depender to dependee. The graph is acyclic; the host is
   the only consumer of every other new crate.

   .. mermaid::

      flowchart TB
        subgraph existing[existing crates]
          EX[sonic-executor]
          TR[sonic-executor-tracing]
        end
        subgraph new[new crates - this spec]
          CO[sonic-connector-core<br/>BB_0001]
          TX[sonic-connector-transport-iox<br/>BB_0002]
          CD[sonic-connector-codec<br/>BB_0003]
          MQ[sonic-connector-mqtt<br/>BB_0004]
          HO[sonic-connector-host<br/>BB_0005]
        end
        CO --> TX
        CO --> CD
        CO --> MQ
        TX --> MQ
        CD --> MQ
        EX --> TX
        EX --> MQ
        CO --> HO
        TX --> HO
        CD --> HO
        MQ --> HO
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
        participant W as ChannelWriter&lt;T&gt;
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
        participant R as ChannelReader&lt;T&gt;
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
        Down --> Connecting: ReconnectPolicy::next_delay() elapses
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
