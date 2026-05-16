Connector framework — verification
==================================

Test cases verifying the connector framework requirements. Each ``test``
directive ``:verifies:`` one or more requirements from
:doc:`../requirements/connector` (or building blocks from
:doc:`../architecture/connector`). The four-layer test pyramid from the
architecture's quality strategy is reflected by the section grouping
below: unit, codec, transport integration, MQTT integration, workspace
end-to-end, and loom concurrency.

Implementation tests (Rust ``#[test]``) and the verification artefacts
on this page trace 1:1 — once the implementation lands, each ``test``
body cites the Rust test path that runs it.

----

Unit tests
----------

Per-crate, no IPC, parallel-safe.

.. test:: ExponentialBackoff invariants
   :id: TEST_0100
   :status: open
   :verifies: REQ_0233

   Property test (``proptest``) on ``ExponentialBackoff`` confirming:
   delays are monotonically non-decreasing until the cap is reached,
   delays never exceed the configured maximum, ``reset()`` returns the
   policy to the initial delay, and jitter stays within the configured
   ratio. Lives under ``sonic-connector-core/tests/``.

.. test:: ConnectorHealth state-machine transitions
   :id: TEST_0101
   :status: open
   :verifies: REQ_0230, REQ_0234

   Unit test asserting that every valid transition between
   ``ConnectorHealth`` variants (per :need:`ARCH_0012`) emits exactly
   one ``HealthEvent`` on the connector's health channel, and that
   illegal transitions panic in debug builds.

.. test:: MqttRouting wildcard demux predicate
   :id: TEST_0102
   :status: open
   :verifies: REQ_0254

   Unit-level coverage of the topic-match predicate independent of any
   broker or iceoryx2 service: every (subscription pattern, incoming
   topic) pair is asserted against the MQTT 3.1.1 wildcard semantics
   (single-level ``+``, multi-level ``#``).

.. test:: ChannelDescriptor validation
   :id: TEST_0103
   :status: open
   :verifies: REQ_0201, REQ_0221

   Asserts that constructing a ``ChannelDescriptor`` with an empty
   name fails, and that the const-generic ``N`` propagates correctly
   through ``create_writer`` / ``create_reader`` (compile-fail tests
   ensure mismatched ``N`` values do not type-check).

----

Codec tests
-----------

.. test:: JsonCodec round-trip property test
   :id: TEST_0110
   :status: open
   :verifies: REQ_0210, REQ_0212

   ``proptest``-driven round-trip for a representative struct:
   ``encode(value, &mut buf)`` followed by ``decode(&buf[..len])``
   yields a value equal to the original under every shrunken input.
   Runs against ``JsonCodec``; will be parameterised over
   ``MsgPackCodec`` and ``ProtoCodec`` once those land.

.. test:: Codec encode error on undersized buffer
   :id: TEST_0111
   :status: open
   :verifies: REQ_0213

   Encoding a value larger than the provided buffer returns
   ``ConnectorError::PayloadOverflow { actual, max }`` so the
   buffer-exhaustion path is distinguishable from genuine serializer
   faults at the codec layer. Other serializer failures (NaN
   floats with strict configuration, non-string map keys, etc.)
   surface as ``ConnectorError::Codec`` carrying the codec's static
   ``format_name()`` and the underlying serializer error chain.
   Routing buffer-overflow to ``PayloadOverflow`` keeps the codec
   layer consistent with :need:`REQ_0323` and :need:`TEST_0125` —
   buffer exhaustion is always the same variant regardless of which
   layer detects it.

.. test:: Codec decode error propagation
   :id: TEST_0112
   :status: open
   :verifies: REQ_0214

   Receiving a payload that fails ``decode<T>`` (e.g. truncated JSON,
   wrong shape) surfaces as ``ConnectorError::Codec`` from
   ``ChannelReader::try_recv`` rather than silently dropping the
   envelope.

----

Transport integration tests
---------------------------

Iceoryx2 services are real; tests run with ``--test-threads=1``; each
test scopes its own ``Node`` name.

.. test:: ChannelWriter → ChannelReader round-trip
   :id: TEST_0120
   :status: open
   :verifies: REQ_0205, REQ_0223

   End-to-end zero-copy round-trip through a real iceoryx2 service:
   ``writer.send(&value)`` followed by ``reader.try_recv()`` yields
   the same value. Verifies that ``Publisher::loan`` is used (no
   intermediate copies) by asserting on a header field set in-place.

.. test:: Sequence-number monotonicity
   :id: TEST_0121
   :status: open
   :verifies: REQ_0202

   Sending N envelopes through a single ``ChannelWriter`` and reading
   them on the corresponding ``ChannelReader`` asserts strictly
   increasing ``sequence_number`` values starting at zero.

.. test:: Timestamp populated at send
   :id: TEST_0122
   :status: open
   :verifies: REQ_0203

   Captures wall-clock time before and after ``writer.send``; the
   received envelope's ``timestamp_ns`` falls within the bracket.

.. test:: Correlation ID round-trip
   :id: TEST_0123
   :status: open
   :verifies: REQ_0204

   ``writer.send_with_correlation(&value, id)`` followed by
   ``reader.try_recv()`` yields a header whose ``correlation_id``
   bytes equal ``id``. Confirms the framework does not interpret the
   field — random bytes round-trip unchanged.

.. test:: Per-channel size — 4 KB, 64 KB, 1 MB
   :id: TEST_0124
   :status: open
   :verifies: REQ_0201, BB_0010

   Three round-trip tests with channels parameterised at distinct
   ``N`` (4 096, 65 536, 1 048 576). All three succeed; iceoryx2
   services have non-overlapping pool sizes per channel.

.. test:: Payload-overflow rejection
   :id: TEST_0125
   :status: open
   :verifies: REQ_0201

   ``writer.send(&value)`` for a value whose encoded form exceeds
   the channel's ``N`` returns
   ``ConnectorError::PayloadOverflow { actual, max }`` and emits no
   envelope on the wire.

.. test:: Service naming derived from descriptor
   :id: TEST_0126
   :status: open
   :verifies: REQ_0206, BB_0011

   Two ``ChannelDescriptor`` values with identical ``name`` produce
   identical iceoryx2 service names; differing ``name`` values
   produce different service names. Names follow the convention
   documented in :need:`BB_0011`.

----

MQTT integration tests
----------------------

Embedded ``rumqttd`` per-test fixture on an ephemeral port; iceoryx2
services per test as before; one tokio runtime per test.

.. test:: QoS 0 round-trip
   :id: TEST_0130
   :status: open
   :verifies: REQ_0252

   Plugin → gateway → broker → gateway → plugin round-trip with
   ``MqttRouting { qos: AtMostOnce, retained: false }``. Asserts the
   payload bytes are preserved end-to-end.

.. test:: QoS 1 round-trip
   :id: TEST_0131
   :status: open
   :verifies: REQ_0252

   Same as TEST_0130 but with ``qos: AtLeastOnce``. Additionally
   asserts a ``PUBACK`` is observed on the gateway side before
   reporting success.

.. test:: Retained-message publish + subscribe
   :id: TEST_0132
   :status: open
   :verifies: REQ_0253

   Publish with ``retained: true``; a subsequent subscribe receives
   the retained payload as the first message. Publish a second
   payload with ``retained: false`` and verify the retained value is
   not overwritten by an unset retained.

.. test:: Wildcard subscription with `+`
   :id: TEST_0133
   :status: open
   :verifies: REQ_0254

   Subscribe with ``plant/+/temperature``; publish to
   ``plant/A/temperature`` and ``plant/B/temperature`` and
   ``plant/A/B/temperature``. Reader receives the first two; not the
   third.

.. test:: Wildcard subscription with `#`
   :id: TEST_0134
   :status: open
   :verifies: REQ_0254

   Subscribe with ``plant/#``; publish to ``plant/A``,
   ``plant/A/B``, ``plant/A/B/C``. Reader receives all three.

.. test:: Username/password authentication
   :id: TEST_0135
   :status: open
   :verifies: REQ_0255

   ``MqttConnectorOptions`` configured with username + password;
   ``rumqttd`` fixture configured to require credentials. CONNECT
   succeeds; a wrong-credential variant of the same test fails with
   ``ConnectorHealth::Down { reason: "auth" }``.

.. test:: TLS connection (developer-machine only)
   :id: TEST_0136
   :status: open
   :verifies: REQ_0256

   ``rumqttd`` fixture configured with a self-signed cert; the
   ``tls`` cargo feature is enabled; ``MqttConnectorOptions`` points
   at the test cert. CONNECT succeeds. **Not run in CI** — gated
   behind ``cfg(feature = "tls")`` and a ``CONNECTOR_MQTT_TLS_TESTS``
   env var so the repo carries no embedded test certs.

.. test:: Reconnect after broker bounce
   :id: TEST_0137
   :status: open
   :verifies: REQ_0232, REQ_0233

   While the connector is ``Up``, kill the ``rumqttd`` fixture;
   observe transition to ``Down`` then ``Connecting``; restart the
   broker; observe transition back to ``Up`` within
   ``ExponentialBackoff::max_delay`` seconds. Counts of HealthEvent
   transitions are asserted.

.. test:: HealthEvent emitted on every transition
   :id: TEST_0138
   :status: open
   :verifies: REQ_0234

   Drives the connector through every legal transition in
   :need:`ARCH_0012` and asserts a ``HealthEvent`` arrives on
   ``subscribe_health()`` for each one, in the order driven.

.. test:: Outbound bridge saturation → BackPressure
   :id: TEST_0139
   :status: open
   :verifies: REQ_0260

   Configure ``MqttConnectorOptions`` with a tiny outbound-bridge
   capacity (e.g. 2). Stop draining the gateway by holding the
   tokio task busy. Send N > 2 messages; the (N-1)th or Nth send
   returns ``Err(ConnectorError::BackPressure)`` and the connector
   transitions to ``ConnectorHealth::Degraded``.

.. test:: Inbound bridge saturation → DroppedInbound
   :id: TEST_0140
   :status: open
   :verifies: REQ_0261

   Configure a tiny inbound-bridge capacity. Block the inbound
   gateway item from draining (e.g. by holding ``ChannelReader``).
   Publish a flood of QoS 0 messages from the broker fixture; the
   gateway emits ``HealthEvent::DroppedInbound { count }`` with
   ``count > 0``. For QoS 1 traffic, ``PUBACK`` is observably
   delayed until the bridge drains.

----

EtherCAT integration tests
--------------------------

EtherCAT tests come in two flavours: software tests (unit + raw-frame
mock) parallel-safe on any host, and hardware-in-the-loop tests marked
``[bench]`` that require an EK1100 + EL-series fixture on the CI test
bench. The mock implementation lives in
``sonic-connector-ethercat/tests/mock/`` and replays canned EtherCAT
frame responses so bring-up, PDO mapping, and WKC scenarios can be
exercised without hardware. Bench tests run only when invoked as
``cargo test --features ethercat-bench``.

.. test:: EthercatConnector trait surface
   :id: TEST_0200
   :status: open
   :verifies: REQ_0310

   Compile-time test confirming that
   ``EthercatConnector<JsonCodec>`` implements ``Connector`` with
   ``type Routing = EthercatRouting``. A ``trybuild`` compile-fail
   companion test asserts that swapping ``Routing`` to a foreign
   marker fails to compile.

.. test:: EthercatRouting field round-trip
   :id: TEST_0201
   :status: open
   :verifies: REQ_0311

   Unit test constructing ``EthercatRouting`` values with the four
   fields (SubDevice configured address, PDO direction, bit offset,
   bit length); asserts the values round-trip through serialization
   and that ``EthercatRouting: Routing + Clone + Send + Sync + Debug
   + 'static`` holds at compile time.

.. test:: Single MainDevice per gateway instance
   :id: TEST_0202
   :status: open
   :verifies: REQ_0312

   Construct an ``EthercatGateway`` builder and confirm that the
   builder accepts exactly one network interface name and produces
   exactly one ``MainDevice``. A second call to ``.with_interface``
   replaces the previous value rather than producing a second device.

.. test:: Bus reaches OP before traffic accepted
   :id: TEST_0203
   :status: open
   :verifies: REQ_0313

   Mock-frame test: start an ``EthercatGateway`` against the
   raw-frame mock; before the mock acknowledges the SAFE-OP → OP
   transition, ``ChannelWriter::send`` from the plugin side returns
   ``ConnectorError::NotReady``. After the mock signals OP, the same
   send completes successfully.

.. test:: Static PDO map accepted from options
   :id: TEST_0204
   :status: open
   :verifies: REQ_0314

   Unit test that ``EthercatConnectorOptions::with_pdo_mapping``
   accepts a static description per SubDevice (RxPDO / TxPDO
   entries) and that the in-memory representation preserves entry
   order and bit-offset values. Mismatched declarations (bit length
   exceeds SubDevice capacity) are rejected at builder time.

.. test:: PDO mapping applied during PRE-OP to SAFE-OP
   :id: TEST_0205
   :status: open
   :verifies: REQ_0315

   Mock-frame test observing the SDO write sequence during bring-up:
   the gateway emits writes to ``0x1C12`` (RxPDO) and ``0x1C13``
   (TxPDO) before the SAFE-OP transition is requested. The exact
   sub-index sequence matches the configured PDO mapping.

.. test:: Cycle time configurable
   :id: TEST_0206
   :status: open
   :verifies: REQ_0316

   Unit test that ``EthercatConnectorOptions::cycle_time`` accepts
   ``Duration`` values from 1 ms upward; the default is 2 ms; values
   below 1 ms are rejected at builder time. The configured value is
   observable on the gateway's metadata accessor.

.. test:: Missed ticks are skipped not queued
   :id: TEST_0207
   :status: open
   :verifies: REQ_0317

   Mock-frame test: stall the gateway's tokio sidecar for 5 cycles
   by holding a mutex on the bridge. After release, exactly one
   tx_rx cycle runs (not five) — the skipped ticks are dropped, not
   queued for catch-up. The asserted behaviour matches
   ``tokio::time::MissedTickBehavior::Skip``.

.. test:: Distributed Clocks bring-up is opt-in
   :id: TEST_0208
   :status: open
   :verifies: REQ_0318

   Two mock-frame scenarios. (1) Default options: the gateway
   completes bring-up without emitting any BWR to ``0x0900``, FRMW
   to ``0x0910``, or write to ``0x0920``. (2)
   ``distributed_clocks: true``: the DC register sequence appears
   between PRE-OP and SAFE-OP exactly once per bring-up.

.. test:: Up requires OP and matching working counter
   :id: TEST_0209
   :status: open
   :verifies: REQ_0319

   Mock-frame test: drive the gateway to OP with the mock reporting
   the expected WKC on every cycle for 10 consecutive cycles;
   ``Connector::health()`` returns ``ConnectorHealth::Up``. Inject a
   single low WKC cycle and immediately query health; ``Up`` is no
   longer reported.

.. test:: Working-counter mismatch transitions to Degraded
   :id: TEST_0210
   :status: open
   :verifies: REQ_0320

   Mock-frame test: configure a degradation threshold of N=3 cycles;
   inject N consecutive cycles with WKC below the expected value;
   the gateway transitions to ``ConnectorHealth::Degraded`` and
   emits exactly one ``HealthEvent::Degraded`` with a reason naming
   the offending cycle count.

.. test:: Tokio sidecar contained inside connector crate
   :id: TEST_0211
   :status: open
   :verifies: REQ_0321

   Structural test using ``cargo tree``: assert that
   ``sonic-executor`` does not appear with ``tokio`` in its
   transitive dependency closure, and that ``tokio`` appears only
   under ``sonic-connector-ethercat`` (and other connector crates).
   A second assertion: the published ``EthercatConnector`` plugin
   surface contains no ``tokio::`` types.

.. test:: Bridge channels are bounded with configurable capacity
   :id: TEST_0212
   :status: open
   :verifies: REQ_0322

   Unit test that ``EthercatConnectorOptions::outbound_capacity``
   and ``inbound_capacity`` produce bridges with exactly the
   configured number of slots. After filling the channel, further
   non-blocking sends return ``Full`` rather than allocating
   additional capacity.

.. test:: Outbound bridge saturation surfaces as BackPressure
   :id: TEST_0213
   :status: open
   :verifies: REQ_0323

   Mock-frame test: configure a tiny outbound-bridge capacity. Stall
   the tokio sidecar from draining. The plugin's ``ChannelWriter::send``
   returns ``ConnectorError::BackPressure`` and the gateway reports
   ``ConnectorHealth::Degraded`` until the bridge drains.

.. test:: Inbound bridge saturation surfaces as DroppedInbound
   :id: TEST_0214
   :status: open
   :verifies: REQ_0324

   Mock-frame test: configure a tiny inbound-bridge capacity. Block
   the inbound gateway item from draining. Drive a flood of inbound
   process-image updates through the mock; the gateway emits one or
   more ``HealthEvent::DroppedInbound { count }`` and the inbound
   image for affected cycles is dropped (not buffered).

.. test:: Gateway opens raw socket on Linux with CAP_NET_RAW
   :id: TEST_0215
   :status: open
   :verifies: REQ_0325

   Hardware-bench test ``[bench]``: on a Linux CI host with
   ``CAP_NET_RAW`` granted to the test binary, the gateway opens the
   configured NIC via ``ethercrab::std::tx_rx_task`` and reports
   ``Up``. Companion negative test (also Linux): when
   ``CAP_NET_RAW`` is absent, gateway startup fails with a
   permission error before any EtherCAT frame is sent.

.. test:: PDI bit-slice byte-aligned round-trip
   :id: TEST_0216
   :status: open
   :verifies: REQ_0326, REQ_0327

   Pure-logic test of the ``pdi`` module. For a representative
   set of ``(bit_offset, bit_length)`` pairs where both endpoints
   are byte-aligned (``bit_offset % 8 == 0``,
   ``bit_length % 8 == 0``), the round-trip
   ``write_routing(buf, routing, value); read_routing(buf, routing)``
   shall yield the original ``value`` byte-for-byte, with no
   modification to PDI bytes outside the slice. Property test via
   ``proptest`` over slice positions, lengths, and pre-existing
   buffer contents.

.. test:: PDI bit-slice unaligned round-trip
   :id: TEST_0217
   :status: open
   :verifies: REQ_0326, REQ_0327

   Property test for the same round-trip as :need:`TEST_0216` but
   covering ``bit_offset`` and ``bit_length`` values that are not
   multiples of 8. Verifies that read-modify-write on partial
   leading / trailing bytes preserves the unaffected bits exactly
   (no spillover into adjacent slices).

.. test:: Adjacent PDI bit slices do not interfere
   :id: TEST_0218
   :status: open
   :verifies: REQ_0326

   Construct two ``EthercatRouting`` declarations whose bit slices
   are adjacent (e.g. slice A = bits 0..12, slice B = bits 12..24
   on the same SubDevice / direction). Write distinct values to A
   and B in arbitrary order; read both back. Both reads shall
   return the original written values; neither write shall corrupt
   the other slice.

.. test:: Per-channel routing registry has stable iteration order
   :id: TEST_0219
   :status: open
   :verifies: REQ_0328

   When the application registers N channel descriptors in order
   ``D_1 … D_N`` via ``create_writer`` / ``create_reader``, the
   gateway's cycle-loop iteration over the registry shall visit
   them in the same order on every cycle. Property test confirms
   the order is stable across 1 000 cycles and zero per-cycle
   allocations are observed via ``CountingAllocator``.

.. test:: Outbound end-to-end (plugin send → PDI slice via mock)
   :id: TEST_0220
   :status: open
   :verifies: REQ_0326, REQ_0328

   With a ``MockBusDriver`` configured for a single SubDevice at
   address ``0x0001`` with an outputs buffer of N bytes, a plugin
   that constructs an ``EthercatConnector`` and calls
   ``create_writer`` for a ``ChannelDescriptor`` whose
   ``EthercatRouting`` selects bits ``[bit_offset, bit_offset +
   bit_length)`` of that SubDevice's outputs, then invokes
   ``ChannelWriter::send(value)``: after the next cycle, the
   mock's outputs buffer at the routing's bit slice shall equal
   the codec-encoded representation of ``value``. PDI bytes
   outside the routing's slice shall remain unchanged.

.. test:: Inbound end-to-end (PDI slice via mock → plugin recv)
   :id: TEST_0221
   :status: open
   :verifies: REQ_0327, REQ_0328

   With a ``MockBusDriver`` preloaded with inputs bytes at a known
   bit slice via ``with_subdevice_inputs``, a plugin that
   constructs an ``EthercatConnector`` and calls ``create_reader``
   for a routing pointing at that slice: after one cycle,
   ``ChannelReader::try_recv()`` shall return an envelope whose
   decoded payload equals the value the mock's preloaded bytes
   represent under the channel's codec.

.. test:: Loopback round-trip (plugin → mock → plugin)
   :id: TEST_0222
   :status: open
   :verifies: REQ_0326, REQ_0327

   Compose :need:`TEST_0220` and :need:`TEST_0221` via a ``MockBusDriver``
   variant that, on every cycle, copies the SubDevice's outputs
   buffer over to its inputs buffer (synthetic loopback). The
   plugin registers paired Rx and Tx channels pointing at the
   same bit slice and a fresh routing pair; after one cycle of
   ``ChannelWriter::send(v)`` + ``ChannelReader::try_recv()``,
   the received value equals ``v`` byte-for-byte. Verifies
   end-to-end iceoryx2 ↔ PDI ↔ iceoryx2 plumbing without
   hardware.

----

Workspace end-to-end tests
--------------------------

Full stack exercised via ``sonic-connector-host`` examples or
``assert_cmd``-driven binary smoke tests.

.. test:: In-process gateway smoke
   :id: TEST_0150
   :status: open
   :verifies: REQ_0241, ARCH_0020

   Single-binary integration: ``ConnectorHost`` launches the plugin
   executor and an in-process tokio task hosting ``MqttGateway``
   against a ``rumqttd`` fixture. End-to-end pub/sub round-trip
   succeeds; process exits cleanly on programmatic stop.

.. test:: Separate-process gateway smoke
   :id: TEST_0151
   :status: open
   :verifies: REQ_0242, ARCH_0021

   Two binaries: a plugin process running ``ConnectorHost`` and a
   gateway process running ``ConnectorGateway`` against
   ``rumqttd``. SHM transport carries envelopes between them. A
   round-trip succeeds; both processes exit cleanly.

.. test:: SIGINT clean exit within 5-second budget
   :id: TEST_0152
   :status: open
   :verifies: REQ_0243, ARCH_0013

   While the connector is mid-traffic, send SIGINT; the host returns
   from ``run()`` within 5 seconds; tokio runtime drains; all
   iceoryx2 services release; exit code is 0.

.. test:: No control-plane envelopes flow
   :id: TEST_0153
   :status: open
   :verifies: REQ_0244, REQ_0291

   With one channel configured, observe the iceoryx2 service for the
   duration of a normal session: the only envelopes that flow are
   user-payload envelopes (no "ping", "version", or "shutdown
   handshake"). Asserts the framework's no-control-plane invariant.

----

Zenoh reference connector
~~~~~~~~~~~~~~~~~~~~~~~~~

Layer-1 (pure-logic) tests run in default CI against
``MockZenohSession`` and require no real ``zenoh`` crate.
Layer-2 (``zenoh-integration`` feature gate) and Layer-3
(``ZENOH_TEST_ROUTER`` env-gated client mode) tests run on
dedicated CI jobs; they ``:status: draft`` until those jobs
land.

.. test:: ZenohRouting field validation
   :id: TEST_0300
   :status: open
   :verifies: REQ_0401

   Unit test asserting that constructing a ``ZenohRouting`` with
   an invalid ``key_expr`` (empty, leading slash, illegal wildcard
   combination) fails with ``ConnectorError::Configuration`` before
   any iceoryx2 service is created. Asserts that valid
   congestion-control / priority / reliability / express
   combinations round-trip through ``ChannelDescriptor`` without
   loss.

.. test:: ZenohConnector implements Connector with ZenohRouting
   :id: TEST_0301
   :status: open
   :verifies: REQ_0400

   Compile-fail test ensuring ``ZenohConnector<JsonCodec>`` is
   accepted in any position requiring ``Connector<Routing =
   ZenohRouting>``. Asserts ``create_writer`` / ``create_reader``
   return the expected ``ChannelWriter<T, JsonCodec, N>`` /
   ``ChannelReader<T, JsonCodec, N>`` concrete types.

.. test:: Pub/sub end-to-end against MockZenohSession
   :id: TEST_0302
   :status: open
   :verifies: REQ_0402, REQ_0407, REQ_0408, REQ_0445

   Drive a ``ChannelWriter::send(value)`` through
   ``MockZenohSession`` and observe ``ChannelReader::try_recv``
   receive the same value. Asserts sequence number monotonicity
   (:need:`REQ_0202`), timestamp non-zero, and that the gateway
   publishes raw bytes on the inbound service (codec runs on the
   plugin side, per :need:`REQ_0408`).

.. test:: Query round-trip against MockZenohSession
   :id: TEST_0303
   :status: open
   :verifies: REQ_0420, REQ_0421, REQ_0422, REQ_0423, REQ_0424, REQ_0426, REQ_0427

   End-to-end query test: plugin A calls
   ``ZenohQuerier::send(q)``; plugin B's ``ZenohQueryable::try_recv``
   surfaces ``(QueryId, Q)``; plugin B calls ``reply(id, r)`` three
   times then ``terminate(id)``; plugin A's ``ZenohQuerier::try_recv``
   observes the three replies in order followed by a 0x02
   end-of-stream envelope. Asserts ``QueryId`` round-trips through
   the envelope's ``correlation_id`` unchanged and that the
   queryable's gateway map entry is freed after ``terminate``.

.. test:: Codec failure paths for queries
   :id: TEST_0304
   :status: open
   :verifies: REQ_0427

   Encoding a value larger than the envelope's payload returns
   ``ConnectorError::Codec`` from ``ZenohQuerier::send`` and from
   ``ZenohQueryable::reply``; decoding malformed bytes returns
   ``ConnectorError::Codec`` from the matching ``try_recv``. The
   envelope is not silently dropped (re-affirming
   :need:`REQ_0214`).

.. test:: Outbound bridge saturation surfaces as BackPressure
   :id: TEST_0305
   :status: open
   :verifies: REQ_0404, REQ_0405

   With ``outbound_bridge_capacity = 1`` and a deliberately stalled
   ``MockZenohSession``, the second ``ChannelWriter::send`` (and
   the second ``ZenohQuerier::send``) returns
   ``ConnectorError::BackPressure`` and the connector's
   ``health()`` snapshot transitions to ``Degraded``.

.. test:: Inbound bridge saturation surfaces as DroppedInbound
   :id: TEST_0306
   :status: open
   :verifies: REQ_0406, REQ_0428

   With ``inbound_bridge_capacity = 1`` and a deliberately stalled
   plugin reader, the gateway emits
   ``HealthEvent::DroppedInbound { count = N }`` reflecting the
   number of pub/sub samples and reply chunks discarded. The
   in-flight ``QueryId`` is observable as a reply stream with
   fewer chunks than the upstream peer sent.

.. test:: Query timeout emits 0x03 terminator
   :id: TEST_0307
   :status: open
   :verifies: REQ_0425

   With ``query_timeout = 50 ms`` and a queryable that never
   replies, ``ZenohQuerier::try_recv`` observes a single envelope
   with ``payload[0] == 0x03`` for the in-flight ``QueryId``;
   subsequent ``try_recv`` calls for that ``QueryId`` return
   ``None``. Gateway map entry for that ``QueryId`` is freed.

.. test:: Health state machine on MockZenohSession lifecycle
   :id: TEST_0308
   :status: open
   :verifies: REQ_0440, REQ_0442

   Walk the mock session through ``Connecting → Up → Degraded →
   Up → Down`` and assert one ``HealthEvent`` per transition on
   the connector's health channel. Asserts each variant carries
   the documented payload (``since`` timestamp, ``reason``
   string).

.. test:: REQ_0441 anti-req — no ReconnectPolicy on session loss
   :id: TEST_0309
   :status: implemented
   :verifies: REQ_0441

   Static check that ``ZenohGateway`` exposes no
   ``ReconnectPolicy``-typed field and the
   ``ZenohConnectorOptions`` struct does not declare a
   ``reconnect_policy`` setting. Realised as
   ``crates/sonic-connector-zenoh/tests/no_reconnect_policy.rs``,
   which shells out to ``cargo public-api`` and asserts the
   public surface contains no ``ReconnectPolicy`` /
   ``reconnect_policy`` identifier.

.. test:: zenoh-integration feature gates the real zenoh dep
   :id: TEST_0310
   :status: implemented
   :verifies: REQ_0444, REQ_0445

   Build the crate twice — once with default features, once with
   ``--features zenoh-integration`` — and assert that the default
   build does not link ``zenoh`` (via ``cargo tree`` introspection)
   while the feature build does. Both builds expose
   ``MockZenohSession``. Realised as ``scripts/check_dep_gating.sh``
   invoked from the ``dep-gating`` job in
   ``.github/workflows/ci-zenoh.yml``.

.. test:: Cross-platform support
   :id: TEST_0311
   :status: implemented
   :verifies: REQ_0446

   CI matrix builds the crate on Linux, macOS, and Windows
   (default features) and runs ``cargo test`` on all three. No
   platform-specific compile errors; no platform-gated
   ``#[cfg]`` paths break. Realised as the ``build-test-default``
   matrix job in ``.github/workflows/ci-zenoh.yml``.

.. test:: Two-peer real session pub/sub
   :id: TEST_0312
   :status: draft
   :verifies: REQ_0440, REQ_0443

   Layer-2 integration test (``zenoh-integration`` feature):
   spawn two ``ZenohConnector`` instances in peer mode in the
   same process with disjoint locator listings, exchange envelopes
   through a real ``zenoh::Session`` pair, and assert payloads /
   sequence numbers round-trip correctly. Status remains
   ``deferred`` until the dedicated CI job lands.

.. test:: Client-mode router smoke
   :id: TEST_0313
   :status: draft
   :verifies: REQ_0440, REQ_0443

   Layer-3 env-gated test: when ``ZENOH_TEST_ROUTER`` names a
   reachable ``zenohd``, open a client-mode session and assert
   ``ConnectorHealth`` reaches ``Up``. Status remains
   ``deferred`` until the client-mode CI job lands.

.. test:: Tokio sidecar contained inside sonic-connector-zenoh
   :id: TEST_0314
   :status: implemented
   :verifies: REQ_0403

   Static check that the ``zenoh::Session`` and any tokio runtime
   handle live entirely inside the ``sonic-connector-zenoh`` crate.
   No public type exported by ``sonic-connector-zenoh`` shall name a
   ``tokio::*`` type in its signature (compile-time API surface scan).
   Realised as
   ``crates/sonic-connector-zenoh/tests/tokio_containment.rs``,
   which shells out to ``cargo public-api`` and asserts the public
   surface contains no ``tokio::`` identifier. The runtime piece —
   an executor unit test asserting that the WaitSet thread does not
   contain any tokio task handle attributable to the gateway
   sidecar (mirrors :need:`TEST_0211` under :need:`REQ_0321`) — is
   deferred to a Z6+ stage that lands ``sonic-executor`` task
   introspection.

----

Loom concurrency tests
----------------------

Run with ``cargo test --features loom`` under ``cfg(loom)``.

.. test:: Bridge handoff under arbitrary interleaving
   :id: TEST_0160
   :status: open
   :verifies: REQ_0259, BB_0022

   Loom model of ``OutboundGatewayItem.execute`` racing with the
   tokio task draining the bridge: every produced frame is observed
   exactly once by the consumer; no deadlock.

.. test:: Health state-machine under concurrent updates
   :id: TEST_0161
   :status: open
   :verifies: REQ_0230, REQ_0234

   Loom model with multiple threads attempting transitions
   simultaneously (e.g. the tokio task reporting ``Down`` while the
   reconnect timer fires ``Connecting``): the state machine never
   enters an invalid state and no event is dropped.

----

Cross-cutting traceability
--------------------------

.. needtable::
   :types: test
   :filter: "TEST_01" in id or "TEST_02" in id
   :columns: id, title, status, verifies
   :show_filters:
