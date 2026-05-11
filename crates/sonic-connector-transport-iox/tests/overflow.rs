//! TEST_0125 — `ChannelWriter::send` returns
//! `ConnectorError::PayloadOverflow` when the encoded payload exceeds
//! the channel's compile-time maximum, and no envelope is published.

#![allow(clippy::doc_markdown)]

mod common;

use common::{Msg, TestJsonCodec, descriptor, make_node};
use sonic_connector_core::ConnectorError;
use sonic_connector_transport_iox::ServiceFactory;

#[test]
fn overflow_returns_payload_overflow_and_skips_publish() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    // Tiny channel — 64-byte payload buffer. A `Msg { note: 200 chars }`
    // will JSON-encode to ~220 bytes, well over the cap.
    let desc = descriptor::<64>("overflow");

    let reader = factory
        .create_reader::<Msg, _, _, 64>(&desc, TestJsonCodec)
        .unwrap();
    let writer = factory
        .create_writer::<Msg, _, _, 64>(&desc, TestJsonCodec)
        .unwrap();

    let huge = Msg {
        value: 0,
        note: "x".repeat(200),
    };
    let err = writer.send(&huge).expect_err("expected overflow");
    match err {
        ConnectorError::PayloadOverflow { actual, max } => {
            assert!(actual > max, "actual={actual} max={max}");
            assert_eq!(max, 64);
        }
        other => panic!("unexpected error: {other:?}"),
    }

    // No envelope should be on the wire — the reader must see nothing.
    assert!(reader.try_recv().unwrap().is_none());

    // A subsequent small send must succeed normally — the failed send
    // must not have advanced the sequence counter or left a leaked loan.
    let small = Msg {
        value: 1,
        note: "ok".into(),
    };
    let outcome = writer.send(&small).expect("small send ok");
    assert_eq!(
        outcome.sequence_number, 0,
        "sequence number must not advance on overflow"
    );
    let env = reader.try_recv().unwrap().expect("small envelope present");
    assert_eq!(env.value, small);
}
