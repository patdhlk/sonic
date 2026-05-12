//! Tests for `MockZenohSession` — the in-process [`ZenohSessionLike`]
//! used by Layer-1 unit tests (`REQ_0445`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use sonic_connector_zenoh::{
    DoneCallback, KeyExprOwned, MockZenohSession, PayloadSink, SessionState, ZenohRouting,
    ZenohSessionLike,
};

fn routing(key: &str) -> ZenohRouting {
    ZenohRouting::new(KeyExprOwned::try_from(key).unwrap())
}

fn collect_sink() -> (Arc<Mutex<Vec<Vec<u8>>>>, PayloadSink) {
    let received: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let received_clone = received.clone();
    let sink: PayloadSink = Box::new(move |bytes: &[u8]| {
        received_clone.lock().unwrap().push(bytes.to_vec());
    });
    (received, sink)
}

#[test]
fn mock_session_starts_alive() {
    let session = MockZenohSession::new();
    assert_eq!(session.state(), SessionState::Alive);
}

#[test]
fn publish_with_no_subscriber_does_not_error() {
    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");
    session.publish(&r, b"hello").expect("publish ok");
}

#[test]
fn pub_sub_loopback_single_subscriber() {
    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");

    let (received, sink) = collect_sink();
    let _sub = session.subscribe(&r, sink).expect("subscribed");
    session.publish(&r, b"hello").expect("publish ok");
    session.publish(&r, b"world").expect("publish ok");

    let got = received.lock().unwrap().clone();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], b"hello");
    assert_eq!(got[1], b"world");
}

#[test]
fn pub_sub_loopback_fans_out_to_multiple_subscribers() {
    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");

    let (received_a, sink_a) = collect_sink();
    let (received_b, sink_b) = collect_sink();
    let _sub_a = session.subscribe(&r, sink_a).expect("sub_a");
    let _sub_b = session.subscribe(&r, sink_b).expect("sub_b");

    session.publish(&r, b"broadcast").expect("publish ok");

    assert_eq!(received_a.lock().unwrap().len(), 1);
    assert_eq!(received_b.lock().unwrap().len(), 1);
}

#[test]
fn pub_sub_loopback_filters_by_key_expr() {
    let session = MockZenohSession::new();
    let arm = routing("robot/arm");
    let leg = routing("robot/leg");

    let (received, sink) = collect_sink();
    let _sub = session.subscribe(&arm, sink).expect("subscribed");

    session.publish(&arm, b"arm-payload").unwrap();
    session.publish(&leg, b"leg-payload").unwrap();

    let got = received.lock().unwrap().clone();
    assert_eq!(got.len(), 1, "only arm payload should arrive");
    assert_eq!(got[0], b"arm-payload");
}

#[test]
fn dropping_subscription_handle_stops_delivery() {
    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");

    let (received, sink) = collect_sink();
    let sub = session.subscribe(&r, sink).expect("subscribed");

    session.publish(&r, b"first").unwrap();
    drop(sub);
    session.publish(&r, b"second").unwrap();

    let got = received.lock().unwrap().clone();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], b"first");
}

#[test]
fn programmable_session_state_steps() {
    let session = MockZenohSession::new();
    assert_eq!(session.state(), SessionState::Alive);

    session.set_state(SessionState::Closed {
        reason: "test close".into(),
    });
    assert!(matches!(session.state(), SessionState::Closed { .. }));

    session.set_state(SessionState::Connecting);
    assert_eq!(session.state(), SessionState::Connecting);
}

#[test]
fn publish_returns_not_alive_when_closed() {
    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");
    session.set_state(SessionState::Closed {
        reason: "test".into(),
    });

    let err = session
        .publish(&r, b"ignored")
        .expect_err("closed rejects publish");
    let msg = err.to_string();
    assert!(msg.contains("not alive"));
}

#[test]
fn query_returns_not_implemented_in_z1() {
    // Z3 lands query support. Z1 stubs return `NotImplemented` so tests
    // that assert "no query in Z1" can pin the failure mode.
    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");
    let on_reply: PayloadSink = Box::new(|_| {});
    let on_done: DoneCallback = Box::new(|| {});
    let err = session
        .query(&r, b"", Duration::from_millis(100), on_reply, on_done)
        .expect_err("Z1 query is stub");
    assert!(err.to_string().contains("not yet implemented"));
}
