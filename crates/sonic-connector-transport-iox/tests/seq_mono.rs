//! TEST_0121 — sequence-number monotonicity. Sending N envelopes
//! through one `ChannelWriter` yields strictly increasing sequence
//! numbers starting at zero on the corresponding `ChannelReader`.

#![allow(clippy::doc_markdown)]

mod common;

const N: u64 = 32;

use common::{Msg, TestJsonCodec, descriptor, make_node};
use sonic_connector_transport_iox::ServiceFactory;

#[test]
fn sequence_numbers_strictly_increase_from_zero() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let desc = descriptor::<1024>("seq_mono");

    let reader = factory
        .create_reader::<Msg, _, _, 1024>(&desc, TestJsonCodec)
        .expect("create reader");
    let writer = factory
        .create_writer::<Msg, _, _, 1024>(&desc, TestJsonCodec)
        .expect("create writer");

    // Iceoryx2's default subscriber buffer is small (a handful of
    // samples). Sending N in a burst would silently drop most of them
    // — the publisher's loan path doesn't block on full queues by
    // default. Interleave send / recv so at most one envelope is
    // in-flight at any time; the strict-monotonicity property is the
    // same.
    let mut last: i64 = -1;
    for i in 0..N {
        writer
            .send(&Msg {
                value: u32::try_from(i).unwrap(),
                note: format!("msg-{i}"),
            })
            .expect("send");
        let mut received = false;
        for _ in 0..1024 {
            if let Some(env) = reader.try_recv().expect("try_recv") {
                let seq_i64 = i64::try_from(env.sequence_number).unwrap();
                assert!(
                    seq_i64 > last,
                    "sequence not strictly increasing: prev={last}, now={seq_i64}"
                );
                last = seq_i64;
                received = true;
                break;
            }
            std::thread::yield_now();
        }
        assert!(received, "envelope #{i} not delivered within retry budget");
    }
    assert_eq!(last, i64::try_from(N - 1).unwrap());
}
