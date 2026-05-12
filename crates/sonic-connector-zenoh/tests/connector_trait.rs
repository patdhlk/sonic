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
