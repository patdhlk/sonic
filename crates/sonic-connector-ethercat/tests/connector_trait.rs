//! TEST_0200 (partial) — `EthercatConnector<D, C>` implements the
//! framework's `Connector` trait. The full test asserts both surface
//! shape (this commit) and behaviour against real bus traffic (TEST_0220
//! / TEST_0221 / TEST_0222 land that via the dispatcher in C7b).

#![allow(clippy::doc_markdown)]

use std::sync::Arc;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::{ChannelDescriptor, ConnectorHealth, ConnectorHealthKind};
use sonic_connector_ethercat::connector::EthercatState;
use sonic_connector_ethercat::{
    EthercatConnector, EthercatConnectorOptions, EthercatRouting, MockBusDriver, PdoDirection,
};
use sonic_connector_host::Connector;

fn make_connector() -> EthercatConnector<MockBusDriver, JsonCodec> {
    let opts = EthercatConnectorOptions::builder().build();
    let state = Arc::new(EthercatState::new(opts));
    EthercatConnector::new(state, MockBusDriver::new(), JsonCodec::new())
        .expect("construct EthercatConnector")
}

#[test]
fn connector_reports_static_name() {
    let c = make_connector();
    assert_eq!(c.name(), "ethercat");
}

#[test]
fn fresh_connector_starts_in_connecting_state() {
    let c = make_connector();
    // ConnectorHealth's initial state per ARCH_0012 is Connecting.
    assert_eq!(c.health().kind(), ConnectorHealthKind::Connecting);
}

#[test]
fn subscribe_health_observes_internal_transitions() {
    let c = make_connector();
    let sub = c.subscribe_health();
    // No events yet — try_next returns Ok(None).
    let first = sub.try_next().expect("not disconnected");
    assert!(first.is_none());

    // Drive a Connecting → Up transition through the shared state.
    c.state()
        .health()
        .transition_to(ConnectorHealth::Up)
        .expect("legal transition");

    let observed = sub
        .try_next()
        .expect("not disconnected")
        .expect("event available");
    assert_eq!(observed.from.kind(), ConnectorHealthKind::Connecting);
    assert_eq!(observed.to.kind(), ConnectorHealthKind::Up);
}

#[test]
fn create_writer_registers_an_outbound_channel() {
    let c = make_connector();
    let routing = EthercatRouting::new(0x0001, PdoDirection::Rx, 0, 16);
    let desc: ChannelDescriptor<EthercatRouting, 1024> =
        ChannelDescriptor::new("connector_trait.create_writer.out", routing).unwrap();
    let _writer = c
        .create_writer::<u32, 1024>(&desc)
        .expect("create_writer succeeds");

    let registry = c
        .state()
        .registry()
        .lock()
        .expect("registry mutex not poisoned");
    let len = registry.len();
    let entry = registry.iter().next().unwrap();
    let address = entry.routing.subdevice_address;
    let direction = entry.direction;
    drop(registry);
    assert_eq!(len, 1);
    assert_eq!(address, 0x0001);
    assert_eq!(direction, PdoDirection::Rx);
}

#[test]
fn create_reader_registers_an_inbound_channel() {
    let c = make_connector();
    let routing = EthercatRouting::new(0x0002, PdoDirection::Tx, 0, 16);
    let desc: ChannelDescriptor<EthercatRouting, 1024> =
        ChannelDescriptor::new("connector_trait.create_reader.in", routing).unwrap();
    let _reader = c
        .create_reader::<u32, 1024>(&desc)
        .expect("create_reader succeeds");

    let registry = c
        .state()
        .registry()
        .lock()
        .expect("registry mutex not poisoned");
    let len = registry.len();
    let entry = registry.iter().next().unwrap();
    let address = entry.routing.subdevice_address;
    let direction = entry.direction;
    drop(registry);
    assert_eq!(len, 1);
    assert_eq!(address, 0x0002);
    assert_eq!(direction, PdoDirection::Tx);
}

#[test]
fn create_writer_rejects_tx_routing() {
    let c = make_connector();
    let routing = EthercatRouting::new(0x0001, PdoDirection::Tx, 0, 16);
    let desc: ChannelDescriptor<EthercatRouting, 1024> =
        ChannelDescriptor::new("connector_trait.wrong_direction", routing).unwrap();
    let result = c.create_writer::<u32, 1024>(&desc);
    let Err(err) = result else {
        panic!("must reject Tx routing for create_writer");
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("RxPDO") || msg.contains("create_writer"),
        "{msg}"
    );
}

#[test]
fn create_reader_rejects_rx_routing() {
    let c = make_connector();
    let routing = EthercatRouting::new(0x0001, PdoDirection::Rx, 0, 16);
    let desc: ChannelDescriptor<EthercatRouting, 1024> =
        ChannelDescriptor::new("connector_trait.wrong_direction2", routing).unwrap();
    let result = c.create_reader::<u32, 1024>(&desc);
    let Err(err) = result else {
        panic!("must reject Rx routing for create_reader");
    };
    let msg = format!("{err}");
    assert!(
        msg.contains("TxPDO") || msg.contains("create_reader"),
        "{msg}"
    );
}
