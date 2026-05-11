//! TEST_0220 / TEST_0221 / TEST_0222 — end-to-end I/O via the gateway
//! dispatcher (`REQ_0326`, `REQ_0327`, `REQ_0328`). Driven by
//! [`MockBusDriver`] so the full iceoryx2 ↔ PDI ↔ iceoryx2 hop is
//! exercised in CI without hardware.
//!
//! These tests wire the moving parts directly (iceoryx2 [`Node`],
//! [`ServiceFactory`], a hand-built [`ChannelRegistry`],
//! [`CycleRunner`], and [`dispatch_one_cycle`]) rather than going
//! through [`EthercatConnector`]'s `register_with` spawn path. The
//! direct wiring keeps the cycle synchronous and lets each test
//! inspect the mock's PDI buffers between cycles. The
//! `EthercatConnector::create_writer` / `create_reader` plumbing that
//! does the same wiring at the higher level is exercised by the
//! `connector_trait` integration tests.

#![allow(clippy::doc_markdown, clippy::cast_possible_truncation)]

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_codec::JsonCodec;
use sonic_connector_core::{ChannelDescriptor, ConnectorHealthKind, PayloadCodec};
use sonic_connector_ethercat::{
    BringUp, ChannelBinding, ChannelRegistry, CycleRunner, EthercatConnectorOptions,
    EthercatHealthMonitor, EthercatRouting, IoxInboundPublish, IoxOutboundDrain, MockBusDriver,
    PdoDirection, dispatch_one_cycle,
};
use sonic_connector_transport_iox::ServiceFactory;

fn make_node() -> Node<ipc::Service> {
    NodeBuilder::new()
        .create::<ipc::Service>()
        .expect("create iceoryx2 node")
}

fn make_options() -> EthercatConnectorOptions {
    EthercatConnectorOptions::builder().build()
}

/// Helper: open an outbound (plugin → gateway) pair on `service_name`
/// and register an Outbound binding into `registry`. Returns the
/// plugin-side typed writer.
fn open_outbound<T, const N: usize>(
    factory: &ServiceFactory<'_>,
    registry: &Mutex<ChannelRegistry>,
    service_name: &str,
    routing: EthercatRouting,
) -> sonic_connector_transport_iox::ChannelWriter<T, JsonCodec, N>
where
    T: serde::Serialize + 'static,
{
    let desc =
        ChannelDescriptor::<EthercatRouting, N>::new(service_name.to_string(), routing).unwrap();
    let writer = factory
        .create_writer::<T, _, _, N>(&desc, JsonCodec::new())
        .expect("plugin writer");
    let raw_reader = factory
        .create_raw_reader_named::<N>(service_name)
        .expect("gateway raw reader");
    registry.lock().unwrap().register(
        service_name.to_string(),
        routing,
        PdoDirection::Rx,
        ChannelBinding::Outbound(Box::new(IoxOutboundDrain::new(raw_reader))),
    );
    writer
}

/// Helper: open an inbound (gateway → plugin) pair on `service_name`.
fn open_inbound<T, const N: usize>(
    factory: &ServiceFactory<'_>,
    registry: &Mutex<ChannelRegistry>,
    service_name: &str,
    routing: EthercatRouting,
) -> sonic_connector_transport_iox::ChannelReader<T, JsonCodec, N>
where
    T: serde::de::DeserializeOwned + 'static,
{
    let desc =
        ChannelDescriptor::<EthercatRouting, N>::new(service_name.to_string(), routing).unwrap();
    // Reader first — subscribers must attach before publishers send
    // (iceoryx2's default pub/sub semantics).
    let reader = factory
        .create_reader::<T, _, _, N>(&desc, JsonCodec::new())
        .expect("plugin reader");
    let raw_writer = factory
        .create_raw_writer_named::<N>(service_name)
        .expect("gateway raw writer");
    registry.lock().unwrap().register(
        service_name.to_string(),
        routing,
        PdoDirection::Tx,
        ChannelBinding::Inbound(Box::new(IoxInboundPublish::new(raw_writer))),
    );
    reader
}

/// TEST_0220 — plugin send → mock PDI at routing slice (`REQ_0326`).
#[tokio::test]
async fn test_0220_outbound_end_to_end() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry: Arc<Mutex<ChannelRegistry>> =
        Arc::new(Mutex::new(ChannelRegistry::with_capacity(4)));

    // Byte-aligned routing: 8 bits at offset 0 of SubDevice 0x0001's
    // outputs PDI. Plugin writes `u8`; JsonCodec encodes a single
    // digit (e.g. 7 → b"7" = [0x37]). pdi::write_routing then drops
    // those 8 bits at PDI[0].
    let routing = EthercatRouting::new(0x0001, PdoDirection::Rx, 0, 8);
    let writer = open_outbound::<u8, 64>(&factory, &registry, "test_0220_outbound.out", routing);

    // MockBusDriver with a 4-byte outputs buffer for the SubDevice.
    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 3,
            subdevice_count: 1,
        })
        .with_subdevice_outputs(0x0001, vec![0u8; 4]);
    let health = Arc::new(EthercatHealthMonitor::new());
    let options = make_options();
    let mut runner = CycleRunner::new(driver, &options, Arc::clone(&health))
        .await
        .expect("bring_up");
    assert_eq!(health.current().kind(), ConnectorHealthKind::Up);

    // Plugin sends a single u8.
    writer.send(&7u8).expect("plugin writer.send");

    // Drive one dispatcher iteration.
    let report = dispatch_one_cycle(&registry, &mut runner, Instant::now())
        .await
        .expect("dispatch one cycle");
    assert_eq!(report.outbound_envelopes, 1);
    assert!(report.cycle.is_some());

    // Inspect the mock's outputs buffer. JsonCodec on u8(7) writes
    // exactly the ASCII byte 0x37 ('7'); pdi::write_routing's
    // bit-at-a-time RMW copies the low 8 bits verbatim because the
    // routing is byte-aligned.
    let outputs = runner
        .driver()
        .snapshot_outputs(0x0001)
        .expect("outputs configured");
    let json = serde_json::to_vec(&7u8).expect("serde json");
    assert_eq!(json, b"7");
    assert_eq!(&outputs[..1], &json[..1]);
    // Adjacent bytes untouched.
    assert_eq!(&outputs[1..], &[0u8; 3]);
}

/// TEST_0221 — mock PDI preloaded → plugin recv (`REQ_0327`).
#[tokio::test]
async fn test_0221_inbound_end_to_end() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry: Arc<Mutex<ChannelRegistry>> =
        Arc::new(Mutex::new(ChannelRegistry::with_capacity(4)));

    // Same byte-aligned routing as TEST_0220 but Tx direction.
    let routing = EthercatRouting::new(0x0002, PdoDirection::Tx, 0, 8);
    let reader = open_inbound::<u8, 64>(&factory, &registry, "test_0221_inbound.in", routing);

    // Preload mock inputs[0] with JSON encoding of 9: b'9' = 0x39.
    let mut preloaded = vec![0u8; 4];
    preloaded[0] = b'9';
    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 3,
            subdevice_count: 1,
        })
        .with_subdevice_inputs(0x0002, preloaded);
    let health = Arc::new(EthercatHealthMonitor::new());
    let options = make_options();
    let mut runner = CycleRunner::new(driver, &options, Arc::clone(&health))
        .await
        .expect("bring_up");

    // Drive one dispatcher iteration — dispatch_inbound should read
    // PDI[0..1] = b"9" and publish it on the inbound iceoryx2
    // service.
    let report = dispatch_one_cycle(&registry, &mut runner, Instant::now())
        .await
        .expect("dispatch one cycle");
    assert_eq!(report.inbound_envelopes, 1);

    let received = reader
        .try_recv()
        .expect("try_recv")
        .expect("envelope available");
    assert_eq!(received.value, 9);
}

/// TEST_0222 — full round-trip through mock loopback. Plugin sends
/// `v`, MockBusDriver loopback copies outputs → inputs, plugin reads
/// `v` back.
#[tokio::test]
async fn test_0222_loopback_round_trip() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry: Arc<Mutex<ChannelRegistry>> =
        Arc::new(Mutex::new(ChannelRegistry::with_capacity(4)));

    let routing_rx = EthercatRouting::new(0x0003, PdoDirection::Rx, 0, 8);
    let routing_tx = EthercatRouting::new(0x0003, PdoDirection::Tx, 0, 8);
    let writer = open_outbound::<u8, 64>(&factory, &registry, "test_0222_loopback.out", routing_rx);
    let reader = open_inbound::<u8, 64>(&factory, &registry, "test_0222_loopback.in", routing_tx);

    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 3,
            subdevice_count: 1,
        })
        .with_subdevice_outputs(0x0003, vec![0u8; 4])
        .with_subdevice_inputs(0x0003, vec![0u8; 4])
        .with_loopback();
    let health = Arc::new(EthercatHealthMonitor::new());
    let options = make_options();
    let mut runner = CycleRunner::new(driver, &options, Arc::clone(&health))
        .await
        .expect("bring_up");

    writer.send(&3u8).expect("plugin writer.send");

    let report = dispatch_one_cycle(&registry, &mut runner, Instant::now())
        .await
        .expect("dispatch one cycle");
    assert_eq!(report.outbound_envelopes, 1);
    assert_eq!(report.inbound_envelopes, 1);

    let received = reader
        .try_recv()
        .expect("try_recv")
        .expect("envelope available");
    assert_eq!(received.value, 3);
}

/// Smoke-check that JSON-encoded bytes survive the byte-aligned
/// routing path for a multi-byte payload as well. Sends `"abc"` (3
/// chars), routes through a 24-bit byte-aligned slice on the loopback
/// driver, expects `"abc"` to come back.
#[tokio::test]
async fn loopback_round_trip_multi_byte() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry: Arc<Mutex<ChannelRegistry>> =
        Arc::new(Mutex::new(ChannelRegistry::with_capacity(4)));

    let routing_rx = EthercatRouting::new(0x0004, PdoDirection::Rx, 0, 40);
    let routing_tx = EthercatRouting::new(0x0004, PdoDirection::Tx, 0, 40);
    let writer =
        open_outbound::<String, 256>(&factory, &registry, "loopback_multi_byte.out", routing_rx);
    let reader =
        open_inbound::<String, 256>(&factory, &registry, "loopback_multi_byte.in", routing_tx);

    let driver = MockBusDriver::new()
        .with_bring_up(BringUp {
            expected_wkc: 3,
            subdevice_count: 1,
        })
        .with_subdevice_outputs(0x0004, vec![0u8; 32])
        .with_subdevice_inputs(0x0004, vec![0u8; 32])
        .with_loopback();
    let health = Arc::new(EthercatHealthMonitor::new());
    let options = make_options();
    let mut runner = CycleRunner::new(driver, &options, Arc::clone(&health))
        .await
        .unwrap();

    // JSON of "abc" is `"abc"` (5 bytes including the quotes). With
    // a 40-bit routing, we extract those 5 bytes and the codec
    // decodes them back to "abc".
    let original: String = "abc".to_string();
    let json_len = serde_json::to_vec(&original).unwrap().len();
    assert_eq!(json_len, 5, "JSON encoding of \"abc\" is 5 bytes");
    writer.send(&original).expect("send");

    dispatch_one_cycle(&registry, &mut runner, Instant::now())
        .await
        .unwrap();

    let received = reader
        .try_recv()
        .expect("try_recv")
        .expect("envelope available");
    assert_eq!(received.value, original);
}

/// Ensure the PayloadCodec trait import remains live even when
/// `JsonCodec` is the only codec touched — keeps clippy honest about
/// the `#[allow(unused_imports)]` we'd otherwise need.
#[test]
fn json_codec_format_name_is_stable() {
    assert_eq!(JsonCodec::new().format_name(), "json");
}
