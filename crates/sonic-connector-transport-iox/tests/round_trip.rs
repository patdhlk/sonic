//! TEST_0120 — `ChannelWriter` → `ChannelReader` round-trip via a real
//! iceoryx2 service. Verifies REQ_0205 (zero-copy publish) end-to-end
//! by sending one envelope and observing it on the reader side with the
//! payload bytes intact.

#![allow(clippy::doc_markdown)]

mod common;

use common::{Msg, TestJsonCodec, descriptor, make_node};
use sonic_connector_transport_iox::ServiceFactory;

#[test]
fn writer_to_reader_round_trip() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let desc = descriptor::<4096>("round_trip");

    // Reader created first so the subscriber is attached before the
    // publisher's first send — iceoryx2 publish/subscribe is not
    // retroactive by default.
    let reader = factory
        .create_reader::<Msg, _, _, 4096>(&desc, TestJsonCodec)
        .expect("create reader");
    let writer = factory
        .create_writer::<Msg, _, _, 4096>(&desc, TestJsonCodec)
        .expect("create writer");

    let original = Msg {
        value: 42,
        note: "round trip".into(),
    };
    let outcome = writer.send(&original).expect("send");
    assert_eq!(outcome.sequence_number, 0);
    assert!(outcome.bytes_written > 0);

    let received = reader
        .try_recv()
        .expect("try_recv")
        .expect("an envelope is available");

    assert_eq!(received.value, original);
    assert_eq!(received.sequence_number, 0);
}
