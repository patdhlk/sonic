//! TEST_0500 — `CanConnector` implements `Connector` with the
//! framework's required associated types. Compile-time API surface
//! check; no runtime work.

#![allow(clippy::doc_markdown)]

use std::sync::Arc;

use sonic_connector_can::connector::CanState;
use sonic_connector_can::{
    CanConnector, CanConnectorOptions, CanInterfaceLike, CanRouting, MockCanInterface,
};
use sonic_connector_codec::JsonCodec;
use sonic_connector_core::ChannelDescriptor;
use sonic_connector_host::Connector;
use sonic_connector_transport_iox::{ChannelReader, ChannelWriter};

#[test]
fn can_connector_implements_connector_with_required_associated_types() {
    let iface = sonic_connector_can::CanIface::new("vcan0").unwrap();
    let opts = CanConnectorOptions::builder().iface(iface).build();
    let state = Arc::new(CanState::new(opts));
    let driver = MockCanInterface::new(iface);
    let connector =
        CanConnector::<MockCanInterface, JsonCodec>::new(state, vec![driver], JsonCodec::new())
            .expect("construct CanConnector");

    // Associated-type witnesses.
    fn requires_routing<T: sonic_connector_core::Routing>() {}
    fn requires_codec<T: sonic_connector_core::PayloadCodec>() {}
    requires_routing::<<CanConnector<MockCanInterface, JsonCodec> as Connector>::Routing>();
    requires_codec::<<CanConnector<MockCanInterface, JsonCodec> as Connector>::Codec>();

    // Connector::name + health surface.
    assert_eq!(connector.name(), "can");
    let _ = connector.health(); // smoke

    // create_writer / create_reader return concrete handles, not boxed
    // trait objects (REQ_0223). The types appear in this signature to
    // assert that statically.
    fn returns_concrete_writer(_w: ChannelWriter<u32, JsonCodec, 8>) {}
    fn returns_concrete_reader(_r: ChannelReader<u32, JsonCodec, 8>) {}

    let routing = CanRouting::new(
        iface,
        sonic_connector_can::CanId::standard(0x100).unwrap(),
        0x7FF,
        sonic_connector_can::CanFrameKind::Classical,
    );
    let desc = ChannelDescriptor::<CanRouting, 8>::new("trait_surface.classical", routing).unwrap();
    let writer = connector.create_writer::<u32, 8>(&desc).unwrap();
    let reader = connector.create_reader::<u32, 8>(&desc).unwrap();
    returns_concrete_writer(writer);
    returns_concrete_reader(reader);

    // Smoke: MockCanInterface implements the driver trait.
    fn requires_can_interface_like<T: CanInterfaceLike>() {}
    requires_can_interface_like::<MockCanInterface>();
}
