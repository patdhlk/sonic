//! TEST_0123 — the 32-byte correlation id round-trips verbatim. The
//! framework never inspects the bytes; it carries them sender→receiver
//! unchanged (`REQ_0204`).

#![allow(clippy::doc_markdown)]

mod common;

use common::{Msg, TestJsonCodec, descriptor, make_node};
use sonic_connector_transport_iox::ServiceFactory;

#[test]
fn correlation_id_round_trips_verbatim() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let desc = descriptor::<1024>("correlation");

    let reader = factory
        .create_reader::<Msg, _, _, 1024>(&desc, TestJsonCodec)
        .unwrap();
    let writer = factory
        .create_writer::<Msg, _, _, 1024>(&desc, TestJsonCodec)
        .unwrap();

    // Pattern that's distinguishable from accidental defaults and from
    // sequential incrementation — a sentinel + counting tail.
    let mut id = [0u8; 32];
    id[0] = 0xAB;
    id[1] = 0xCD;
    id[31] = 0xEF;
    for (i, slot) in id.iter_mut().enumerate().skip(2).take(29) {
        *slot = u8::try_from(i).unwrap();
    }

    writer
        .send_with_correlation(
            &Msg {
                value: 7,
                note: "with id".into(),
            },
            id,
        )
        .unwrap();

    let env = reader.try_recv().unwrap().expect("envelope present");
    assert_eq!(env.correlation_id, id);
}

/// Without `send_with_correlation`, the default `send` zeroes the id —
/// receivers see all-zeros. The framework's "passive carrier"
/// contract (`REQ_0204`) holds for the zero case too.
#[test]
fn correlation_id_defaults_to_zero() {
    let node = make_node();
    let factory = ServiceFactory::new(&node);
    let desc = descriptor::<512>("correlation_default");

    let reader = factory
        .create_reader::<Msg, _, _, 512>(&desc, TestJsonCodec)
        .unwrap();
    let writer = factory
        .create_writer::<Msg, _, _, 512>(&desc, TestJsonCodec)
        .unwrap();

    writer
        .send(&Msg {
            value: 1,
            note: "default id".into(),
        })
        .unwrap();
    let env = reader.try_recv().unwrap().expect("envelope present");
    assert_eq!(env.correlation_id, [0u8; 32]);
}
