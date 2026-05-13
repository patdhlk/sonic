//! End-to-end query round-trip via `MockZenohSession`. Maps to
//! `TEST_0303`: plugin A calls `querier.send(q)`; plugin B's
//! `queryable.try_recv` surfaces `(QueryId, Q)`; plugin B replies
//! three times then terminates; plugin A observes three replies
//! followed by `EndOfStream`.

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

const N: usize = 512;

#[test]
fn query_round_trip_three_replies_then_terminate() {
    // Both querier and queryable share one mock session.
    let session = Arc::new(MockZenohSession::new());

    // Connector A — hosts the queryable.
    let opts_a = ZenohConnectorOptions::builder()
        .tokio_worker_threads(1)
        .dispatcher_tick(Duration::from_millis(1))
        .query_timeout(Duration::from_secs(5))
        .build();
    let state_a = Arc::new(ZenohState::new(opts_a));
    let mut conn_a = ZenohConnector::new(state_a, Arc::clone(&session), JsonCodec).unwrap();

    // Connector B — hosts the querier.
    let opts_b = ZenohConnectorOptions::builder()
        .tokio_worker_threads(1)
        .dispatcher_tick(Duration::from_millis(1))
        .query_timeout(Duration::from_secs(5))
        .build();
    let state_b = Arc::new(ZenohState::new(opts_b));
    let mut conn_b = ZenohConnector::new(state_b, Arc::clone(&session), JsonCodec).unwrap();

    let routing = ZenohRouting::new(KeyExprOwned::try_from("robot/q").unwrap());
    let desc_a =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.q".to_string(), routing.clone()).unwrap();
    let desc_b =
        ChannelDescriptor::<ZenohRouting, N>::new("robot.q".to_string(), routing).unwrap();

    let mut qable = conn_a
        .create_queryable::<u32, String, N>(&desc_a)
        .expect("queryable");
    let mut querier = conn_b
        .create_querier::<u32, String, N>(&desc_b)
        .expect("querier");

    let mut exec_a = Executor::builder().worker_threads(0).build().unwrap();
    let mut exec_b = Executor::builder().worker_threads(0).build().unwrap();
    conn_a.register_with(&mut exec_a).unwrap();
    conn_b.register_with(&mut exec_b).unwrap();

    // Plugin B sends a query.
    let q_id = querier.send(&42_u32).expect("send query");

    // Plugin A drains queries with a generous deadline.
    let mut received_query = None;
    let deadline = std::time::Instant::now() + Duration::from_millis(500);
    while received_query.is_none() && std::time::Instant::now() < deadline {
        if let Ok(Some((id, q))) = qable.try_recv() {
            received_query = Some((id, q));
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let (id, q) = received_query.expect("queryable received query");
    assert_eq!(q, 42);

    // Plugin A replies three times and terminates.
    qable.reply(id, &"reply-1".to_string()).expect("reply 1");
    qable.reply(id, &"reply-2".to_string()).expect("reply 2");
    qable.reply(id, &"reply-3".to_string()).expect("reply 3");
    qable.terminate(id).expect("terminate");

    // Plugin B drains all reply events.
    let mut replies: Vec<String> = Vec::new();
    let mut saw_eos = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !saw_eos && std::time::Instant::now() < deadline {
        if let Ok(Some(event)) = querier.try_recv() {
            match event {
                QuerierEvent::Reply { id: ev_id, value } => {
                    assert_eq!(ev_id, q_id);
                    replies.push(value);
                }
                QuerierEvent::EndOfStream { id: ev_id } => {
                    assert_eq!(ev_id, q_id);
                    saw_eos = true;
                }
                QuerierEvent::Timeout { .. } => panic!("unexpected timeout"),
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    assert_eq!(replies.len(), 3, "expected 3 replies, got {replies:?}");
    assert_eq!(replies[0], "reply-1");
    assert_eq!(replies[1], "reply-2");
    assert_eq!(replies[2], "reply-3");
    assert!(saw_eos, "expected EndOfStream after replies");
}
