//! Per-call timeout propagation (`REQ_0425`) + synthetic `0x03`
//! terminator (`TEST_0307`). Exercises:
//! - `ZenohQuerier::send_with_timeout` writes `timeout_ms` into the
//!   envelope's `reserved` header word.
//! - The dispatcher reads `reserved` and uses it as the
//!   `tokio::time::timeout` budget.
//! - When the mock's `query` hangs (test-only knob), the dispatcher
//!   emits a `[0x03]` frame; the querier observes
//!   `QuerierEvent::Timeout`.

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
fn query_timeout_emits_synthetic_terminator() {
    // TEST_0307: with the mock hanging, the dispatcher's timeout fires
    // and the querier sees QuerierEvent::Timeout.
    let session = Arc::new(MockZenohSession::new());
    session.set_query_hangs(true);

    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(Duration::from_millis(1))
        .query_timeout(Duration::from_millis(100))
        .tokio_worker_threads(1)
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let mut connector = ZenohConnector::new(state, session, JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/timeout").unwrap());
    let desc =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.timeout".to_string(), routing).unwrap();
    let mut querier = connector.create_querier::<u32, String, N>(&desc).unwrap();

    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut exec).unwrap();

    let q_id = querier.send(&42_u32).expect("send query");

    let mut saw_timeout = false;
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    while !saw_timeout && std::time::Instant::now() < deadline {
        if let Ok(Some(event)) = querier.try_recv() {
            match event {
                QuerierEvent::Timeout { id } => {
                    assert_eq!(id, q_id);
                    saw_timeout = true;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(saw_timeout, "expected QuerierEvent::Timeout within 800ms");
}

#[test]
fn per_call_timeout_overrides_default() {
    // Connector default is 10s; per-call says 100ms. Querier should see
    // Timeout in ~100-300ms range (not 10s).
    let session = Arc::new(MockZenohSession::new());
    session.set_query_hangs(true);

    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(Duration::from_millis(1))
        .query_timeout(Duration::from_secs(10)) // 10 SECONDS default
        .tokio_worker_threads(1)
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let mut connector = ZenohConnector::new(state, session, JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/pcto").unwrap());
    let desc =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.pcto".to_string(), routing).unwrap();
    let mut querier = connector.create_querier::<u32, String, N>(&desc).unwrap();

    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut exec).unwrap();

    let started = std::time::Instant::now();
    let q_id = querier
        .send_with_timeout(&42_u32, Duration::from_millis(100))
        .expect("send_with_timeout");

    let mut saw_timeout = false;
    while !saw_timeout && started.elapsed() < Duration::from_secs(2) {
        if let Ok(Some(QuerierEvent::Timeout { id })) = querier.try_recv() {
            assert_eq!(id, q_id);
            saw_timeout = true;
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    let elapsed = started.elapsed();
    assert!(saw_timeout, "expected Timeout");
    assert!(
        elapsed < Duration::from_secs(2),
        "per-call 100ms timeout should fire well before the 10s default; took {elapsed:?}"
    );
}

#[test]
fn per_call_timeout_zero_uses_default() {
    let session = Arc::new(MockZenohSession::new());
    session.set_query_hangs(true);

    let opts = ZenohConnectorOptions::builder()
        .dispatcher_tick(Duration::from_millis(1))
        .query_timeout(Duration::from_millis(150)) // default = 150ms
        .tokio_worker_threads(1)
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let mut connector = ZenohConnector::new(state, session, JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/dflt").unwrap());
    let desc =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.dflt".to_string(), routing).unwrap();
    let mut querier = connector.create_querier::<u32, String, N>(&desc).unwrap();

    let mut exec = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut exec).unwrap();

    let started = std::time::Instant::now();
    let q_id = querier.send(&42_u32).unwrap(); // reserved=0, use default

    let mut saw_timeout = false;
    while !saw_timeout && started.elapsed() < Duration::from_secs(1) {
        if let Ok(Some(QuerierEvent::Timeout { id })) = querier.try_recv() {
            assert_eq!(id, q_id);
            saw_timeout = true;
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(saw_timeout, "expected default-150ms timeout to fire");
}
