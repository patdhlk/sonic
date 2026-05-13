//! Surface-level tests for `ZenohConnector` against `MockZenohSession`.
//!
//! End-to-end pub/sub round-trip is exercised in `tests/end_to_end.rs`
//! (added by Z2 Task 7); this file verifies the type compiles, exposes
//! the expected associated types, and returns sane health snapshots.

use std::sync::Arc;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::ConnectorHealthKind;
use sonic_connector_host::Connector;
use sonic_connector_zenoh::{
    MockZenohSession, ZenohConnector, ZenohConnectorOptions, ZenohRouting, ZenohState,
};

#[test]
fn connector_name_is_zenoh() {
    let opts = ZenohConnectorOptions::builder().build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let connector =
        ZenohConnector::new(state, session, JsonCodec).expect("constructable");
    assert_eq!(connector.name(), "zenoh");
}

#[test]
fn connector_starts_in_connecting_health() {
    let opts = ZenohConnectorOptions::builder().build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let connector =
        ZenohConnector::new(state, session, JsonCodec).expect("constructable");
    assert_eq!(connector.health().kind(), ConnectorHealthKind::Connecting);
}

/// Compile-check: `ZenohConnector<MockZenohSession, JsonCodec>` satisfies
/// the `Connector` trait with the documented associated types.
#[test]
fn connector_associated_types_match_routing_and_codec() {
    fn assert_routing<R, C, T>()
    where
        T: Connector<Routing = R, Codec = C>,
    {
    }
    assert_routing::<ZenohRouting, JsonCodec, ZenohConnector<MockZenohSession, JsonCodec>>();
}

#[test]
fn create_querier_returns_zenoh_querier() {
    use sonic_connector_core::ChannelDescriptor;
    use sonic_connector_zenoh::{KeyExprOwned, ZenohQuerier};

    let opts = ZenohConnectorOptions::builder().build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let connector =
        ZenohConnector::new(state, session, JsonCodec).expect("constructable");

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/query").unwrap());
    let desc = ChannelDescriptor::<ZenohRouting, 256>::new("robot.query".to_string(), routing)
        .unwrap();
    let _q: ZenohQuerier<u32, u32, JsonCodec, 256> =
        connector.create_querier::<u32, u32, 256>(&desc).expect("querier");
}

#[test]
fn create_queryable_returns_zenoh_queryable() {
    use sonic_connector_core::ChannelDescriptor;
    use sonic_connector_zenoh::{KeyExprOwned, ZenohQueryable};

    let opts = ZenohConnectorOptions::builder().build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let connector =
        ZenohConnector::new(state, session, JsonCodec).expect("constructable");

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/query").unwrap());
    let desc = ChannelDescriptor::<ZenohRouting, 256>::new("robot.query".to_string(), routing)
        .unwrap();
    let _qable: ZenohQueryable<u32, u32, JsonCodec, 256> =
        connector.create_queryable::<u32, u32, 256>(&desc).expect("queryable");
}
