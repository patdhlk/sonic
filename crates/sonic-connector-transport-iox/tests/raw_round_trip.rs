//! TEST_0126 — `RawChannelWriter` → `RawChannelReader` round-trip via a
//! real iceoryx2 service. Byte-only path used by the gateway dispatcher
//! (`REQ_0326`, `REQ_0327`); confirms `send_raw_bytes` /
//! `try_recv_into` move bytes verbatim with no codec involvement and
//! that the header fields propagate end-to-end.

#![allow(clippy::doc_markdown)]

mod common;

use common::{make_node, unique_channel_name};
use sonic_connector_transport_iox::ServiceFactory;

#[test]
fn raw_writer_to_raw_reader_round_trip() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let name = unique_channel_name("raw_round_trip");

    let reader = factory
        .create_raw_reader_named::<256>(&name)
        .expect("create raw reader");
    let writer = factory
        .create_raw_writer_named::<256>(&name)
        .expect("create raw writer");

    let payload: [u8; 7] = [0xDE, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56];
    let correlation = [0u8; 32];
    let outcome = writer
        .send_raw_bytes(&payload, correlation)
        .expect("send raw");
    assert_eq!(outcome.sequence_number, 0);
    assert_eq!(outcome.bytes_written, payload.len());

    let mut dest = [0u8; 256];
    let sample = reader
        .try_recv_into(&mut dest)
        .expect("try_recv_into")
        .expect("envelope available");
    assert_eq!(sample.sequence_number, 0);
    assert_eq!(sample.payload_len, payload.len());
    assert_eq!(&dest[..sample.payload_len], &payload);
}

#[test]
fn raw_send_with_correlation_id_propagates() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let name = unique_channel_name("raw_correlation");

    let reader = factory.create_raw_reader_named::<64>(&name).unwrap();
    let writer = factory.create_raw_writer_named::<64>(&name).unwrap();

    let mut correlation = [0u8; 32];
    correlation[0] = 0xC0;
    correlation[31] = 0xDE;
    writer
        .send_raw_bytes(&[0xAA, 0xBB], correlation)
        .expect("send");
    let mut dest = [0u8; 64];
    let sample = reader.try_recv_into(&mut dest).unwrap().unwrap();
    assert_eq!(sample.correlation_id, correlation);
}

#[test]
fn raw_send_overflow_is_rejected() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let name = unique_channel_name("raw_overflow");
    let writer = factory.create_raw_writer_named::<8>(&name).unwrap();
    let too_big = [0u8; 16];
    let err = writer
        .send_raw_bytes(&too_big, [0u8; 32])
        .expect_err("must overflow");
    let msg = format!("{err}");
    assert!(msg.contains("payload") || msg.contains("overflow"), "{msg}");
}

#[test]
fn raw_writer_then_typed_reader_sees_raw_payload() {
    // Mixed setup: the raw writer publishes a buffer, a typed reader
    // attached to the same service decodes it via its codec. Confirms
    // that the raw path produces wire-compatible envelopes with the
    // typed path — load-bearing for the dispatcher pairing model where
    // a gateway-side RawChannelWriter publishes inbound bytes and the
    // plugin-side ChannelReader decodes them.
    use common::{Msg, TestJsonCodec};
    use sonic_connector_core::PayloadCodec;

    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let name = unique_channel_name("raw_to_typed");

    // Reader first (subscribers attach before publishers send).
    let reader_desc = sonic_connector_core::ChannelDescriptor::<common::TestRouting, 256>::new(
        name.clone(),
        common::TestRouting,
    )
    .unwrap();
    let reader = factory
        .create_reader::<Msg, _, _, 256>(&reader_desc, TestJsonCodec)
        .unwrap();
    let writer = factory.create_raw_writer_named::<256>(&name).unwrap();

    // Build the same on-wire bytes the typed writer would produce by
    // calling the codec ourselves.
    let original = Msg {
        value: 7,
        note: "raw→typed".into(),
    };
    let mut scratch = [0u8; 256];
    let written = TestJsonCodec
        .encode(&original, &mut scratch)
        .expect("encode");
    writer
        .send_raw_bytes(&scratch[..written], [0u8; 32])
        .expect("send raw");

    let received = reader
        .try_recv()
        .expect("try_recv")
        .expect("envelope available");
    assert_eq!(received.value, original);
}
