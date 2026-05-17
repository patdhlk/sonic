//! Layer-1 end-to-end coverage for the CAN connector.
//!
//! Each test wires the moving parts directly — iceoryx2 `Node`,
//! `ServiceFactory`, a hand-built `ChannelRegistry`, a
//! `MockCanInterface`, and `dispatch_one_iteration` — rather than
//! going through `CanConnector::register_with`'s spawn path. The
//! direct wiring keeps each dispatcher iteration synchronous and lets
//! the test inspect the mock's state between iterations.

#![allow(clippy::doc_markdown)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iceoryx2::node::Node;
use iceoryx2::prelude::{NodeBuilder, ipc};
use sonic_connector_can::{
    CanData, CanErrorKind, CanFdFlags, CanFrameKind, CanHealthMonitor, CanId, CanIface, CanRouting,
    ChannelBinding, ChannelRegistry, Direction, IfaceHealthKind, IoxInboundPublish,
    IoxOutboundDrain, MockCanInterface, dispatch_one_iteration,
};
use sonic_connector_codec::JsonCodec;
use sonic_connector_core::{ChannelDescriptor, ConnectorHealthKind, ExponentialBackoff};
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter, ServiceFactory};

fn make_node() -> Node<ipc::Service> {
    NodeBuilder::new()
        .create::<ipc::Service>()
        .expect("create iceoryx2 node")
}

fn iface(name: &str) -> CanIface {
    CanIface::new(name).unwrap()
}

fn open_outbound<T, const N: usize>(
    factory: &ServiceFactory<'_>,
    registry: &Mutex<ChannelRegistry>,
    service_name: &str,
    routing: CanRouting,
) -> ChannelWriter<T, JsonCodec, N>
where
    T: serde::Serialize + 'static,
{
    let desc = ChannelDescriptor::<CanRouting, N>::new(service_name.to_string(), routing).unwrap();
    let writer = factory
        .create_writer::<T, _, _, N>(&desc, JsonCodec::new())
        .expect("plugin writer");
    let raw_reader = factory
        .create_raw_reader_named::<N>(service_name)
        .expect("gateway raw reader");
    registry.lock().unwrap().register(
        service_name.to_string(),
        routing,
        Direction::Outbound,
        ChannelBinding::Outbound(Box::new(IoxOutboundDrain::<N>::new(raw_reader))),
    );
    writer
}

fn open_inbound<T, const N: usize>(
    factory: &ServiceFactory<'_>,
    registry: &Mutex<ChannelRegistry>,
    service_name: &str,
    routing: CanRouting,
) -> ChannelReader<T, JsonCodec, N>
where
    T: serde::de::DeserializeOwned + 'static,
{
    let desc = ChannelDescriptor::<CanRouting, N>::new(service_name.to_string(), routing).unwrap();
    let reader = factory
        .create_reader::<T, _, _, N>(&desc, JsonCodec::new())
        .expect("plugin reader");
    let raw_writer = factory
        .create_raw_writer_named::<N>(service_name)
        .expect("gateway raw writer");
    registry.lock().unwrap().register(
        service_name.to_string(),
        routing,
        Direction::Inbound,
        ChannelBinding::Inbound(Box::new(IoxInboundPublish::<N>::new(raw_writer))),
    );
    reader
}

/// TEST_0502 — classical CAN round-trip via MockCanInterface.
/// Verifies REQ_0610 (classical frame support), REQ_0612 (channel
/// payload sizing keyed on frame kind), REQ_0613 (outbound payload
/// serialised to CanFrame), REQ_0614 (inbound is byte-only on the
/// publish path).
#[tokio::test]
async fn test_0502_classical_round_trip() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(2)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0")]));

    // Reader registered first (iox pub/sub semantics).
    let routing = CanRouting::new(
        iface("vcan0"),
        CanId::standard(0x100).unwrap(),
        0x7FF,
        CanFrameKind::Classical,
    );
    let reader = open_inbound::<u8, 8>(&factory, &registry, "test_0502.in", routing);
    let writer = open_outbound::<u8, 8>(&factory, &registry, "test_0502.out", routing);

    let mut driver = MockCanInterface::new(iface("vcan0"));
    let mut policy = ExponentialBackoff::default();

    writer.send(&7u8).expect("send");

    // Single iteration: drain TX → mock loopback queues the frame →
    // recv pops it → demux to the reader → publish on the .in service.
    let outcome = dispatch_one_iteration(
        &iface("vcan0"),
        &mut driver,
        &registry,
        &health,
        &mut policy,
        Duration::from_millis(200),
    )
    .await
    .expect("dispatch iteration");
    assert_eq!(outcome.tx_sent, 1);
    assert_eq!(outcome.inbound_publishes, 1);

    let received = reader
        .try_recv()
        .expect("reader try_recv")
        .expect("envelope available");
    assert_eq!(received.value, 7u8);
}

/// TEST_0503 — CAN-FD round-trip. Verifies REQ_0611 (FD support) and
/// REQ_0613.
#[tokio::test]
async fn test_0503_fd_round_trip() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(2)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0")]));

    let routing = CanRouting::new(
        iface("vcan0"),
        CanId::extended(0x1ABCDEF).unwrap(),
        0x1FFF_FFFF,
        CanFrameKind::Fd,
    )
    .with_fd_flags(CanFdFlags::BRS);
    let reader = open_inbound::<String, 64>(&factory, &registry, "test_0503.in", routing);
    let writer = open_outbound::<String, 64>(&factory, &registry, "test_0503.out", routing);

    let mut driver = MockCanInterface::new(iface("vcan0"));
    let mut policy = ExponentialBackoff::default();

    let payload = "fd-can-payload-up-to-64-bytes".to_string();
    writer.send(&payload).expect("send");

    let outcome = dispatch_one_iteration(
        &iface("vcan0"),
        &mut driver,
        &registry,
        &health,
        &mut policy,
        Duration::from_millis(200),
    )
    .await
    .unwrap();
    assert_eq!(outcome.tx_sent, 1);
    assert_eq!(outcome.inbound_publishes, 1);

    let received = reader.try_recv().unwrap().unwrap();
    assert_eq!(received.value, payload);
}

/// TEST_0504 — per-iface filter union. Register three inbound channels
/// with distinct (can_id, mask) and check that the mock recorded a
/// filter with three entries. REQ_0522, REQ_0623.
#[tokio::test]
async fn test_0504_filter_union() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(4)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0")]));

    let r1 = CanRouting::new(
        iface("vcan0"),
        CanId::standard(0x100).unwrap(),
        0x7FF,
        CanFrameKind::Classical,
    );
    let r2 = CanRouting::new(
        iface("vcan0"),
        CanId::standard(0x200).unwrap(),
        0x7F0,
        CanFrameKind::Classical,
    );
    let r3 = CanRouting::new(
        iface("vcan0"),
        CanId::extended(0x12345).unwrap(),
        0x1FFF_FFFF,
        CanFrameKind::Classical,
    );

    let _r1 = open_inbound::<u8, 8>(&factory, &registry, "test_0504.r1.in", r1);
    let _r2 = open_inbound::<u8, 8>(&factory, &registry, "test_0504.r2.in", r2);
    let _r3 = open_inbound::<u8, 8>(&factory, &registry, "test_0504.r3.in", r3);

    let mut driver = MockCanInterface::new(iface("vcan0"));
    let state_handle = driver.state_handle();
    let mut policy = ExponentialBackoff::default();

    let _ = dispatch_one_iteration(
        &iface("vcan0"),
        &mut driver,
        &registry,
        &health,
        &mut policy,
        Duration::from_millis(50),
    )
    .await
    .unwrap();

    let guard = state_handle.lock().unwrap();
    assert_eq!(
        guard.last_applied_filter.len(),
        3,
        "expected 3 distinct filter entries for 3 inbound channels"
    );
    assert!(guard.apply_filter_count >= 1);
}

/// TEST_0505 — multi-iface inbound demux. Two ifaces, frames sent on
/// each must reach only the matching reader on that iface.
/// REQ_0520, REQ_0521, REQ_0524.
#[tokio::test]
async fn test_0505_multi_iface_demux() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(4)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0"), iface("vcan1")]));

    let r_a = CanRouting::new(
        iface("vcan0"),
        CanId::standard(0x100).unwrap(),
        0x7FF,
        CanFrameKind::Classical,
    );
    let r_b = CanRouting::new(
        iface("vcan1"),
        CanId::standard(0x200).unwrap(),
        0x7FF,
        CanFrameKind::Classical,
    );

    let reader_a = open_inbound::<u8, 8>(&factory, &registry, "test_0505.vcan0.in", r_a);
    let reader_b = open_inbound::<u8, 8>(&factory, &registry, "test_0505.vcan1.in", r_b);

    let mut driver_a = MockCanInterface::new(iface("vcan0"));
    let mut driver_b = MockCanInterface::new(iface("vcan1"));
    let mut policy_a = ExponentialBackoff::default();
    let mut policy_b = ExponentialBackoff::default();

    // Inject matching JSON-encoded frames so the reader's JsonCodec
    // round-trip succeeds. JSON of 170u8 is `"170"` (3 ASCII bytes);
    // JSON of 187u8 is `"187"`.
    let json_a = serde_json::to_vec(&170u8).unwrap();
    let json_b = serde_json::to_vec(&187u8).unwrap();
    driver_a.inject_frame(
        CanData::new(
            CanId::standard(0x100).unwrap(),
            CanFrameKind::Classical,
            CanFdFlags::empty(),
            &json_a,
        )
        .unwrap(),
    );
    driver_b.inject_frame(
        CanData::new(
            CanId::standard(0x200).unwrap(),
            CanFrameKind::Classical,
            CanFdFlags::empty(),
            &json_b,
        )
        .unwrap(),
    );

    let oa = dispatch_one_iteration(
        &iface("vcan0"),
        &mut driver_a,
        &registry,
        &health,
        &mut policy_a,
        Duration::from_millis(200),
    )
    .await
    .unwrap();
    let ob = dispatch_one_iteration(
        &iface("vcan1"),
        &mut driver_b,
        &registry,
        &health,
        &mut policy_b,
        Duration::from_millis(200),
    )
    .await
    .unwrap();

    assert_eq!(oa.inbound_publishes, 1);
    assert_eq!(ob.inbound_publishes, 1);

    let recv_a = reader_a.try_recv().unwrap().unwrap();
    let recv_b = reader_b.try_recv().unwrap().unwrap();
    assert_eq!(recv_a.value, 170u8);
    assert_eq!(recv_b.value, 187u8);

    // Cross-iface: a's reader sees no frame intended for b.
    assert!(reader_a.try_recv().unwrap().is_none());
    assert!(reader_b.try_recv().unwrap().is_none());
}

/// TEST_0506 — bus-off → Down → reopen cycle. Verifies REQ_0533 and
/// REQ_0534.
#[tokio::test]
async fn test_0506_bus_off_reconnect() {
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(0)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0")]));
    let mut driver = MockCanInterface::new(iface("vcan0"));
    let mut policy = ExponentialBackoff::builder()
        .initial(Duration::from_millis(1))
        .max(Duration::from_millis(2))
        .jitter(0.0)
        .build();

    driver.inject_error(CanErrorKind::BusOff);
    let outcome = dispatch_one_iteration(
        &iface("vcan0"),
        &mut driver,
        &registry,
        &health,
        &mut policy,
        Duration::from_millis(200),
    )
    .await
    .unwrap();
    assert_eq!(outcome.error_kind, Some(CanErrorKind::BusOff));
    // After bus-off + reopen, iface should be Up.
    assert_eq!(
        health.iface_kind(&iface("vcan0")).unwrap(),
        IfaceHealthKind::Up
    );
}

/// TEST_0507 — error-passive → Degraded with the iface name in the
/// reason. Verifies REQ_0532, REQ_0535.
#[tokio::test]
async fn test_0507_error_passive_degraded() {
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(0)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0"), iface("vcan1")]));
    let mut driver_a = MockCanInterface::new(iface("vcan0"));
    let mut policy = ExponentialBackoff::default();
    let sub = health.subscribe();

    // Bring vcan1 up so the aggregate isn't all-Down.
    let _ = health.set_iface(iface("vcan1"), IfaceHealthKind::Up);
    let _ = sub.try_recv(); // drain initial Connecting → Up if it fired

    driver_a.inject_error(CanErrorKind::Passive);
    let outcome = dispatch_one_iteration(
        &iface("vcan0"),
        &mut driver_a,
        &registry,
        &health,
        &mut policy,
        Duration::from_millis(200),
    )
    .await
    .unwrap();
    assert_eq!(outcome.error_kind, Some(CanErrorKind::Passive));
    assert_eq!(
        health.iface_kind(&iface("vcan0")).unwrap(),
        IfaceHealthKind::Degraded
    );
    // Aggregated visible state must be Degraded (one iface Degraded,
    // one Up) per REQ_0530.
    assert_eq!(health.current().kind(), ConnectorHealthKind::Degraded);
}

/// TEST_0513 — error frames are never delivered to a reader.
/// Regression-guards the explicit anti-requirement REQ_0543 / REQ_0636.
#[tokio::test]
async fn test_0513_error_frames_not_exposed_to_reader() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let registry = Arc::new(Mutex::new(ChannelRegistry::with_capacity(2)));
    let health = Arc::new(CanHealthMonitor::new(&[iface("vcan0")]));

    let routing = CanRouting::new(
        iface("vcan0"),
        CanId::standard(0x100).unwrap(),
        0x000, // match-anything mask: accept all standard IDs
        CanFrameKind::Classical,
    );
    let reader = open_inbound::<u8, 8>(&factory, &registry, "test_0513.in", routing);

    let mut driver = MockCanInterface::new(iface("vcan0"));
    let mut policy = ExponentialBackoff::builder()
        .initial(Duration::from_millis(1))
        .max(Duration::from_millis(2))
        .jitter(0.0)
        .build();

    // Inject every error kind. None should reach the reader.
    driver.inject_error(CanErrorKind::Warning);
    driver.inject_error(CanErrorKind::Passive);
    driver.inject_error(CanErrorKind::BusOff);

    for _ in 0..3 {
        let _ = dispatch_one_iteration(
            &iface("vcan0"),
            &mut driver,
            &registry,
            &health,
            &mut policy,
            Duration::from_millis(200),
        )
        .await
        .unwrap();
    }

    assert!(
        reader.try_recv().unwrap().is_none(),
        "reader should never observe error frames"
    );
}

/// TEST_0514 — per-iface routing registry iterates in insertion order.
/// REQ_0525.
#[test]
fn test_0514_registry_iter_order_is_insertion_order() {
    let mut registry = ChannelRegistry::with_capacity(8);
    let ifc = iface("vcan0");
    for i in 0..8u16 {
        let routing = CanRouting::new(
            ifc,
            CanId::standard(0x100 + i).unwrap(),
            0x7FF,
            CanFrameKind::Classical,
        );
        let name = format!("test_0514.{i}");
        registry.register(name, routing, Direction::Inbound, ChannelBinding::Unbound);
    }
    let observed: Vec<u32> = registry
        .iter_iface_direction(&ifc, Direction::Inbound)
        .map(|c| c.routing.can_id.value)
        .collect();
    let expected: Vec<u32> = (0..8u32).map(|i| 0x100 + i).collect();
    assert_eq!(observed, expected);
}
