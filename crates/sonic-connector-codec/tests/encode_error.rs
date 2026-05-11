//! TEST_0111 — encode error on undersized buffer. `JsonCodec::encode`
//! into a buffer smaller than the encoded form returns
//! [`ConnectorError::PayloadOverflow`] carrying the actual and max
//! sizes; no partial write reaches the wire.
//!
//! Deviation from the spec's TEST_0111 narrative (which uses
//! `ConnectorError::Codec` for buffer-too-small): we route
//! buffer-exhaustion to `PayloadOverflow` so the variant is
//! distinguishable from genuine serializer faults (mismatched schema,
//! recursion limit, etc.). This keeps the codec layer consistent with
//! `ChannelWriter::send`'s contract (TEST_0125, REQ_0323) — both
//! surface buffer overflow as `PayloadOverflow`, never as `Codec`.

#![allow(clippy::doc_markdown)]

use serde::Serialize;
use sonic_connector_codec::{JsonCodec, PayloadCodec};
use sonic_connector_core::ConnectorError;

#[derive(Serialize)]
struct Msg {
    note: String,
}

#[test]
fn undersized_buffer_returns_payload_overflow() {
    let codec = JsonCodec::new();
    // Encoded form is approximately `{"note":"xxxxxxxxxx..."}` — well
    // over 16 bytes for a 1000-char note.
    let original = Msg {
        note: "x".repeat(1000),
    };
    let mut tiny = [0u8; 16];

    let err = codec
        .encode(&original, &mut tiny)
        .expect_err("expected overflow");
    match err {
        ConnectorError::PayloadOverflow { actual, max } => {
            assert!(actual > max, "actual={actual} must exceed max={max}");
            assert_eq!(max, 16, "max must equal the buffer size");
        }
        other => panic!("expected PayloadOverflow, got {other:?}"),
    }
}

/// Length-zero buffer is the smallest possible overflow case — JSON
/// requires at least `{}` (2 bytes) for the simplest object, so any
/// non-trivial value overflows a zero-length buffer.
#[test]
fn zero_length_buffer_returns_payload_overflow() {
    let codec = JsonCodec::new();
    let original = Msg { note: "hi".into() };
    let mut empty = [];
    let err = codec
        .encode(&original, &mut empty)
        .expect_err("expected overflow");
    assert!(matches!(
        err,
        ConnectorError::PayloadOverflow { max: 0, .. }
    ));
}

/// Genuine codec failures (not buffer overflow) surface as
/// [`ConnectorError::Codec`]. JSON requires object keys to be strings;
/// a `BTreeMap` with tuple keys hits `serde_json`'s "key must be a
/// string" path — a `Category::Data` error, not IO.
#[test]
fn non_string_map_key_returns_codec_not_overflow() {
    use std::collections::BTreeMap;
    let codec = JsonCodec::new();
    let mut bad: BTreeMap<(i32, i32), i32> = BTreeMap::new();
    bad.insert((1, 2), 3);
    let mut buf = vec![0u8; 4096]; // huge — overflow is not the failure mode
    let err = codec
        .encode(&bad, &mut buf)
        .expect_err("non-string map keys must fail");
    match err {
        ConnectorError::Codec { format, source: _ } => {
            assert_eq!(format, "json", "format must name the codec");
        }
        other => panic!("expected Codec, got {other:?}"),
    }
}
