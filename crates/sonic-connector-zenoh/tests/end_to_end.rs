//! End-to-end pub/sub round-trip via `MockZenohSession`. Maps to
//! `TEST_0302` (pub/sub end-to-end against `MockZenohSession`).

use std::sync::Arc;
use std::time::Duration;

use sonic_connector_codec::JsonCodec;
use sonic_connector_core::ChannelDescriptor;
use sonic_connector_host::Connector;
use sonic_connector_zenoh::{
    KeyExprOwned, MockZenohSession, ZenohConnector, ZenohConnectorOptions, ZenohRouting,
    ZenohState,
};
use sonic_executor::Executor;

const N: usize = 256;

#[test]
fn pub_sub_round_trip_through_mock_session() {
    let opts = ZenohConnectorOptions::builder()
        .tokio_worker_threads(1)
        .dispatcher_tick(Duration::from_millis(1))
        .build();
    let state = Arc::new(ZenohState::new(opts));
    let session = Arc::new(MockZenohSession::new());
    let mut connector =
        ZenohConnector::new(state, Arc::clone(&session), JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/arm/joint1").unwrap());
    let desc_reader =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.arm.joint1".to_string(), routing.clone())
            .unwrap();
    let desc_writer =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.arm.joint1".to_string(), routing)
            .unwrap();

    // Create reader BEFORE writer so the session subscriber is in
    // place before the first publish reaches the dispatcher.
    let reader = connector
        .create_reader::<u32, N>(&desc_reader)
        .expect("reader");
    let writer = connector
        .create_writer::<u32, N>(&desc_writer)
        .expect("writer");

    let mut executor = Executor::builder().worker_threads(0).build().unwrap();
    connector.register_with(&mut executor).unwrap();

    // Publish a couple of values. `send` takes `&T`.
    writer.send(&42_u32).expect("send 42");
    writer.send(&43_u32).expect("send 43");

    // Dispatcher ticks at 1ms; allow up to 500ms for both round-trips.
    let mut got: Vec<u32> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while got.len() < 2 && std::time::Instant::now() < deadline {
        if let Ok(Some(env)) = reader.try_recv() {
            got.push(env.value);
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(got.len(), 2, "expected 2 round-tripped values, got {got:?}");
    assert_eq!(got[0], 42);
    assert_eq!(got[1], 43);
}
