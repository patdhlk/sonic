//! Z5c: late-reply dedup via the `sealed_queries` sidecar.
//!
//! When `tokio::time::timeout` fires the synthetic `[0x03]` `Timeout`
//! frame in the dispatcher, an upstream zenoh reply callback may still
//! fire later and try to publish a stray `Reply (0x01)` or
//! `EndOfStream (0x02)` frame on the same correlation id. Real-session
//! parity (`TODO(Z4f)`).
//!
//! This test simulates that race against `MockZenohSession` via the
//! `force_late_reply` test knob: hang the query so the gateway's
//! `tokio::time::timeout` fires; observe the `Timeout` event; then
//! force the captured reply/done callbacks to fire LATE; drain the
//! querier and assert zero further events surface.

use std::sync::Arc;
use std::time::Duration;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::ChannelDescriptor;
use sonic_connector_host::Connector;
use sonic_connector_zenoh::{
    KeyExprOwned, MockZenohSession, QuerierEvent, ZenohConnector, ZenohConnectorOptions,
    ZenohRouting, ZenohState,
};
use sonic_executor::Executor;

const N: usize = 256;

#[test]
fn late_reply_after_timeout_is_dropped() {
    let session = Arc::new(MockZenohSession::new());
    session.set_query_hangs(true);

    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(Duration::from_millis(1))
        .query_timeout(Duration::from_millis(100))
        .tokio_worker_threads(1)
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let mut connector = ZenohConnector::new(state, Arc::clone(&session), JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/late").unwrap());
    let desc = ChannelDescriptor::<ZenohRouting, N>::new(
        "robot.late".to_string(),
        routing,
    )
    .unwrap();
    let mut querier = connector.create_querier::<u32, String, N>(&desc).unwrap();

    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut exec).unwrap();

    let q_id = querier.send(&42_u32).expect("send query");

    // Wait for the synthetic Timeout event.
    let mut saw_timeout = false;
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    while !saw_timeout && std::time::Instant::now() < deadline {
        if let Ok(Some(event)) = querier.try_recv() {
            match event {
                QuerierEvent::Timeout { id } => {
                    assert_eq!(id, q_id);
                    saw_timeout = true;
                }
                other => panic!("unexpected event before Timeout: {other:?}"),
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(saw_timeout, "expected Timeout within 800ms");

    // Unhang the mock and fire the late callbacks captured when the
    // query was made. With the `sealed_queries` sidecar in place the
    // dispatcher's reply/done closures should drop both the data
    // chunk and the end-of-stream frame.
    session.set_query_hangs(false);
    let fired = session.force_late_reply("robot/late", b"\"late!\"");
    assert!(
        fired,
        "expected captured callbacks for robot/late; force_late_reply returned false"
    );

    // Drain for 200ms — assert ZERO further events surface.
    let drain_deadline = std::time::Instant::now() + Duration::from_millis(200);
    while std::time::Instant::now() < drain_deadline {
        if let Ok(Some(event)) = querier.try_recv() {
            panic!("unexpected event after Timeout: {event:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
