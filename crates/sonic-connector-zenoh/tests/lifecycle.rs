//! Verify connector teardown cleans up subscriptions and queryables.
//! No `Box::leak` — when the connector drops, the mock session's
//! registries should be empty.

use std::sync::Arc;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::ChannelDescriptor;
use sonic_connector_host::Connector;
use sonic_connector_zenoh::{
    KeyExprOwned, MockZenohSession, ZenohConnector, ZenohConnectorOptions,
    ZenohRouting, ZenohState,
};

const N: usize = 128;

#[test]
fn dropping_connector_clears_mock_session_subscribers() {
    let session = Arc::new(MockZenohSession::new());
    {
        let opts = ZenohConnectorOptions::builder().build();
        let state = Arc::new(ZenohState::new(opts));
        let connector = ZenohConnector::new(state, Arc::clone(&session), JsonCodec).unwrap();

        let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/lc").unwrap());
        let desc = ChannelDescriptor::<ZenohRouting, N>::new(
            "robot.lc".to_string(),
            routing,
        ).unwrap();
        let _reader = connector.create_reader::<u32, N>(&desc).expect("reader");
        assert_eq!(session.subscriber_count(), 1);
    }
    assert_eq!(
        session.subscriber_count(),
        0,
        "mock subscriber should be removed when its handle drops"
    );
}

#[test]
fn dropping_connector_clears_mock_session_queryables() {
    let session = Arc::new(MockZenohSession::new());
    {
        let opts = ZenohConnectorOptions::builder().build();
        let state = Arc::new(ZenohState::new(opts));
        let connector = ZenohConnector::new(state, Arc::clone(&session), JsonCodec).unwrap();

        let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/lc").unwrap());
        let desc = ChannelDescriptor::<ZenohRouting, N>::new(
            "robot.lc".to_string(),
            routing,
        ).unwrap();
        let _qable = connector.create_queryable::<u32, u32, N>(&desc).expect("queryable");
        assert_eq!(session.queryable_count(), 1);
    }
    assert_eq!(
        session.queryable_count(),
        0,
        "mock queryable should be removed when its handle drops"
    );
}
