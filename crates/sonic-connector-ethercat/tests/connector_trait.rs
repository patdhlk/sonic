//! TEST_0200 (partial) — `EthercatConnector<C>` implements the
//! framework's `Connector` trait. The full test asserts both surface
//! shape (this commit) and behaviour against real bus traffic (C5b
//! once ethercrab integration lands).

#![allow(clippy::doc_markdown)]

use std::sync::Arc;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::{ChannelDescriptor, ConnectorHealth, ConnectorHealthKind};
use sonic_connector_ethercat::connector::EthercatState;
use sonic_connector_ethercat::{
    EthercatConnector, EthercatConnectorOptions, EthercatRouting, PdoDirection,
};
use sonic_connector_host::Connector;

fn make_connector() -> EthercatConnector<JsonCodec> {
    let opts = EthercatConnectorOptions::builder().build();
    let state = Arc::new(EthercatState::new(opts));
    EthercatConnector::new(state, JsonCodec::new()).expect("construct EthercatConnector")
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
fn create_writer_and_reader_round_trip_through_iox() {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Frame {
        seq: u32,
        note: String,
    }

    let c = make_connector();
    let routing = EthercatRouting::new(0x0001, PdoDirection::Tx, 0, 16);
    let desc: ChannelDescriptor<EthercatRouting, 1024> =
        ChannelDescriptor::new("connector_trait.round_trip", routing).unwrap();

    // Reader first so the subscriber is attached before the
    // publisher's first send (iceoryx2 default behaviour).
    let reader = c.create_reader::<Frame, 1024>(&desc).expect("reader");
    let writer = c.create_writer::<Frame, 1024>(&desc).expect("writer");

    let frame = Frame {
        seq: 42,
        note: "ethercat-stub".into(),
    };
    writer.send(&frame).expect("send");
    let received = reader
        .try_recv()
        .expect("try_recv")
        .expect("envelope present");
    assert_eq!(received.value, frame);
}
