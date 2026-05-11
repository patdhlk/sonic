//! TEST_0103 — `ChannelDescriptor` validation. Verifies REQ_0201 + REQ_0221.

#![allow(clippy::doc_markdown)]

use sonic_connector_core::{ChannelDescriptor, ConnectorError, Routing};

#[derive(Clone, Debug)]
struct TestRouting {
    tag: u32,
}

impl Routing for TestRouting {}

#[test]
fn ok_descriptor_round_trips_name_and_routing() {
    let d: ChannelDescriptor<TestRouting, 1024> =
        ChannelDescriptor::new("brake.commands", TestRouting { tag: 42 }).unwrap();
    assert_eq!(d.name(), "brake.commands");
    assert_eq!(d.routing().tag, 42);
    assert_eq!(d.max_payload_size(), 1024);
}

#[test]
fn empty_name_fails_with_invalid_descriptor() {
    let err = ChannelDescriptor::<TestRouting, 64>::new("", TestRouting { tag: 0 })
        .expect_err("empty name must fail validation");
    match err {
        ConnectorError::InvalidDescriptor(msg) => {
            assert!(msg.contains("empty"), "unexpected message: {msg}");
        }
        other => panic!("unexpected error variant: {other:?}"),
    }
}

#[test]
fn const_generic_n_propagates_per_channel() {
    let small: ChannelDescriptor<TestRouting, 4_096> =
        ChannelDescriptor::new("small", TestRouting { tag: 1 }).unwrap();
    let big: ChannelDescriptor<TestRouting, 1_048_576> =
        ChannelDescriptor::new("big", TestRouting { tag: 2 }).unwrap();
    assert_eq!(small.max_payload_size(), 4_096);
    assert_eq!(big.max_payload_size(), 1_048_576);
}
