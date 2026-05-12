Connector framework
===================

This page captures the requirements for ``sonic-connector``: a framework that
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

.. req:: Outbound payload written to PDI bit slice per routing
   :id: REQ_0326
   :status: open
   :satisfies: FEAT_0041

   When a plugin publishes a value through ``ChannelWriter::send``, the
   gateway shall, before the next cycle's ``tx_rx`` call, write the
   codec-encoded payload into the cycle's outbound PDI buffer at the
   bit offset and bit length declared by the channel's
   :need:`REQ_0311` ``EthercatRouting``. The write shall target the
   SubDevice's process image starting at ``bit_offset`` from the
   start of that SubDevice's outputs region, covering exactly
   ``bit_length`` bits. The framework shall preserve adjacent bit
   slices (read-modify-write on partial leading / trailing bytes).

.. req:: Inbound payload read from PDI bit slice per routing
   :id: REQ_0327
   :status: open
   :satisfies: FEAT_0041

   After each cycle's ``tx_rx`` call returns successfully, the gateway
   shall, for every registered inbound channel, extract
   ``bit_length`` bits starting at ``bit_offset`` from the SubDevice's
   process image inputs region (per the channel's
   :need:`REQ_0311` ``EthercatRouting``), and publish the resulting
   byte slice on the channel's inbound iceoryx2 service as a
   ``ConnectorEnvelope`` whose ``payload_len`` is
   ``ceil(bit_length / 8)``. The gateway shall **not** invoke the
   channel's codec on this path — codec decoding is the
   responsibility of the plugin-side ``ChannelReader::try_recv``,
   keeping the gateway a byte-only mover (symmetric with
   :need:`REQ_0326`, where the plugin's ``ChannelWriter::send``
   encodes and the gateway moves the already-encoded bytes). Reads
   shall not modify the PDI buffer.

.. req:: Per-channel routing registry on the gateway
   :id: REQ_0328
   :status: open
   :satisfies: FEAT_0041

   The gateway shall maintain a registry mapping each open
   ``ChannelDescriptor`` to its ``EthercatRouting`` and direction
   (RxPDO outbound / TxPDO inbound), populated when the application
   calls ``Connector::create_writer`` / ``Connector::create_reader``.
   The cycle loop shall iterate this registry on every cycle —
   draining the outbound bridge for each Rx channel, repopulating
   the inbound iceoryx2 service for each Tx channel — without per-
   cycle heap allocation (no ``Vec`` resize, no ``HashMap``
   re-hash). Required by :need:`REQ_0060` from the steady-state
   posture: connector dispatch shall not allocate.

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

Zenoh reference connector
~~~~~~~~~~~~~~~~~~~~~~~~~

.. feat:: Zenoh reference connector
   :id: FEAT_0042
   :status: open
   :satisfies: FEAT_0030

   A third concrete connector instantiating the framework's contracts:
   ``zenoh``-backed plugin and gateway with bidirectional pub/sub and
   queries. The session topology is configurable between peer and
   client modes; reconnect is delegated to the Zenoh session itself
   (stack-internal posture mirroring :need:`REQ_0235`). Queries are
   exposed via concrete methods on ``ZenohConnector`` only — the
   shared ``Connector`` trait is not modified. The gateway owns one
   ``zenoh::Session`` and runs Zenoh's async callbacks on a tokio
   sidecar contained inside ``sonic-connector-zenoh``. Linux, macOS,
   and Windows are supported host operating systems.

.. feat:: Zenoh pub/sub
   :id: FEAT_0043
   :status: open
   :satisfies: FEAT_0042

   The pub/sub half of the Zenoh connector. ``ChannelWriter`` and
   ``ChannelReader`` carry codec-encoded values through iceoryx2 SHM
   services to / from Zenoh publishers and subscribers running on
   the gateway's tokio sidecar. Bridges between sonic-executor and
   tokio are bounded; saturation surfaces as ``BackPressure`` on
   outbound and ``DroppedInbound`` health events on inbound.

.. req:: ZenohConnector implements Connector
   :id: REQ_0400
   :status: open
   :satisfies: FEAT_0043

   The connector crate shall expose ``ZenohConnector<C: PayloadCodec>``
   that implements the ``Connector`` trait with
   ``type Routing = ZenohRouting`` and ``type Codec = C``.

.. req:: ZenohRouting carries key_expr and pub/sub QoS fields
   :id: REQ_0401
   :status: open
   :satisfies: FEAT_0043

   The ``ZenohRouting`` struct shall carry the Zenoh key expression
   (``key_expr: KeyExprOwned``), congestion control mode
   (``Block | Drop``), priority (``RealTime..Background``),
   reliability (``Reliable | BestEffort``), and a boolean
   ``express`` flag (batching opt-out). It shall implement the
   ``Routing`` marker trait. Validation of the key expression shall
   occur on the plugin side inside ``create_writer`` /
   ``create_reader`` (and the query-side analogues), before any
   iceoryx2 service is created; an invalid expression shall yield
   ``ConnectorError::Configuration``.

.. req:: JsonCodec is the default codec for Zenoh
   :id: REQ_0402
   :status: open
   :satisfies: FEAT_0043

   The Zenoh connector shall accept any ``PayloadCodec`` via its
   ``C`` generic parameter (re-affirming :need:`REQ_0211`), with
   ``JsonCodec`` as the default codec used by example wiring
   (re-affirming :need:`REQ_0212`).

.. req:: Tokio sidecar contained inside the Zenoh connector crate
   :id: REQ_0403
   :status: open
   :satisfies: FEAT_0043

   The Zenoh gateway shall host the ``zenoh::Session`` and all
   Zenoh async callbacks on a tokio runtime contained inside
   ``sonic-connector-zenoh``. Tokio shall not leak into
   sonic-executor's WaitSet thread (mirrors :need:`REQ_0321` and
   :need:`REQ_0258`).

.. req:: Zenoh bridge channels are bounded
   :id: REQ_0404
   :status: open
   :satisfies: FEAT_0043

   The outbound (sonic-executor → tokio) and inbound (tokio →
   sonic-executor) bridges between the plugin and the Zenoh gateway
   sidecar shall be bounded channels with capacities configurable
   via ``ZenohConnectorOptions`` (``outbound_bridge_capacity`` and
   ``inbound_bridge_capacity``).

.. req:: Outbound bridge saturation surfaces as BackPressure
   :id: REQ_0405
   :status: open
   :satisfies: FEAT_0043

   When the outbound bridge channel is full, ``ChannelWriter::send``
   (and any other plugin-side write entry-point that feeds the
   outbound bridge) shall return ``ConnectorError::BackPressure``
   and the gateway shall report ``ConnectorHealth::Degraded``.

.. req:: Inbound bridge saturation surfaces as DroppedInbound
   :id: REQ_0406
   :status: open
   :satisfies: FEAT_0043

   When the inbound bridge channel is full, the gateway shall emit
   ``HealthEvent::DroppedInbound { count }`` and drop the offending
   inbound Zenoh sample or reply chunk for that callback.

.. req:: Zenoh zero-copy publish via iceoryx2 loan
   :id: REQ_0407
   :status: open
   :satisfies: FEAT_0043

   ``ChannelWriter::send`` on a Zenoh channel shall publish
   envelopes via ``Publisher::loan`` so that the codec writes the
   payload directly into shared memory (re-affirms :need:`REQ_0205`).

.. req:: Zenoh gateway is byte-only on the inbound publish path
   :id: REQ_0408
   :status: open
   :satisfies: FEAT_0043

   On the inbound leg (Zenoh peer → plugin), the gateway shall
   publish the raw payload bytes received from the Zenoh subscriber
   or reply callback onto the channel's inbound iceoryx2 service as
   a ``ConnectorEnvelope`` without invoking the channel's codec —
   codec decoding is the responsibility of the plugin-side
   ``ChannelReader::try_recv`` (symmetric with :need:`REQ_0327`).

.. feat:: Zenoh queries
   :id: FEAT_0044
   :status: open
   :satisfies: FEAT_0042

   The query half of the Zenoh connector — Zenoh's signature
   request/response primitive, layered on top of the same
   ``ConnectorEnvelope`` shape used by pub/sub. Exposed via concrete
   non-trait methods on ``ZenohConnector``: ``create_querier`` and
   ``create_queryable``. The framework's anti-goal
   :need:`REQ_0290` (no framework-level correlation matching) is
   preserved — correlation lives inside the Zenoh-specific handle
   types, using the framework's existing 32-byte passive
   ``correlation_id`` carrier (:need:`REQ_0204`).

.. req:: ZenohConnector exposes create_querier and create_queryable
   :id: REQ_0420
   :status: open
   :satisfies: FEAT_0044

   ``ZenohConnector`` shall expose, as concrete methods (NOT on the
   ``Connector`` trait), ``create_querier<Q, R, const N: usize>`` and
   ``create_queryable<Q, R, const N: usize>``, returning
   ``ZenohQuerier<Q, R, C, N>`` and ``ZenohQueryable<Q, R, C, N>``
   respectively, with ``Q`` and ``R`` bound by ``serde::Serialize`` /
   ``serde::de::DeserializeOwned`` as appropriate per direction.

.. req:: ZenohQuerier maps QueryId to envelope correlation_id
   :id: REQ_0421
   :status: open
   :satisfies: FEAT_0044

   ``ZenohQuerier::send(q: Q)`` shall mint a fresh ``QueryId`` for
   each call, populate the outbound envelope's ``correlation_id``
   with the ``QueryId``, and return the ``QueryId`` to the caller so
   incoming replies on the matching ``{name}.reply.in`` iceoryx2
   service can be demultiplexed by ``QueryId``.

.. req:: ZenohQueryable correlates replies via correlation_id
   :id: REQ_0422
   :status: open
   :satisfies: FEAT_0044

   ``ZenohQueryable::try_recv`` shall surface the gateway-minted
   ``QueryId`` (= the envelope's ``correlation_id``) alongside the
   decoded request value ``Q``. ``ZenohQueryable::reply(id, r)``
   shall stamp ``id`` onto the reply envelope's ``correlation_id``
   so the gateway-side dispatcher can look up the corresponding
   ``zenoh::Query`` handle. The framework shall not perform this
   matching itself (preserves :need:`REQ_0290`); the matching lives
   inside ``ZenohQueryable``.

.. req:: Multi-reply per query supported
   :id: REQ_0423
   :status: open
   :satisfies: FEAT_0044

   ``ZenohQueryable::reply(id, r)`` shall be callable zero or more
   times for the same ``QueryId`` before ``terminate(id)``. Each
   call shall publish one reply envelope on the channel's
   ``{name}.reply.out`` iceoryx2 service; the gateway shall forward
   each to ``zenoh::Query::reply`` on the matching handle.

.. req:: Reply stream end-of-stream framed in payload
   :id: REQ_0424
   :status: open
   :satisfies: FEAT_0044

   The end of a reply stream shall be signalled by a one-byte
   Zenoh-private frame discriminator at the start of the reply
   envelope's payload: ``0x01`` = data chunk (followed by
   codec-encoded ``R``); ``0x02`` = end of stream (no body);
   ``0x03`` = timeout terminator (gateway-synthetic, no body). The
   framework's ``ConnectorEnvelope`` reserved word
   (:need:`REQ_0200`) shall not be repurposed for this signal.
   ``ZenohQueryable::terminate(id)`` shall emit a ``0x02`` envelope
   for ``id`` and free the gateway-side ``zenoh::Query`` handle.

.. req:: Query timeout sourced from options, overridable per-querier
   :id: REQ_0425
   :status: open
   :satisfies: FEAT_0044

   The default per-query timeout shall be sourced from
   ``ZenohConnectorOptions::query_timeout``. ``ZenohQuerier`` shall
   allow this default to be overridden at querier-creation time
   (via a builder option) or per-call (via an explicit
   ``send_with_timeout(q, timeout)`` method). Timeout expiry on the
   gateway shall emit a ``0x03`` terminator (per :need:`REQ_0424`)
   on the reply stream for that ``QueryId``.

.. req:: terminate(id) finalizes the upstream zenoh::Query
   :id: REQ_0426
   :status: open
   :satisfies: FEAT_0044

   When the gateway observes a ``0x02`` end-of-stream envelope from
   the queryable side (or synthesises a ``0x03`` timeout), it shall
   drop the corresponding entry from its ``correlation_id →
   zenoh::Query`` map. Dropping the ``zenoh::Query`` handle
   finalizes the reply stream as observed by the upstream Zenoh
   peer.

.. req:: Codec applied to Q on send and to R on reply
   :id: REQ_0427
   :status: open
   :satisfies: FEAT_0044

   ``ZenohQuerier::send`` shall encode ``Q`` via the connector's
   ``C: PayloadCodec`` into the envelope payload before SHM
   publish. ``ZenohQueryable::reply`` shall encode ``R`` via the
   same codec into ``envelope.payload[1..]`` (with byte ``[0]``
   carrying the ``0x01`` data discriminator per :need:`REQ_0424`).
   Decoding the inbound counterpart (``Q`` on the queryable side,
   ``R`` on the querier side) shall happen plugin-side in
   ``try_recv`` and shall surface codec failures as
   ``ConnectorError::Codec`` per :need:`REQ_0214`.

.. req:: Reply-side inbound saturation emits DroppedInbound
   :id: REQ_0428
   :status: open
   :satisfies: FEAT_0044

   When the inbound bridge for the reply path (gateway → plugin
   on a querier channel) saturates, the gateway shall emit
   ``HealthEvent::DroppedInbound { count }`` (re-affirming
   :need:`REQ_0406`) and drop the offending reply chunk. The
   in-flight ``QueryId`` shall be observable on the plugin side as
   a reply stream with fewer chunks than the upstream peer sent;
   no separate "partial reply" error variant is added.

.. feat:: Zenoh session topology and health
   :id: FEAT_0045
   :status: open
   :satisfies: FEAT_0042

   The Zenoh-specific session and observability surface — peer-vs-
   client mode configuration, scout/locator wiring, and the
   stack-internal reconnect posture. Health-event semantics inherit
   from :need:`FEAT_0034` and re-affirm :need:`REQ_0235` (stack-
   internal reconnect emits health events without
   ``ReconnectPolicy``).

.. req:: Zenoh session mode is a config knob
   :id: REQ_0440
   :status: open
   :satisfies: FEAT_0045

   ``ZenohConnectorOptions::mode`` shall accept the values
   ``SessionMode::{Peer, Client, Router}`` and shall default to
   ``Peer``. The gateway shall translate this knob into the
   corresponding ``zenoh::Config`` field before calling
   ``zenoh::open``.

.. req:: NO ReconnectPolicy on Zenoh session loss
   :id: REQ_0441
   :status: rejected
   :satisfies: FEAT_0045

   The Zenoh connector shall **not** use
   :need:`REQ_0232` ``ReconnectPolicy`` on session loss. Zenoh's
   own scout / reconnect machinery owns the retry; the connector
   merely emits ``HealthEvent`` on every observed transition
   between ``ConnectorHealth`` variants (mirrors :need:`REQ_0235`
   for tonic/gRPC).

.. req:: HealthEvent emitted on every Zenoh session transition
   :id: REQ_0442
   :status: open
   :satisfies: FEAT_0045

   Every transition of the Zenoh session between alive and closed
   states observed by the gateway (including the initial
   ``Connecting → Up`` and any subsequent re-bringup driven by
   Zenoh's own retry) shall emit one ``HealthEvent`` on the
   connector's health channel (re-affirms :need:`REQ_0234`).

.. req:: Connect and listen locators surfaced to zenoh::Config
   :id: REQ_0443
   :status: open
   :satisfies: FEAT_0045

   ``ZenohConnectorOptions::connect`` and
   ``ZenohConnectorOptions::listen`` shall be carried to
   ``zenoh::Config`` verbatim before ``zenoh::open``. Validation of
   locator URIs is delegated to ``zenoh`` (the connector neither
   parses nor canonicalises them).

.. req:: zenoh-integration cargo feature gates the real zenoh dep
   :id: REQ_0444
   :status: open
   :satisfies: FEAT_0045

   The ``sonic-connector-zenoh`` crate shall expose a default-off
   ``zenoh-integration`` cargo feature. The real ``zenoh`` crate
   shall be an optional dependency activated only by that feature
   (mirrors :need:`BB_0030`'s ``bus-integration`` posture). Without
   the feature, the connector framework types plus
   ``MockZenohSession`` shall remain available.

.. req:: MockZenohSession ships unfeature-gated
   :id: REQ_0445
   :status: open
   :satisfies: FEAT_0045

   ``MockZenohSession`` — an in-process pub/sub + query loopback
   implementation of the ``ZenohSessionLike`` trait — shall ship in
   the default build, not gated by ``zenoh-integration``. It exists
   so that the Layer-1 (pure-logic) test pyramid can exercise the
   full envelope ↔ session ↔ envelope hop without depending on the
   real ``zenoh`` crate.

.. req:: Linux, macOS, and Windows are supported host operating systems
   :id: REQ_0446
   :status: open
   :satisfies: FEAT_0045

   The Zenoh connector shall support Linux, macOS, and Windows as
   host operating systems for both plugin and gateway (broader than
   :need:`REQ_0325`'s Linux-only EtherCAT posture, because Zenoh has
   no OS-specific socket requirement comparable to ``CAP_NET_RAW``).

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
   :filter: "FEAT_003" in id or id in ("FEAT_0041", "FEAT_0042", "FEAT_0043", "FEAT_0044", "FEAT_0045")
   :columns: id, title, status, satisfies
   :show_filters:

.. needtable::
   :types: req
   :filter: "REQ_02" in id or ("REQ_03" in id and id >= "REQ_0310") or "REQ_04" in id
   :columns: id, title, status, satisfies
   :show_filters:
