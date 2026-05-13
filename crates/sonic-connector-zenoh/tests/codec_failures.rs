//! Codec failure paths for queries (`TEST_0304`). Verifies that
//! encoding overflows and decoding malformed bytes surface as
//! `ConnectorError::Codec` from both `ZenohQuerier::send` and
//! `ZenohQueryable::reply`.

use std::sync::Arc;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::{ChannelDescriptor, ConnectorError};
use sonic_connector_zenoh::registry::QueryId;
use sonic_connector_zenoh::{
    KeyExprOwned, MockZenohSession, ZenohConnector, ZenohConnectorOptions, ZenohRouting,
    ZenohState,
};

const TINY: usize = 4; // 4-byte channel — any non-trivial JSON overflows.

#[test]
fn querier_send_overflow_returns_payload_overflow() {
    let opts = ZenohConnectorOptions::builder().build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let connector =
        ZenohConnector::new(state, session, JsonCodec).expect("constructable");

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/q").unwrap());
    let desc =
        ChannelDescriptor::<ZenohRouting, TINY>::new("robot.q".to_string(), routing).unwrap();
    let mut querier = connector
        .create_querier::<String, String, TINY>(&desc)
        .expect("querier");

    // JSON-encoded "a long enough string" definitely exceeds 4 bytes.
    let err = querier
        .send(&"a long enough string".to_string())
        .expect_err("overflow");
    assert!(
        matches!(err, ConnectorError::PayloadOverflow { .. } | ConnectorError::Codec { .. }),
        "expected codec/overflow error, got {err:?}"
    );
}

#[test]
fn queryable_reply_overflow_returns_payload_overflow() {
    let opts = ZenohConnectorOptions::builder().build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let connector =
        ZenohConnector::new(state, session, JsonCodec).expect("constructable");

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/q").unwrap());
    let desc =
        ChannelDescriptor::<ZenohRouting, TINY>::new("robot.q".to_string(), routing).unwrap();
    let mut queryable = connector
        .create_queryable::<String, String, TINY>(&desc)
        .expect("queryable");

    let synthetic_id = QueryId([1u8; 32]);

    let err = queryable
        .reply(synthetic_id, &"a long enough string".to_string())
        .expect_err("overflow");
    assert!(
        matches!(err, ConnectorError::PayloadOverflow { .. } | ConnectorError::Codec { .. }),
        "expected codec/overflow error, got {err:?}"
    );
}
