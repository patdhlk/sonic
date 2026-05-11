Connector framework
===================

This page captures the requirements for `sonic-connector`: a framework that
connects sonic-executor applications to external protocols (MQTT, OPC UA,
gRPC, fieldbus) through a controlled boundary, so messy network code lives
outside the application's deterministic core.

The decomposition is two-tier:

* **Top-level umbrella feature** — :need:`FEAT_0030` — peer to
  :need:`FEAT_0010`. Sonic-connector is a general-purpose framework usable
  by any sonic-executor consumer; it is not bound to the PLC use case.
* **Capability-cluster sub-features** — one per architectural concern, each
  ``:satisfies:`` :need:`FEAT_0030`.
* **Requirements** — concrete shall-clauses that ``:satisfies:`` a
  capability-cluster feature.

This round covers the framework core plus an MQTT reference connector
(``rumqttc``-backed). OPC UA, gRPC, and Beckhoff ADS connectors are
deferred to follow-on specs that will reuse the same five contracts.

Top-level umbrella
------------------

.. feat:: Connector framework
   :id: FEAT_0030
   :status: open

   A Rust framework that bridges sonic-executor applications to external
   protocols through a typed envelope carried over iceoryx2 shared memory.
   The framework provides five contracts — envelope, codec, routing, health,
   lifecycle — that every protocol connector instantiates as a plugin
   (in-app side) and a gateway (out-of-app side). Both halves are
   sonic-executor ``ExecutableItem`` consumers; protocol-specific async
   work runs on a tokio sidecar contained inside each connector crate.

   Deployment chooses whether the gateway runs as a tokio task in-process
   alongside the plugin host, or as a separate gateway binary. The envelope
   contract is identical either way; only process-startup wiring differs.

   This umbrella is a peer of :need:`FEAT_0010` "PLC runtime heart"; the
   connector framework is a general-purpose mechanism, not PLC-specific.
   :need:`FEAT_0023` "Fieldbus integration interface" is later expected to
   ``:refines:`` this umbrella once a fieldbus connector spec lands.

----

Capability clusters
-------------------

The umbrella decomposes into seven capability clusters. Each cluster is a
sub-feature ``:satisfies:`` :need:`FEAT_0030`, with concrete shall-clauses
underneath.

Envelope transport
~~~~~~~~~~~~~~~~~~

.. feat:: Envelope transport
   :id: FEAT_0031
   :status: open
   :satisfies: FEAT_0030

   The on-wire form of every message crossing the plugin↔gateway boundary
   and the iceoryx2 service shape that carries it. Defines header fields,
   per-channel sizing, and the zero-copy publish path.

.. req:: ConnectorEnvelope is a POD type
   :id: REQ_0200
   :status: open
   :satisfies: FEAT_0031

   The framework shall define ``ConnectorEnvelope`` as a ``#[repr(C)]``
   plain-old-data type that derives ``ZeroCopySend`` (iceoryx2) and
   contains a fixed header (sequence number, timestamp, payload length,
   correlation id, reserved word) followed by an inline payload buffer.

.. req:: Per-channel max payload size
   :id: REQ_0201
   :status: open
   :satisfies: FEAT_0031

   The framework shall allow each channel to declare its maximum payload
   size at service-creation time, carried in ``ChannelDescriptor``. A
   channel's envelope payload buffer shall be sized to that maximum; no
   universal payload ceiling is imposed across the framework.

.. req:: Sequence number monotonically increasing
   :id: REQ_0202
   :status: open
   :satisfies: FEAT_0031

   For each (publisher, channel) pair, the framework shall populate
   ``ConnectorEnvelope::sequence_number`` with a strictly monotonically
   increasing ``u64`` so receivers can detect missed envelopes.

.. req:: Timestamp recorded at send
   :id: REQ_0203
   :status: open
   :satisfies: FEAT_0031

   The framework shall populate ``ConnectorEnvelope::timestamp_ns`` with
   nanoseconds since the UNIX epoch at the moment the envelope is loaned
   for send.

.. req:: Correlation id is a passive carrier
   :id: REQ_0204
   :status: open
   :satisfies: FEAT_0031

   The framework shall carry the 32-byte ``correlation_id`` field
   end-to-end from sender to receiver without inspecting it. Application
   layers may use this field for request/response matching; the framework
   itself shall not.

.. req:: Zero-copy publish via iceoryx2 loan
   :id: REQ_0205
   :status: open
   :satisfies: FEAT_0031

   The framework shall publish envelopes via ``Publisher::loan`` such that
   the codec writes the payload directly into shared memory. No envelope
   shall be copied between an intermediate user-side buffer and shared
   memory on the send path.

.. req:: One iceoryx2 service per channel direction
   :id: REQ_0206
   :status: open
   :satisfies: FEAT_0031

   For each logical channel direction (outbound app→gateway, inbound
   gateway→app), the framework shall create a separate iceoryx2
   publish-subscribe service whose name is derived deterministically from
   ``ChannelDescriptor::name``.

Codec abstraction
~~~~~~~~~~~~~~~~~

.. feat:: Codec abstraction
   :id: FEAT_0032
   :status: open
   :satisfies: FEAT_0030

   How typed values become payload bytes, and back. Codec selection is a
   compile-time decision via a generic parameter on the connector type;
   no runtime codec dispatch.

.. req:: PayloadCodec trait
   :id: REQ_0210
   :status: open
   :satisfies: FEAT_0032

   The framework shall define a ``PayloadCodec`` trait carrying
   ``format_name()``, ``encode<T: Serialize>(value, &mut [u8]) -> Result<usize>``,
   and ``decode<T: DeserializeOwned>(&[u8]) -> Result<T>``.

.. req:: Codec is a generic parameter on connectors
   :id: REQ_0211
   :status: open
   :satisfies: FEAT_0032

   Each ``Connector`` implementation shall expose its codec as a generic
   parameter (``MqttConnector<C: PayloadCodec>``), monomorphised at
   compile time. The framework shall not provide runtime codec dispatch
   or ``erased_serde``-style indirection.

.. req:: JsonCodec is the default codec
   :id: REQ_0212
   :status: open
   :satisfies: FEAT_0032

   The framework shall ship a ``JsonCodec`` implementation in
   ``sonic-connector-codec`` behind a default-on ``json`` cargo feature.

.. req:: Codec encode error variant
   :id: REQ_0213
   :status: open
   :satisfies: FEAT_0032

   When ``PayloadCodec::encode`` fails (buffer too small, serializer error),
   ``ChannelWriter::send`` shall return ``ConnectorError::Codec`` carrying
   the codec's ``format_name()`` and the underlying source error.

.. req:: Codec decode error variant
   :id: REQ_0214
   :status: open
   :satisfies: FEAT_0032

   When ``PayloadCodec::decode`` fails on a received envelope,
   ``ChannelReader::try_recv`` shall return ``ConnectorError::Codec`` and
   shall not silently drop the envelope.

Connector trait and routing
~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Connector trait and routing
   :id: FEAT_0033
   :status: open
   :satisfies: FEAT_0030

   The plugin-side public API: a ``Connector`` trait every connector
   implements, parameterised on a typed routing struct so plugin code is
   compile-time-checked against the protocol it targets.

.. req:: Connector trait
   :id: REQ_0220
   :status: open
   :satisfies: FEAT_0033

   The framework shall define a ``Connector`` trait with associated types
   ``Routing: Routing`` and ``Codec: PayloadCodec``, plus methods
   ``name``, ``health``, ``subscribe_health``, ``create_writer<T>``, and
   ``create_reader<T>``.

.. req:: ChannelDescriptor carries typed routing
   :id: REQ_0221
   :status: open
   :satisfies: FEAT_0033

   ``ChannelDescriptor<R: Routing>`` shall carry a logical channel name,
   the per-channel max payload size, and a typed routing struct ``R``
   declared by the connector crate.

.. req:: Routing is a marker trait with bounds
   :id: REQ_0222
   :status: open
   :satisfies: FEAT_0033

   The ``Routing`` trait shall require ``Clone + Send + Sync + Debug +
   'static`` and shall add no methods of its own.

.. req:: create_writer / create_reader return concrete handles
   :id: REQ_0223
   :status: open
   :satisfies: FEAT_0033

   ``Connector::create_writer<T>`` and ``Connector::create_reader<T>``
   shall return concrete generic types ``ChannelWriter<T, C, N>`` and
   ``ChannelReader<T, C, N>``, not boxed trait objects.

.. req:: Connector ships its own routing struct
   :id: REQ_0224
   :status: open
   :satisfies: FEAT_0033

   Each connector crate (``sonic-connector-mqtt``, future
   ``sonic-connector-opcua``, etc.) shall define its own routing struct
   (``MqttRouting``, ``OpcUaRouting``, ...) implementing the ``Routing``
   marker trait, exposing protocol-specific fields.

Connection lifecycle
~~~~~~~~~~~~~~~~~~~~

.. feat:: Connection lifecycle
   :id: FEAT_0034
   :status: open
   :satisfies: FEAT_0030

   The observable health state of every connector and the policy by which
   a connector retries after a stack-level disconnect. Both surfaces are
   uniform across protocols, regardless of which protocol stack owns the
   reconnect mechanism.

.. req:: ConnectorHealth state machine
   :id: REQ_0230
   :status: open
   :satisfies: FEAT_0034

   The framework shall define ``ConnectorHealth`` as an enum with
   variants ``Up``, ``Connecting { since }``, ``Degraded { reason }``,
   and ``Down { reason, since }``. Every connector shall report current
   health via ``Connector::health()``.

.. req:: subscribe_health returns a Channel of HealthEvent
   :id: REQ_0231
   :status: open
   :satisfies: FEAT_0034

   ``Connector::subscribe_health()`` shall return an observable handle
   over the connector's ``HealthEvent`` stream so callers can wire
   health transitions into ``ExecutableItem`` triggers. The handle
   type is connector-implementation dependent — typically a
   sonic-executor ``Channel<HealthEventWire>`` (where
   ``HealthEventWire`` is the POD wire form, preferred for
   cross-process gateways) or a thin in-process wrapper around a
   ``crossbeam_channel::Receiver<HealthEvent>`` (acceptable when the
   plugin and gateway share an address space). The choice is recorded
   in the connector's ``impl::`` directive (e.g. :need:`IMPL_0040`).

.. req:: ReconnectPolicy trait
   :id: REQ_0232
   :status: open
   :satisfies: FEAT_0034

   The framework shall define a ``ReconnectPolicy`` trait with
   ``next_delay() -> Duration`` and ``reset()`` for connectors whose
   protocol stack exposes raw connect events.

.. req:: ExponentialBackoff default policy
   :id: REQ_0233
   :status: open
   :satisfies: FEAT_0034

   The framework shall ship an ``ExponentialBackoff`` implementation of
   ``ReconnectPolicy`` configurable with initial delay, max delay, growth
   factor, and jitter ratio.

.. req:: HealthEvent emitted on every transition
   :id: REQ_0234
   :status: open
   :satisfies: FEAT_0034

   Every transition between ``ConnectorHealth`` variants shall emit a
   ``HealthEvent`` on the connector's health channel.

.. req:: Stack-internal-reconnect connectors emit health uniformly
   :id: REQ_0235
   :status: open
   :satisfies: FEAT_0034

   Connectors whose underlying protocol stack manages reconnect internally
   (e.g. tonic-managed gRPC channels) shall not be required to use
   ``ReconnectPolicy``, but shall emit ``HealthEvent`` on every observed
   transition between ``ConnectorHealth`` variants.

Process boundary
~~~~~~~~~~~~~~~~

.. feat:: Process boundary deployments
   :id: FEAT_0035
   :status: open
   :satisfies: FEAT_0030

   The framework supports two deployment shapes — gateway as an in-process
   tokio task or as a separate gateway binary — using the same envelope
   contract on both sides.

.. req:: Same envelope contract for both deployments
   :id: REQ_0240
   :status: open
   :satisfies: FEAT_0035

   The framework shall use the same ``ConnectorEnvelope`` definition,
   iceoryx2 service shape, and ``ChannelDescriptor`` semantics regardless
   of whether the gateway runs in-process or as a separate binary.

.. req:: In-process gateway is a tokio task
   :id: REQ_0241
   :status: open
   :satisfies: FEAT_0035

   The framework shall support running the gateway as a tokio task spawned
   by ``ConnectorHost`` alongside the plugin's executor, in a single
   process.

.. req:: Separate-process gateway is a self-contained binary
   :id: REQ_0242
   :status: open
   :satisfies: FEAT_0035

   The framework shall support running the gateway as a self-contained
   binary in its own OS process, communicating with the plugin only
   through iceoryx2 shared memory.

.. req:: Clean exit on SIGINT / SIGTERM on both sides
   :id: REQ_0243
   :status: open
   :satisfies: FEAT_0035

   Both the plugin host and a separate gateway binary shall return cleanly
   from ``Executor::run()`` on SIGINT/SIGTERM, drain any tokio runtime
   sidecar, and release iceoryx2 services.

.. req:: No app↔gateway control-plane envelopes
   :id: REQ_0244
   :status: open
   :satisfies: FEAT_0035

   The framework shall not introduce envelopes carrying control-plane
   semantics ("ping", "version", "shutdown handshake") on the SHM channel.
   Health is observed via ``ConnectorHealth``, not negotiated.

MQTT reference connector
~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: MQTT reference connector
   :id: FEAT_0036
   :status: open
   :satisfies: FEAT_0030

   The first concrete connector instantiating the framework's contracts:
   ``rumqttc``-backed MQTT 3.1.1 plugin and gateway with bidirectional
   pub/sub, QoS 0+1, retained messages, wildcard subscriptions, and
   optional TLS.

.. req:: MqttConnector implements Connector
   :id: REQ_0250
   :status: open
   :satisfies: FEAT_0036

   The connector crate shall expose ``MqttConnector<C: PayloadCodec>``
   that implements the ``Connector`` trait with
   ``type Routing = MqttRouting``.

.. req:: MqttRouting carries topic, qos, retained
   :id: REQ_0251
   :status: open
   :satisfies: FEAT_0036

   The ``MqttRouting`` struct shall carry the MQTT topic name, the QoS
   level, and a retained-message flag. It shall implement the ``Routing``
   marker trait.

.. req:: QoS 0 and 1 supported
   :id: REQ_0252
   :status: open
   :satisfies: FEAT_0036

   The connector shall support MQTT QoS levels ``AtMostOnce`` (0) and
   ``AtLeastOnce`` (1). QoS 2 is deferred to a follow-on spec.

.. req:: Retained-message publish supported
   :id: REQ_0253
   :status: open
   :satisfies: FEAT_0036

   When ``MqttRouting::retained`` is true, the connector shall publish the
   envelope payload as a retained MQTT message.

.. req:: Wildcard subscriptions supported
   :id: REQ_0254
   :status: open
   :satisfies: FEAT_0036

   The connector shall accept inbound subscriptions whose topic includes
   the MQTT wildcards ``+`` (single-level) and ``#`` (multi-level), and
   shall demultiplex received messages to the matching ``ChannelReader``
   instance(s).

.. req:: Username/password authentication
   :id: REQ_0255
   :status: open
   :satisfies: FEAT_0036

   The connector shall accept username and password credentials in
   ``MqttConnectorOptions`` and present them on the MQTT CONNECT packet.

.. req:: TLS is optional via cargo feature
   :id: REQ_0256
   :status: open
   :satisfies: FEAT_0036

   The connector shall provide TLS support via ``rustls`` behind a
   default-off ``tls`` cargo feature. Client-certificate authentication
   is deferred to a follow-on spec.

.. req:: MQTT 3.1.1 baseline
   :id: REQ_0257
   :status: open
   :satisfies: FEAT_0036

   The connector shall target MQTT protocol version 3.1.1. MQTT 5.0
   features (user properties, shared subscriptions, response topic) are
   deferred to a follow-on spec.

.. req:: Tokio sidecar inside the gateway crate
   :id: REQ_0258
   :status: open
   :satisfies: FEAT_0036

   The MQTT gateway shall host ``rumqttc::EventLoop`` on a tokio runtime
   contained inside ``sonic-connector-mqtt``. Tokio shall not leak into
   sonic-executor's WaitSet thread.

.. req:: Bridge channels are bounded
   :id: REQ_0259
   :status: open
   :satisfies: FEAT_0036

   The outbound (sonic-executor → tokio) and inbound (tokio →
   sonic-executor) bridges shall be bounded channels with configurable
   capacity in ``MqttConnectorOptions``.

.. req:: Outbound bridge saturation surfaces as BackPressure
   :id: REQ_0260
   :status: open
   :satisfies: FEAT_0036

   When the outbound bridge channel is full, ``ChannelWriter::send`` shall
   return ``ConnectorError::BackPressure`` and the connector shall report
   ``ConnectorHealth::Degraded``.

.. req:: Inbound bridge saturation surfaces as DroppedInbound HealthEvent
   :id: REQ_0261
   :status: open
   :satisfies: FEAT_0036

   When the inbound bridge channel is full, the gateway shall emit
   ``HealthEvent::DroppedInbound { count }``. For QoS 1 messages, the
   gateway shall withhold ``PUBACK`` until bridge capacity is restored.

EtherCAT reference connector
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: EtherCAT reference connector
   :id: FEAT_0041
   :status: open
   :satisfies: FEAT_0030

   A second concrete connector instantiating the framework's contracts:
   ``ethercrab``-backed EtherCAT plugin and gateway with cyclic
   process-data exchange, static per-SubDevice PDO mapping, optional
   Distributed Clocks bring-up, and ``ReconnectPolicy``-driven bus
   re-bringup. The gateway owns a single ethercrab ``MainDevice`` on
   one Linux network interface and runs the TX/RX cycle on a tokio
   sidecar contained inside ``sonic-connector-ethercat``. Linux is the
   only supported host OS in the first cut.

.. req:: EthercatConnector implements Connector
   :id: REQ_0310
   :status: open
   :satisfies: FEAT_0041

   The connector crate shall expose ``EthercatConnector<C: PayloadCodec>``
   that implements the ``Connector`` trait with
   ``type Routing = EthercatRouting``.

.. req:: EthercatRouting carries SubDevice and PDO addressing
   :id: REQ_0311
   :status: open
   :satisfies: FEAT_0041

   The ``EthercatRouting`` struct shall identify one process-data slice by
   SubDevice configured address, PDO direction, bit offset within the
   SubDevice's process data, and bit length of the mapped object. It shall
   implement the ``Routing`` marker trait.

.. req:: Single MainDevice per gateway instance
   :id: REQ_0312
   :status: open
   :satisfies: FEAT_0041

   A single ``EthercatGateway`` instance shall own at most one ethercrab
   ``MainDevice`` bound to one network interface. Multi-NIC deployments
   shall instantiate multiple gateways.

.. req:: Bus reaches OP before serving traffic
   :id: REQ_0313
   :status: open
   :satisfies: FEAT_0041

   The gateway shall transition the EtherCAT bus to the OP state before
   accepting envelope traffic from the plugin side.

.. req:: Static PDO mapping per SubDevice
   :id: REQ_0314
   :status: open
   :satisfies: FEAT_0041

   The connector shall accept a static PDO-mapping description per
   SubDevice at build time, declared by the application crate via
   ``EthercatConnectorOptions``.

.. req:: PDO mapping applied during PRE-OP to SAFE-OP transition
   :id: REQ_0315
   :status: open
   :satisfies: FEAT_0041

   The gateway shall apply the configured PDO mapping by issuing SDO writes
   to the sync-manager assignment indices ``0x1C12`` (RxPDO) and ``0x1C13``
   (TxPDO) during the PRE-OP to SAFE-OP transition.

.. req:: Cycle time configurable with millisecond resolution
   :id: REQ_0316
   :status: open
   :satisfies: FEAT_0041

   The gateway shall accept a configurable cycle duration via
   ``EthercatConnectorOptions::cycle_time`` with a default of 2 ms and a
   minimum resolution of 1 ms.

.. req:: Missed cycle ticks are skipped not queued
   :id: REQ_0317
   :status: open
   :satisfies: FEAT_0041

   When the gateway misses one or more cycle ticks, it shall skip the
   missed ticks rather than queue them for catch-up execution.

.. req:: Distributed Clocks bring-up is opt-in
   :id: REQ_0318
   :status: open
   :satisfies: FEAT_0041

   The connector shall perform Distributed Clocks bring-up only when
   ``EthercatConnectorOptions::distributed_clocks`` is enabled by the
   application.

.. req:: Working-counter-based health policy
   :id: REQ_0319
   :status: open
   :satisfies: FEAT_0041

   The gateway shall report ``ConnectorHealth::Up`` only when the bus is in
   OP and the working counter on the latest cycle matches the expected
   value derived from the configured PDO mapping.

.. req:: Working-counter mismatch degrades health
   :id: REQ_0320
   :status: open
   :satisfies: FEAT_0041

   When the working counter on a completed cycle is below the expected
   value, the gateway shall transition ``ConnectorHealth`` to ``Degraded``
   with a reason naming the offending cycle count.

.. req:: Tokio sidecar contained inside the connector crate
   :id: REQ_0321
   :status: open
   :satisfies: FEAT_0041

   The EtherCAT gateway shall host the ethercrab TX/RX task on a tokio
   runtime contained inside ``sonic-connector-ethercat``. Tokio shall not
   leak into sonic-executor's WaitSet thread.

.. req:: Bridge channels are bounded
   :id: REQ_0322
   :status: open
   :satisfies: FEAT_0041

   The outbound (sonic-executor → tokio) and inbound (tokio →
   sonic-executor) bridges between the plugin and the gateway sidecar
   shall be bounded channels with configurable capacity in
   ``EthercatConnectorOptions``.

.. req:: Outbound bridge saturation surfaces as BackPressure
   :id: REQ_0323
   :status: open
   :satisfies: FEAT_0041

   When the outbound bridge channel is full, ``ChannelWriter::send`` shall
   return ``ConnectorError::BackPressure`` and the gateway shall report
   ``ConnectorHealth::Degraded``.

.. req:: Inbound bridge saturation surfaces as DroppedInbound HealthEvent
   :id: REQ_0324
   :status: open
   :satisfies: FEAT_0041

   When the inbound bridge channel is full, the gateway shall emit
   ``HealthEvent::DroppedInbound { count }`` and drop the inbound process
   image for that cycle.

.. req:: Linux raw socket required on gateway host
   :id: REQ_0325
   :status: open
   :satisfies: FEAT_0041

   The gateway shall open the EtherCAT network interface via a Linux raw
   socket, requiring the ``CAP_NET_RAW`` capability on the gateway
   process.

Host wiring
~~~~~~~~~~~

.. feat:: Host wiring and builder
   :id: FEAT_0037
   :status: open
   :satisfies: FEAT_0030

   The composition layer that wraps a ``sonic_executor::Executor`` and
   registers each connector's contributed ``ExecutableItem`` instances —
   matching sonic-executor's existing builder idiom.

.. req:: ConnectorHost builder API
   :id: REQ_0270
   :status: open
   :satisfies: FEAT_0037

   ``sonic-connector-host`` shall expose
   ``ConnectorHost::builder()...with(connector)...build()`` returning a
   ``ConnectorHost`` that owns a ``sonic_executor::Executor``.

.. req:: ConnectorGateway builder API
   :id: REQ_0271
   :status: open
   :satisfies: FEAT_0037

   ``sonic-connector-host`` shall expose a parallel
   ``ConnectorGateway::builder()`` for the gateway-side composition,
   producing a binary that owns its own ``sonic_executor::Executor``.

.. req:: Host registers connector items with the executor
   :id: REQ_0272
   :status: open
   :satisfies: FEAT_0037

   ``ConnectorHost::build()`` shall call ``Executor::add`` for every
   ``ExecutableItem`` contributed by registered connectors and shall
   return an executor ready to run.

.. req:: Optional Observer adapter for tracing
   :id: REQ_0273
   :status: open
   :satisfies: FEAT_0037

   Behind a default-off ``tracing`` cargo feature, the host shall provide
   an ``Observer`` adapter (using ``sonic-executor-tracing``) that
   forwards ``HealthEvent`` and ``ExecutionMonitor`` callbacks through
   the global ``tracing`` subscriber.

----

Anti-goals
----------

The following requirements are explicitly **rejected** — captured for the
record so that future readers see what the framework deliberately does
not do, and why. Each rejected requirement ``:satisfies:`` :need:`FEAT_0030`
to keep the umbrella's traceability complete.

.. req:: NO request/response matching by the framework
   :id: REQ_0290
   :status: rejected
   :satisfies: FEAT_0030

   The framework shall **not** match requests to responses using
   ``ConnectorEnvelope::correlation_id``. The field is a passive carrier;
   higher-layer code may use it for correlation, but the framework
   performs no inspection or matching.

.. req:: NO app↔gateway control plane
   :id: REQ_0291
   :status: rejected
   :satisfies: FEAT_0030

   The framework shall **not** introduce envelopes carrying ``ping``,
   ``version-negotiation``, or ``shutdown-handshake`` semantics across
   the plugin↔gateway boundary. Health and lifecycle are observed via
   ``ConnectorHealth``, not negotiated through SHM control-plane
   envelopes.

.. req:: NO persistent outbox or durable buffering
   :id: REQ_0292
   :status: rejected
   :satisfies: FEAT_0030

   The framework shall **not** persist outbound envelopes on disk or in
   any durable store when the gateway is ``Down``. ``ChannelWriter::send``
   shall return ``Err(Down)`` instead. Durability is the responsibility
   of the broker (MQTT QoS 1/2) or an application-level outbox layered
   above the connector.

.. req:: NO schema/contract enforcement across the boundary
   :id: REQ_0293
   :status: rejected
   :satisfies: FEAT_0030

   The framework shall **not** verify that plugin and gateway agree on
   the channel's payload type ``T`` or codec ``C``. Mismatch surfaces
   only as a decode failure; the framework offers no central schema
   registry.

.. req:: NO protocol-portable Channel<T>
   :id: REQ_0294
   :status: rejected
   :satisfies: FEAT_0030

   The framework shall **not** offer a channel type that is portable
   between protocols ("write the same plugin code, swap MQTT for OPC UA
   without code changes"). Plugin code imports its connector's
   ``Routing`` and is concrete about which protocol it targets.

.. req:: NO multi-broker / multi-tenant gateway
   :id: REQ_0295
   :status: rejected
   :satisfies: FEAT_0030

   A single ``MqttGateway`` instance shall connect to at most one MQTT
   broker. Multi-broker deployments shall instantiate multiple gateways.

.. req:: NO supervision / panic recovery
   :id: REQ_0296
   :status: rejected
   :satisfies: FEAT_0030

   The framework shall **not** catch panics from the tokio task or any
   protocol-stack worker. A panic shall propagate and abort the gateway
   process; restart policy is the host's responsibility, matching
   sonic-executor's existing posture.

----

Cross-cutting traceability
--------------------------

Every requirement on this page (excluding rejected anti-goals) carries a
``:satisfies:`` link to its capability-cluster feat; every cluster feat
``:satisfies:`` :need:`FEAT_0030`. Architectural specifications
(``spec`` directives) refining these requirements are emitted in
:doc:`../architecture/connector`. Verification artefacts (``test``
directives) are emitted in :doc:`../verification/connector`.

.. needtable::
   :types: feat
   :filter: "FEAT_003" in id or id == "FEAT_0041"
   :columns: id, title, status, satisfies
   :show_filters:

.. needtable::
   :types: req
   :filter: "REQ_02" in id or ("REQ_03" in id and id >= "REQ_0310")
   :columns: id, title, status, satisfies
   :show_filters:
