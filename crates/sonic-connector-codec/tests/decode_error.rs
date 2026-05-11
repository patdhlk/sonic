//! TEST_0112 — decode error propagation. `JsonCodec::decode` returns
//! [`ConnectorError::Codec`] (carrying the codec's
//! [`PayloadCodec::format_name`] and underlying serializer error)
//! rather than silently dropping the envelope (`REQ_0214`).

#![cfg(feature = "json")]
#![allow(clippy::doc_markdown)]

use serde::Deserialize;
use sonic_connector_codec::{JsonCodec, PayloadCodec};
use sonic_connector_core::ConnectorError;

#[derive(Debug, Deserialize)]
struct Msg {
    #[allow(dead_code)]
    value: i64,
}

#[test]
fn truncated_input_returns_codec_error() {
    let codec = JsonCodec::new();
    let truncated = b"{\"value\": 4";
    let err = codec
        .decode::<Msg>(truncated)
        .expect_err("expected decode error");
    match err {
        ConnectorError::Codec { format, source: _ } => {
            assert_eq!(format, "json");
        }
        other => panic!("expected Codec, got {other:?}"),
    }
}

#[test]
fn wrong_shape_returns_codec_error() {
    let codec = JsonCodec::new();
    // Missing required `value` field.
    let wrong = b"{\"other\": 1}";
    let err = codec
        .decode::<Msg>(wrong)
        .expect_err("expected decode error");
    assert!(matches!(err, ConnectorError::Codec { format: "json", .. }));
}

#[test]
fn wrong_type_returns_codec_error() {
    let codec = JsonCodec::new();
    // `value` is a string, not the expected integer.
    let wrong = b"{\"value\": \"forty-two\"}";
    let err = codec
        .decode::<Msg>(wrong)
        .expect_err("expected decode error");
    assert!(matches!(err, ConnectorError::Codec { format: "json", .. }));
}

#[test]
fn empty_input_returns_codec_error() {
    let codec = JsonCodec::new();
    let err = codec.decode::<Msg>(b"").expect_err("expected decode error");
    assert!(matches!(err, ConnectorError::Codec { format: "json", .. }));
}
