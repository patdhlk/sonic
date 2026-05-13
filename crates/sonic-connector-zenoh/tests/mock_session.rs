//! Tests for `MockZenohSession` — the in-process [`ZenohSessionLike`]
//! used by Layer-1 unit tests (`REQ_0445`).

use std::sync::{Arc, Mutex};

use sonic_connector_zenoh::{
    KeyExprOwned, MockZenohSession, PayloadSink, SessionState, ZenohRouting, ZenohSessionLike,
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
fn query_round_trip_to_single_queryable() {
    use std::sync::{Arc, Mutex};

    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");

    // Declare a queryable that replies with "hello" + the request bytes.
    let _qable = session
        .declare_queryable(
            &r,
            Box::new(|req: &[u8], replier: sonic_connector_zenoh::session::QueryReplier| {
                let mut out = b"hello,".to_vec();
                out.extend_from_slice(req);
                replier.reply(&out);
                replier.terminate();
            }),
        )
        .expect("queryable declared");

    let replies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let replies_clone = replies.clone();
    let done: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let done_clone = done.clone();

    session
        .query(
            &r,
            b" world",
            std::time::Duration::from_secs(1),
            Box::new(move |bytes: &[u8]| {
                replies_clone.lock().unwrap().push(bytes.to_vec());
            }),
            Box::new(move || {
                *done_clone.lock().unwrap() = true;
            }),
        )
        .expect("query dispatched");

    let got = replies.lock().unwrap().clone();
    assert_eq!(got, vec![b"hello, world".to_vec()]);
    assert!(*done.lock().unwrap(), "on_done should have fired");
}

#[test]
fn query_with_no_queryable_calls_done_immediately() {
    let session = MockZenohSession::new();
    let r = routing("robot/no/queryable");
    let done = std::sync::Arc::new(std::sync::Mutex::new(false));
    let done_clone = done.clone();

    session
        .query(
            &r,
            b"unused",
            std::time::Duration::from_secs(1),
            Box::new(|_: &[u8]| panic!("no replies expected")),
            Box::new(move || {
                *done_clone.lock().unwrap() = true;
            }),
        )
        .expect("query dispatched");

    assert!(*done.lock().unwrap(), "on_done should fire even with no queryable");
}

#[test]
fn query_fans_out_to_multiple_queryables() {
    use std::sync::{Arc, Mutex};

    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");

    let _q1 = session
        .declare_queryable(
            &r,
            Box::new(|_, replier| {
                replier.reply(b"q1-reply");
                replier.terminate();
            }),
        )
        .unwrap();
    let _q2 = session
        .declare_queryable(
            &r,
            Box::new(|_, replier| {
                replier.reply(b"q2-reply");
                replier.terminate();
            }),
        )
        .unwrap();

    let replies: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
    let replies_clone = replies.clone();
    session
        .query(
            &r,
            b"",
            std::time::Duration::from_secs(1),
            Box::new(move |bytes| replies_clone.lock().unwrap().push(bytes.to_vec())),
            Box::new(|| {}),
        )
        .unwrap();

    let got = replies.lock().unwrap().clone();
    assert_eq!(got.len(), 2);
    assert!(got.iter().any(|r| r == b"q1-reply"));
    assert!(got.iter().any(|r| r == b"q2-reply"));
}

#[test]
fn query_fails_when_session_closed() {
    let session = MockZenohSession::new();
    let r = routing("robot/test");
    session.set_state(SessionState::Closed {
        reason: "test".into(),
    });
    let err = session
        .query(
            &r,
            b"",
            std::time::Duration::from_secs(1),
            Box::new(|_| {}),
            Box::new(|| {}),
        )
        .expect_err("closed session rejects query");
    let msg = err.to_string();
    assert!(msg.contains("not alive"));
}

#[test]
fn dropping_queryable_handle_stops_receiving_queries() {
    use std::sync::{Arc, Mutex};

    let session = MockZenohSession::new();
    let r = routing("robot/arm/joint1");

    let fired = Arc::new(Mutex::new(0u32));
    let fired_clone = fired.clone();
    let qable = session
        .declare_queryable(
            &r,
            Box::new(move |_, replier| {
                *fired_clone.lock().unwrap() += 1;
                replier.reply(b"x");
                replier.terminate();
            }),
        )
        .unwrap();

    // First query: queryable fires.
    session
        .query(&r, b"", std::time::Duration::from_secs(1), Box::new(|_| {}), Box::new(|| {}))
        .unwrap();
    assert_eq!(*fired.lock().unwrap(), 1);

    drop(qable);

    // Second query after drop: queryable should NOT fire.
    session
        .query(&r, b"", std::time::Duration::from_secs(1), Box::new(|_| {}), Box::new(|| {}))
        .unwrap();
    assert_eq!(*fired.lock().unwrap(), 1, "queryable should not fire after drop");
}
