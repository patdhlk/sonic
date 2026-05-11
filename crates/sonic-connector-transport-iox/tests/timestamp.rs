//! TEST_0122 — `timestamp_ns` is populated at send. Wall-clock before
//! and after the send brackets the envelope's timestamp.

#![allow(clippy::doc_markdown)]

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use common::{Msg, TestJsonCodec, descriptor, make_node};
use sonic_connector_transport_iox::ServiceFactory;

fn now_ns() -> u64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    u64::try_from(d.as_nanos()).unwrap()
}

#[test]
fn timestamp_falls_within_pre_post_bracket() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let desc = descriptor::<512>("timestamp");

    let reader = factory
        .create_reader::<Msg, _, _, 512>(&desc, TestJsonCodec)
        .unwrap();
    let writer = factory
        .create_writer::<Msg, _, _, 512>(&desc, TestJsonCodec)
        .unwrap();

    let before = now_ns();
    writer
        .send(&Msg {
            value: 1,
            note: "ts".into(),
        })
        .unwrap();
    let after = now_ns();

    let env = reader.try_recv().unwrap().expect("envelope present");
    assert!(
        env.timestamp_ns >= before,
        "timestamp_ns ({}) precedes pre-send ({before})",
        env.timestamp_ns
    );
    assert!(
        env.timestamp_ns <= after,
        "timestamp_ns ({}) exceeds post-send ({after})",
        env.timestamp_ns
    );
}
